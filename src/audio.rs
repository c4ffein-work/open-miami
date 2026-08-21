//! Procedural audio engine for Open Miami // Rogue Purge.
//!
//! Everything here is synthesized at runtime with the Web Audio API (via
//! `web-sys`): oscillators for tones, a white-noise buffer for hits and
//! whooshes, all shaped by gain envelopes for a punchy, glitchy synthwave feel.
//! No audio files, no extra dependencies.
//!
//! The music runs through a dedicated bus — every note flows into a shared
//! lowpass [`web_sys::BiquadFilterNode`] whose cutoff is swept once per bar for
//! that classic synthwave/darksynth filter motion.
//!
//! One-shot SFX go through their own bus, built to make synthesized weapon
//! audio read as *recorded* weapon audio: every sound is a per-event voice
//! (optionally soft-clipped by a [`web_sys::WaveShaperNode`]) that feeds a
//! dry path and a send into a shared [`web_sys::ConvolverNode`] loaded with
//! a synthesized stereo room impulse response. Guns and hits follow measured
//! profiles of field recordings (an uncompressed bright crack, a plateau
//! body, the room; hits are a spectral-modeling resynthesis of a reference
//! recording); melee, misc. and hurt sounds are layered click / crack / body
//! designs through a [`web_sys::DynamicsCompressorNode`]. Everything gets
//! per-play pitch / timing jitter (see the SFX section of [`AudioEngine`]).
//!
//! Because building those per-event node graphs live can stall the main
//! thread (measured 30–100 ms on macOS Chrome), each one-shot kind is
//! pre-rendered at startup: the same voice builders run into an
//! [`web_sys::OfflineAudioContext`] and the resulting dry buffers replace
//! live graph construction with a single `AudioBufferSourceNode` per play —
//! see the "pre-rendered voices" section of [`AudioEngine`]. Until (or
//! unless) a kind's buffers are ready, its `play_*` uses the live path.
//!
//! The MUSIC notes get the same treatment (the tracker's oscillator+gain
//! construction per note was the last measured stall source, 70–113 ms):
//! a song's pitch set is finite pattern data, so every distinct voice ×
//! pitch it can schedule is baked at its exact frequency into a short mono
//! buffer by the identical note builders (see [`AudioEngine::music_keys`] /
//! `BakedMusic`), and `schedule_step` then fires one buffer source per note
//! into the same live music bus — the per-bar lowpass sweep is untouched.
//! The bake queue is prioritized: combat SFX first, then the current
//! song's voices, then the rare SFX; a song switch re-enumerates and bakes
//! in the background while unbaked notes fall back to live synthesis.
//!
//! Robustness first: if the `AudioContext` (or any node) fails to build we
//! silently degrade to silence. Nothing in here ever panics or unwraps a
//! fallible Web Audio call — every `Result` is swallowed so the game runs fine
//! even when audio is unavailable or blocked by the browser.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    AudioBuffer, AudioContext, AudioDestinationNode, BaseAudioContext, BiquadFilterNode,
    BiquadFilterType, GainNode, OfflineAudioContext, OscillatorType, OverSampleType,
};

/// Look-ahead window (seconds) for the music scheduler: we queue notes this far
/// in advance of the audio clock so playback never gaps between frames.
const LOOKAHEAD: f64 = 0.15;

/// Master music level — kept low so the looping backing never buries the SFX.
const MUSIC_GAIN: f64 = 0.07;

/// Output trim after the SFX compressor / soft-clip.
const SFX_GAIN: f64 = 0.9;

/// Level of the reverb return into the SFX compressor.
const REVERB_RETURN: f64 = 0.7;

/// Length of the synthesized room impulse response for the melee / misc.
/// bus (seconds).
const IR_SECONDS: f64 = 1.1;

/// Length of the gun / hit bus impulse response (seconds): RT ~1.5 s.
const IR_REAL_SECONDS: f64 = 1.7;

/// Overall gain of a resynthesised (SMS) metal hit: the model's loudest
/// track (a 1.0 sine partial) lands at this peak; the sub noise band, whose
/// RMS is normalised to its curve, peaks ~3× higher and is what the voice's
/// gentle soft-clip rounds off.
const SMS_HIT_GAIN: f32 = 0.45;

/// Target RMS, per unit of curve value, of an SMS noise band (1.0 = the
/// band's RMS equals its table amplitude — the same scale as the sine
/// partials' peak amplitude).
const SMS_NOISE_TRIM: f64 = 1.0;

/// Empirical per-band trims applied after the analytic RMS normalisation
/// of the SMS noise bands (measured against the METAL02 reference: the
/// 2nd-order bandpass skirts of the low bands leak more than the fc/Q
/// estimate assumes, the high bands land quiet). `(upper edge Hz, factor)`
/// — a band with centre `fc` uses the first row whose edge exceeds `fc`.
const SMS_BAND_TRIMS: &[(f64, f64)] = &[
    (100.0, 2.0),
    (200.0, 0.5),
    (400.0, 0.6),
    (1600.0, 0.7),
    (6000.0, 1.6),
    (12000.0, 1.3),
    (f64::MAX, 1.3),
];

/// Extra gain on SMS sine partials above this frequency (the 6–11 kHz ring
/// cluster measured ~2 dB quiet).
const SMS_HIGH_PARTIAL_HZ: f64 = 5000.0;
const SMS_HIGH_PARTIAL_GAIN: f32 = 1.25;

/// Level of the gun / hit bus reverb return: with a wet send of 1.0 the room
/// tail lands ≈ −21 dB @200 ms … −50 dB @1500 ms under the crack peak.
const REVERB_REAL_RETURN: f64 = 2.5;

/// Length of the shared white-noise buffer (seconds); bursts read it from a
/// random offset so no two share a waveform.
const NOISE_SECONDS: f64 = 2.0;

/// SFX are scheduled this far ahead of the audio clock so their sub-ms
/// transients are never dropped for being "in the past".
const SFX_LEAD: f64 = 0.012;

/// Sentinel used inside a pattern to mean "rest" (no note this step).
const REST: i32 = i32::MIN;

/// Number of sequenced channels (rows in the tracker view).
pub const NUM_CHANNELS: usize = 5;

/// Human-readable channel names, indexed 0..[`NUM_CHANNELS`].
pub const CHANNEL_NAMES: [&str; NUM_CHANNELS] = ["BASS", "LEAD", "PAD", "ARP", "DRUMS"];

// --- data-driven song format ----------------------------------------------
//
// A song is *plain, `const`-able data*: a key (root frequency + scale), a tempo,
// a set of oscillator voices, and — the new part — an ordered list of SECTIONS.
//
// Each [`Section`] is its own multi-bar block of five step-sequenced channels
// (bass, lead, pad, arp, drums). A [`SongSpec`] strings sections together into a
// real arrangement — intro / verse / refrain / bridge / variation — so a full
// play-through develops over time and the refrain *returns* instead of a single
// bar looping forever. Sections are just `&'static` slices of patterns, so a
// section can appear several times in the order (that is how a refrain comes
// back) at zero extra cost.
//
// Melodic patterns are written as *scale degrees* (see `degree_freq`): `0` is
// the root, `1` the next scale note up, `7` an octave up (for a 7-note scale),
// negative degrees drop below the root. `REST` means silence for that step.
// This keeps a song readable and in-key no matter which root/scale it uses.
//
// Lanes inside a section may differ in length: a short 16-step bass simply
// repeats under a longer 32-step lead. A section's length is its longest lane,
// so authoring a 2-bar section only means writing one lane at 32 steps.
//
// The `pad` lane is special: each note blooms into a full triad (root + third +
// fifth taken from the scale) with a slow attack, for sustained chord beds.

/// Scale = semitone offsets from the root, one octave's worth. Darker modes
/// (flat 2nd, tritone) read as more menacing — we escalate them across floors.
type Scale = &'static [i32];

/// Aeolian / natural minor — the classic neon-noir minor key.
const MINOR: Scale = &[0, 2, 3, 5, 7, 8, 10];
/// Dorian — minor with a raised 6th; cool, driving, a touch hopeful.
const DORIAN: Scale = &[0, 2, 3, 5, 7, 9, 10];
/// Harmonic minor — minor with a raised 7th; a sharp, gothic bite.
const HARMONIC_MINOR: Scale = &[0, 2, 3, 5, 7, 8, 11];
/// Phrygian — natural minor with a flat 2nd; tense and claustrophobic.
const PHRYGIAN: Scale = &[0, 1, 3, 5, 7, 8, 10];
/// Phrygian dominant — flat 2nd + major 3rd; exotic, aggressive, menacing.
const PHRYGIAN_DOMINANT: Scale = &[0, 1, 4, 5, 7, 8, 10];
/// Locrian — flat 2nd *and* a diminished 5th (tritone); maximally unstable.
const LOCRIAN: Scale = &[0, 1, 3, 5, 6, 8, 10];

/// One step of the drum lane. Rendered from synthesized noise/tones only.
#[derive(Clone, Copy)]
pub enum Drum {
    /// No percussion this step.
    Silent,
    /// Pitched sine thump + a lick of low noise.
    Kick,
    /// Very short high-passed noise tick.
    Hat,
    /// Noise burst + a short body tone on the backbeat.
    Snare,
}
use Drum::{Hat, Kick, Silent, Snare};

/// One block of an arrangement: a self-contained, multi-bar pattern across all
/// five channels. Songs are built by ordering these (a refrain section can be
/// listed several times so the hook comes back). A section's playable length is
/// the length of its longest lane; shorter lanes loop within it.
#[derive(Clone, Copy)]
pub struct Section {
    /// Human-readable role (intro / verse / refrain / bridge / outro). Purely
    /// documentation + exposed via the tracker API; the scheduler ignores it.
    pub label: &'static str,
    /// Bass lane, one scale-degree (or `REST`) per step.
    pub bass: &'static [i32],
    /// Lead/melody lane, one scale-degree (or `REST`) per step.
    pub lead: &'static [i32],
    /// Pad/chord lane: each note blooms into a slow triad. `REST` sustains.
    pub pad: &'static [i32],
    /// Arp lane — a faster, higher counter-melody.
    pub arp: &'static [i32],
    /// Percussion lane, one `Drum` per step.
    pub drums: &'static [Drum],
}

/// A whole song as copyable data. Author one, drop it in `SONGS`, done.
///
/// The key/tempo/voices live here; the *notes* live in the ordered `sections`.
#[derive(Clone, Copy)]
pub struct SongSpec {
    /// Human-readable name (shown in the `?viz` "Musics" tracker).
    pub name: &'static str,
    /// Root/tonic frequency in Hz (e.g. `55.0` = A1). Lower == darker/deeper.
    pub root: f64,
    /// The key/mode: semitone offsets from `root`.
    pub scale: Scale,
    /// Tempo in beats per minute.
    pub bpm: f64,
    /// Sequencer resolution: steps per beat (`4` = sixteenth notes).
    pub steps_per_beat: u32,
    /// Oscillator shape for the bass voice.
    pub bass_wave: OscillatorType,
    /// Oscillator shape for the lead voice.
    pub lead_wave: OscillatorType,
    /// Oscillator shape for the pad voice.
    pub pad_wave: OscillatorType,
    /// Oscillator shape for the arp voice.
    pub arp_wave: OscillatorType,
    /// The arrangement: an ordered list of sections played back to back, then
    /// looped as a whole. This is what makes a song long and developing.
    pub sections: &'static [Section],
    /// Overall punch/loudness feel (~0.5 lounge .. ~1.2 boss).
    pub intensity: f64,
}

// ---------------------------------------------------------------------------
// SONG 1 — "Insert Coin" (WAVY): ominous, dreamy title theme. A-minor, slow,
// soft triangle/sine voices, lush pad, sparse falling arp. The calm before it.
// ---------------------------------------------------------------------------

const INSERT_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, 14, REST, REST, REST, REST, REST, 12, REST, REST, REST,
        REST, REST, REST, REST, REST, REST, 11, REST, REST, REST, REST, REST, 9, REST, REST, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, REST, REST, 7, REST, 9, REST, REST, REST, REST, REST, 11, REST, 9, REST, REST,
        REST, REST, REST, 9, REST, 11, REST, REST, REST, REST, REST, 12, REST, 9, REST,
    ],
    drums: &[
        Silent, Silent, Silent, Silent, Hat, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Hat, Silent, Silent, Silent,
    ],
};
const INSERT_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, 14, REST, REST, REST, 12, REST, REST, REST, 11, REST, REST, REST, REST, REST,
        REST, REST, 12, REST, REST, REST, 10, REST, REST, REST, 9, REST, REST, REST, 7, REST,
    ],
    pad: &[
        7, REST, REST, REST, REST, REST, REST, REST, 10, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, REST, REST, 7, REST, 9, REST, REST, REST, REST, REST, 11, REST, 9, REST, REST,
        REST, REST, REST, 9, REST, 11, REST, REST, REST, REST, REST, 12, REST, 9, REST,
    ],
    drums: &[
        Silent, Silent, Silent, Silent, Hat, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Hat, Silent, Silent, Silent,
    ],
};
const INSERT_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[
        0, REST, REST, REST, 0, REST, 3, REST, 5, REST, REST, REST, 3, REST, 2, REST,
    ],
    lead: &[
        7, REST, 9, REST, 11, REST, 12, REST, REST, 14, REST, 12, 11, REST, 9, REST, 7, REST, 9,
        REST, 11, REST, 14, REST, REST, 16, REST, 14, 12, REST, 11, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, 16, 18, 16, 14, 16, 18, 21, 14, 16, 18, 16, 18, 16, 14, 11, 14, 16, 18, 21, 18, 16, 14,
        16, 18, 21, 23, 21, 18, 16, 14, 12,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Snare,
        Silent, Hat, Silent,
    ],
};
const INSERT_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, REST, REST, 11, REST, REST, REST, 9, REST, 7, REST,
        REST, REST, REST, REST, REST, REST, REST, REST, 12, REST, REST, REST, 10, REST, 9, REST,
    ],
    pad: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        7, REST, 9, REST, 11, REST, 9, REST, 7, REST, 9, REST, 11, REST, 14, REST, 11, REST, 9,
        REST, 7, REST, 9, REST, 11, REST, 9, REST, 7, REST, 4, REST,
    ],
    drums: &[
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Hat, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent,
    ],
};

const INSERT_COIN: SongSpec = SongSpec {
    name: "Insert Coin",
    root: 55.0, // A1
    scale: MINOR,
    bpm: 84.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Triangle,
    lead_wave: OscillatorType::Sine,
    pad_wave: OscillatorType::Triangle,
    arp_wave: OscillatorType::Sine,
    sections: &[
        INSERT_INTRO,
        INSERT_VERSE,
        INSERT_VERSE,
        INSERT_REFRAIN,
        INSERT_VERSE,
        INSERT_BRIDGE,
        INSERT_REFRAIN,
        INSERT_REFRAIN,
    ],
    intensity: 0.5,
};

// ---------------------------------------------------------------------------
// SONG 2 — "Neon Lounge" (WAVY): cool, loungey opening groove. A-minor,
// laid-back, mellow syncopated lead over light hats — neon at dusk.
// ---------------------------------------------------------------------------

const NEON_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, 7, REST, 9, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST, REST, REST, 11, REST, 9, REST, REST, REST, REST, REST, REST, REST, REST, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, 14, REST, 16, REST, 14, REST, 11, REST, 14, REST, 16, REST, 18, REST, 16, REST, 14,
        REST, 16, REST, 14, REST, 11, REST, 14, REST, 16, REST, 18, REST, 16,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Silent,
        Silent, Hat, Silent,
    ],
};
const NEON_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, REST, REST, REST, 0, REST, 4, REST, 3, REST, REST, REST, 2, REST, 2, REST,
    ],
    lead: &[
        7, REST, 9, REST, REST, 11, REST, 7, REST, REST, 9, REST, 10, REST, REST, REST, 7, REST, 9,
        REST, REST, 11, REST, 12, REST, REST, 10, REST, 9, REST, 7, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, 14, REST, 16, REST, 14, REST, 11, REST, 14, REST, 16, REST, 18, REST, 16, REST, 16,
        REST, 18, REST, 16, REST, 14, REST, 16, REST, 18, REST, 21, REST, 18,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Silent,
        Silent, Hat, Snare,
    ],
};
const NEON_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[
        0, REST, 0, REST, 4, REST, 4, REST, 3, REST, 3, REST, 2, REST, 5, REST,
    ],
    lead: &[
        11, REST, 12, REST, 14, REST, 12, REST, 11, REST, 9, REST, 7, REST, 9, REST, 11, REST, 12,
        REST, 14, REST, 16, REST, 14, REST, 12, REST, 11, REST, 9, REST,
    ],
    pad: &[
        3, REST, REST, REST, REST, REST, REST, REST, 5, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        18, 16, 14, 16, 18, 16, 14, 11, 18, 16, 14, 16, 18, 21, 18, 16, 14, 16, 18, 21, 18, 16, 14,
        16, 18, 21, 23, 21, 18, 16, 14, 12,
    ],
    drums: &[
        Kick, Silent, Hat, Snare, Silent, Silent, Hat, Silent, Kick, Silent, Hat, Snare, Silent,
        Silent, Hat, Snare,
    ],
};
const NEON_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, 9, REST, 7, REST, REST, REST, REST, REST, 11, REST, 9, REST, REST, REST, REST,
        REST, 9, REST, 7, REST, REST, REST, REST, REST, 12, REST, 10, REST, 9, REST,
    ],
    pad: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, REST, 16, REST, 18, REST, 16, REST, 14, REST, 16, REST, 18, REST, 21, REST, 18, REST,
        16, REST, 14, REST, 16, REST, 14, REST, 11, REST, 9, REST, 7, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Silent, Silent, Hat, Silent, Kick, Silent, Silent, Silent,
        Silent, Silent, Hat, Silent,
    ],
};

const NEON_LOUNGE: SongSpec = SongSpec {
    name: "Neon Lounge",
    root: 55.0, // A1
    scale: MINOR,
    bpm: 108.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Triangle,
    lead_wave: OscillatorType::Triangle,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Triangle,
    sections: &[
        NEON_INTRO,
        NEON_VERSE,
        NEON_VERSE,
        NEON_REFRAIN,
        NEON_VERSE,
        NEON_BRIDGE,
        NEON_REFRAIN,
        NEON_REFRAIN,
    ],
    intensity: 0.55,
};

// ---------------------------------------------------------------------------
// SONG 3 — "Chrome Veins" (AGGRESSIVE): chromed, forward-leaning drive. B
// Dorian, pulsing square bass, bright square arp, warm saw pad. City blur.
// ---------------------------------------------------------------------------

const CHROME_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, 7, REST, REST, REST, 0, REST, REST, REST, 5, REST, 3, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, 7, REST, REST, REST, REST, REST, REST, REST, 9, REST,
        REST, REST, REST, REST, REST, REST, 11, REST, REST, REST, REST, REST, REST, REST, 7, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, 16, 18, 16, 14, 16, 18, 21, 14, 16, 18, 16, 18, 16, 14, 11, 14, 16, 18, 16, 14, 16, 18,
        21, 14, 16, 18, 16, 18, 16, 14, 11,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Silent,
        Silent, Hat, Silent,
    ],
};
const CHROME_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, REST, 0, REST, 7, REST, 0, REST, 0, REST, 0, REST, 5, REST, 3, REST,
    ],
    lead: &[
        REST, REST, 7, REST, 9, REST, 11, REST, REST, 12, REST, 11, 9, REST, 7, REST, REST, REST,
        7, REST, 9, REST, 12, REST, REST, 14, REST, 12, 11, REST, 9, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, 16, 18, 16, 14, 16, 18, 21, 14, 16, 18, 16, 18, 16, 14, 11, 14, 16, 18, 21, 18, 16, 14,
        16, 18, 21, 23, 21, 18, 16, 14, 11,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Snare, Silent, Hat, Silent, Kick, Silent, Hat, Kick, Snare,
        Silent, Hat, Silent,
    ],
};
const CHROME_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[
        0, REST, 0, 7, 0, REST, 0, 7, 5, REST, 5, REST, 3, REST, 3, REST,
    ],
    lead: &[
        12, REST, 11, REST, 9, REST, 7, REST, 9, REST, 11, REST, 12, REST, 14, REST, 16, REST, 14,
        REST, 12, REST, 11, REST, 9, REST, 11, REST, 12, REST, 14, REST,
    ],
    pad: &[
        3, REST, REST, REST, REST, REST, REST, REST, 7, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        18, 16, 14, 16, 18, 21, 18, 16, 14, 16, 18, 21, 23, 21, 18, 16, 14, 16, 18, 21, 23, 21, 18,
        16, 18, 21, 23, 26, 23, 21, 18, 16,
    ],
    drums: &[
        Kick, Hat, Hat, Silent, Snare, Silent, Hat, Kick, Kick, Hat, Hat, Kick, Snare, Silent, Hat,
        Snare,
    ],
};
const CHROME_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        5, REST, REST, REST, 5, REST, REST, REST, 4, REST, REST, REST, 4, REST, REST, REST,
    ],
    lead: &[
        REST, REST, 12, REST, 11, REST, 9, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST, 14, REST, 12, REST, 11, REST, REST, REST, REST, REST, REST, REST, REST, REST,
    ],
    pad: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, REST, 16, REST, 18, REST, 16, REST, 14, REST, 16, REST, 18, REST, 21, REST, 18, REST,
        16, REST, 14, REST, 16, REST, 18, REST, 14, REST, 11, REST, 9, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Snare, Silent, Hat, Silent,
    ],
};

const CHROME_VEINS: SongSpec = SongSpec {
    name: "Chrome Veins",
    root: 61.74, // B1
    scale: DORIAN,
    bpm: 118.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Square,
    lead_wave: OscillatorType::Sawtooth,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Square,
    sections: &[
        CHROME_INTRO,
        CHROME_VERSE,
        CHROME_VERSE,
        CHROME_REFRAIN,
        CHROME_VERSE,
        CHROME_BRIDGE,
        CHROME_REFRAIN,
        CHROME_REFRAIN,
    ],
    intensity: 0.72,
};

// ---------------------------------------------------------------------------
// SONG 4 — "Descent" (AGGRESSIVE): tense mid-descent. D Phrygian (flat 2nd),
// driving square bass hammering the root, restless saw arp, four-on-the-floor.
// ---------------------------------------------------------------------------

const DESCENT_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, 0, REST, REST, REST, 0, REST, REST, REST, 0, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, REST, REST, 7, REST, 8, REST, 10, REST, 8, REST, REST,
        REST, REST, REST, REST, REST, REST, REST, 7, REST, 10, REST, 8, REST, 7, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 5, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, REST, 15, REST, 17, REST, 15, REST, 14, REST, 17, REST, 19, REST, 17, REST, 14, REST,
        15, REST, 17, REST, 15, REST, 14, REST, 17, REST, 19, REST, 17, REST,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Snare, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Snare,
        Silent, Hat, Silent,
    ],
};
const DESCENT_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, REST, 0, REST, 0, REST, 0, REST, 0, REST, 0, REST, 5, REST, 4, REST,
    ],
    lead: &[
        7, 8, 10, 8, 7, 10, 8, 10, 12, 11, 10, 8, 7, 8, 7, REST, 7, 8, 10, 8, 10, 11, 12, 10, 8,
        10, 12, 11, 10, 8, 7, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 5, REST, REST, REST, 4, REST, REST, REST,
    ],
    arp: &[
        14, REST, 15, REST, 17, REST, 15, REST, 14, REST, 17, REST, 19, REST, 17, REST, 14, REST,
        17, REST, 19, REST, 17, REST, 15, REST, 17, REST, 15, REST, 14, REST,
    ],
    drums: &[
        Kick, Hat, Hat, Hat, Snare, Hat, Hat, Hat, Kick, Hat, Kick, Hat, Snare, Hat, Hat, Hat,
    ],
};
const DESCENT_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[
        0, 0, REST, 0, 0, 0, REST, 0, 0, 0, REST, 0, 5, REST, 4, REST,
    ],
    lead: &[
        12, REST, 11, REST, 10, REST, 8, REST, 7, REST, 8, REST, 10, REST, 12, REST, 14, REST, 12,
        REST, 11, REST, 10, REST, 8, REST, 10, REST, 12, REST, 14, REST,
    ],
    pad: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        17, REST, 19, REST, 21, REST, 19, REST, 17, REST, 19, REST, 22, REST, 19, REST, 21, REST,
        22, REST, 24, REST, 22, REST, 19, REST, 17, REST, 15, REST, 14, REST,
    ],
    drums: &[
        Kick, Hat, Snare, Hat, Kick, Hat, Snare, Hat, Kick, Kick, Snare, Hat, Kick, Snare, Snare,
        Hat,
    ],
};
const DESCENT_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        5, REST, REST, REST, 5, REST, REST, REST, 4, REST, REST, REST, 4, REST, REST, REST,
    ],
    lead: &[
        REST, REST, 10, REST, 8, REST, 7, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST, 12, REST, 10, REST, 8, REST, REST, REST, REST, REST, REST, REST, REST, REST,
    ],
    pad: &[
        5, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        14, REST, 15, REST, 17, REST, 15, REST, 14, REST, 15, REST, 17, REST, 19, REST, 17, REST,
        15, REST, 14, REST, 12, REST, 10, REST, 8, REST, 7, REST, 5, REST,
    ],
    drums: &[
        Kick, Silent, Hat, Silent, Snare, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Snare,
        Silent, Hat, Hat,
    ],
};

const DESCENT: SongSpec = SongSpec {
    name: "Descent",
    root: 36.71, // D1
    scale: PHRYGIAN,
    bpm: 132.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Square,
    lead_wave: OscillatorType::Sawtooth,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Square,
    sections: &[
        DESCENT_INTRO,
        DESCENT_VERSE,
        DESCENT_VERSE,
        DESCENT_REFRAIN,
        DESCENT_VERSE,
        DESCENT_BRIDGE,
        DESCENT_REFRAIN,
        DESCENT_REFRAIN,
    ],
    intensity: 0.85,
};

// ---------------------------------------------------------------------------
// SONG 5 — "Blood Rush" (AGGRESSIVE): feverish, blood-in-the-eyes rush. F#
// harmonic minor, jagged saw bass, wailing square lead over a stabbing arp.
// ---------------------------------------------------------------------------

const BLOOD_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, 0, REST, REST, REST, 0, REST, REST, REST, 4, REST, 6, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, 11, REST, REST, REST, REST, REST, REST, REST, 12, REST,
        REST, REST, REST, REST, REST, REST, 14, REST, REST, REST, REST, REST, REST, REST, 11, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        7, 9, 11, 9, 7, 9, 11, 14, 7, 9, 11, 9, 11, 9, 7, 4, 7, 9, 11, 9, 7, 9, 11, 14, 7, 9, 11,
        9, 11, 9, 7, 4,
    ],
    drums: &[
        Kick, Hat, Snare, Hat, Kick, Silent, Snare, Hat, Kick, Hat, Snare, Hat, Kick, Silent,
        Snare, Hat,
    ],
};
const BLOOD_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, 0, REST, 0, 6, REST, 0, 0, 0, 0, REST, 0, 4, REST, 6, REST,
    ],
    lead: &[
        11, REST, 12, 11, 9, REST, 11, REST, 12, REST, 14, 12, 11, 9, 11, REST, 12, REST, 14, 12,
        11, REST, 12, REST, 14, REST, 16, 14, 12, 11, 9, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, 6, REST, REST, REST,
    ],
    arp: &[
        7, 9, 11, 9, 7, 9, 11, 14, 7, 9, 11, 9, 11, 9, 7, 4, 9, 11, 14, 11, 9, 11, 14, 16, 9, 11,
        14, 11, 14, 11, 9, 7,
    ],
    drums: &[
        Kick, Hat, Snare, Hat, Kick, Kick, Snare, Hat, Kick, Hat, Snare, Hat, Kick, Snare, Snare,
        Hat,
    ],
};
const BLOOD_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[0, 0, 0, 0, 6, 6, 0, 0, 0, 0, 0, 0, 4, 4, 6, 6],
    lead: &[
        14, REST, 16, 14, 12, REST, 14, REST, 16, REST, 18, 16, 14, 12, 11, REST, 16, REST, 18, 16,
        14, REST, 16, REST, 18, REST, 19, 18, 16, 14, 12, REST,
    ],
    pad: &[
        4, REST, REST, REST, REST, REST, REST, REST, 6, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        11, 14, 16, 14, 11, 14, 16, 19, 11, 14, 16, 14, 16, 14, 11, 7, 14, 16, 19, 16, 14, 16, 19,
        21, 14, 16, 19, 16, 19, 16, 14, 11,
    ],
    drums: &[
        Kick, Kick, Snare, Hat, Kick, Kick, Snare, Kick, Kick, Kick, Snare, Hat, Kick, Snare,
        Snare, Snare,
    ],
};
const BLOOD_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        4, REST, REST, REST, 4, REST, REST, REST, 6, REST, REST, REST, 6, REST, REST, REST,
    ],
    lead: &[
        REST, REST, 12, 11, 9, REST, REST, REST, REST, REST, 11, 9, 7, REST, REST, REST, REST,
        REST, 14, 12, 11, REST, REST, REST, REST, REST, 12, 11, 9, REST, REST, REST,
    ],
    pad: &[
        4, REST, REST, REST, REST, REST, REST, REST, 6, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        7, 9, 11, 9, 7, 9, 11, 14, 7, 9, 11, 9, 11, 9, 7, 4, 11, 9, 7, 9, 11, 14, 11, 9, 7, 9, 11,
        9, 7, 4, 2, 0,
    ],
    drums: &[
        Kick, Silent, Snare, Silent, Kick, Silent, Snare, Silent, Kick, Hat, Snare, Hat, Kick, Hat,
        Snare, Hat,
    ],
};

const BLOOD_RUSH: SongSpec = SongSpec {
    name: "Blood Rush",
    root: 46.25, // F#1
    scale: HARMONIC_MINOR,
    bpm: 140.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Sawtooth,
    lead_wave: OscillatorType::Square,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Sawtooth,
    sections: &[
        BLOOD_INTRO,
        BLOOD_VERSE,
        BLOOD_VERSE,
        BLOOD_REFRAIN,
        BLOOD_VERSE,
        BLOOD_BRIDGE,
        BLOOD_REFRAIN,
        BLOOD_REFRAIN,
    ],
    intensity: 0.95,
};

// ---------------------------------------------------------------------------
// SONG 6 — "Deep Static" (AGGRESSIVE): menacing deep-floor pressure. E
// Phrygian-dominant, relentless saw sub-bass in 16ths, dissonant stabs.
// ---------------------------------------------------------------------------

const DEEP_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, 0, REST, REST, REST, 0, REST, REST, REST, 4, REST, 1, 0,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, 7, REST, REST, REST, REST, REST, REST, REST, 8, REST,
        REST, REST, REST, REST, REST, REST, 7, REST, REST, REST, REST, REST, REST, REST, 11, REST,
    ],
    pad: &[
        0, REST, REST, REST, 1, REST, REST, REST, 0, REST, REST, REST, 4, REST, REST, REST,
    ],
    arp: &[
        REST, 14, 15, REST, 14, REST, 18, REST, REST, 14, 15, REST, 18, REST, 15, 14, REST, 14, 15,
        REST, 14, REST, 18, REST, REST, 14, 15, REST, 18, REST, 15, 14,
    ],
    drums: &[
        Kick, Silent, Kick, Silent, Snare, Silent, Kick, Silent, Kick, Silent, Kick, Silent, Snare,
        Silent, Kick, Silent,
    ],
};
const DEEP_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, 0, 0, REST, 1, REST, 0, REST, 0, 0, 0, REST, 4, REST, 1, 0,
    ],
    lead: &[
        REST, REST, 7, REST, 8, REST, REST, 7, REST, 11, REST, REST, 8, REST, 7, REST, REST, REST,
        8, REST, 7, REST, REST, 8, REST, 11, REST, REST, 7, REST, 8, REST,
    ],
    pad: &[
        0, REST, REST, REST, 1, REST, REST, REST, 0, REST, REST, REST, 4, REST, REST, REST,
    ],
    arp: &[
        REST, 14, 15, REST, 14, REST, 18, REST, REST, 14, 15, REST, 18, REST, 15, 14, 14, REST, 15,
        REST, 18, REST, 15, REST, 14, REST, 18, REST, 21, REST, 18, 15,
    ],
    drums: &[
        Kick, Silent, Kick, Silent, Snare, Silent, Kick, Kick, Kick, Silent, Kick, Silent, Snare,
        Hat, Kick, Snare,
    ],
};
const DEEP_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 4, 4, 1, 0],
    lead: &[
        11, REST, 8, REST, 7, REST, 8, REST, 11, REST, 12, REST, 11, REST, 8, REST, 14, REST, 11,
        REST, 8, REST, 7, REST, 8, REST, 11, REST, 14, REST, 11, REST,
    ],
    pad: &[
        4, REST, REST, REST, 1, REST, REST, REST, 0, REST, REST, REST, 4, REST, REST, REST,
    ],
    arp: &[
        14, 15, 18, 15, 14, 15, 18, 21, 14, 15, 18, 15, 18, 15, 14, 11, 18, 15, 14, 15, 18, 21, 18,
        15, 14, 15, 18, 21, 22, 21, 18, 15,
    ],
    drums: &[
        Kick, Kick, Kick, Snare, Snare, Kick, Kick, Kick, Kick, Kick, Kick, Snare, Snare, Kick,
        Kick, Snare,
    ],
};
const DEEP_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        4, REST, REST, REST, 4, REST, REST, REST, 1, REST, REST, REST, 1, REST, REST, REST,
    ],
    lead: &[
        REST, REST, 8, REST, 7, REST, REST, REST, REST, REST, 11, REST, 8, REST, REST, REST, REST,
        REST, 7, REST, 8, REST, REST, REST, REST, REST, 11, REST, 12, REST, REST, REST,
    ],
    pad: &[
        4, REST, REST, REST, REST, REST, REST, REST, 1, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, 14, 15, REST, 14, REST, 18, REST, REST, 14, 15, REST, 18, REST, 15, 14, 18, REST, 15,
        REST, 14, REST, 11, REST, 8, REST, 7, REST, 4, REST, 1, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Snare, Silent, Kick, Silent, Kick, Silent, Silent, Silent,
        Snare, Silent, Kick, Kick,
    ],
};

const DEEP_STATIC: SongSpec = SongSpec {
    name: "Deep Static",
    root: 41.20, // E1
    scale: PHRYGIAN_DOMINANT,
    bpm: 144.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Sawtooth,
    lead_wave: OscillatorType::Sawtooth,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Square,
    sections: &[
        DEEP_INTRO,
        DEEP_VERSE,
        DEEP_VERSE,
        DEEP_REFRAIN,
        DEEP_VERSE,
        DEEP_BRIDGE,
        DEEP_REFRAIN,
        DEEP_REFRAIN,
    ],
    intensity: 1.0,
};

// ---------------------------------------------------------------------------
// SONG 7 — "Static Prayer" (WAVY): a crawling, hopeless dirge. G Locrian
// (tritone), slow lurching bass, mournful pad drone, sparse detuned wails.
// ---------------------------------------------------------------------------

const PRAYER_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, REST, REST, REST, REST, 0, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, REST, REST, REST, REST, 14, REST, REST, REST, REST, REST, REST, REST, 15, REST,
        REST, REST, REST, REST, REST, REST, 18, REST, REST, REST, REST, REST, REST, REST, 15, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Silent, Silent, Hat, Silent,
    ],
};
const PRAYER_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, REST, REST, REST, 0, REST, REST, 4, 0, REST, REST, REST, 1, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, 8, REST, REST, REST, REST, REST, 7, REST, REST, REST, REST, REST,
        REST, REST, REST, REST, 7, REST, REST, REST, REST, REST, 8, REST, REST, REST, REST, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, 14, REST, REST, REST, 15, REST, REST, REST, 18, REST, REST, REST, 15, REST,
        REST, REST, 15, REST, REST, REST, 18, REST, REST, REST, 14, REST, REST, REST, 11, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Silent, Silent, Snare, Silent, Kick, Silent, Silent, Silent,
        Snare, Silent, Hat, Silent,
    ],
};
const PRAYER_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[
        0, REST, REST, REST, 4, REST, REST, REST, 1, REST, REST, REST, 4, REST, REST, REST,
    ],
    lead: &[
        8, REST, REST, REST, 7, REST, REST, REST, 8, REST, REST, REST, 11, REST, REST, REST, 12,
        REST, REST, REST, 11, REST, REST, REST, 8, REST, REST, REST, 7, REST, REST, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, 1, REST, REST, REST,
    ],
    arp: &[
        REST, 14, REST, 15, REST, 18, REST, 15, REST, 14, REST, 15, REST, 18, REST, 21, REST, 18,
        REST, 15, REST, 14, REST, 11, REST, 14, REST, 15, REST, 18, REST, 15,
    ],
    drums: &[
        Kick, Silent, Silent, Snare, Silent, Silent, Snare, Silent, Kick, Silent, Silent, Snare,
        Silent, Silent, Hat, Snare,
    ],
};
const PRAYER_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        4, REST, REST, REST, REST, REST, REST, REST, 1, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, REST, REST, 11, REST, REST, REST, 8, REST, 7, REST,
        REST, REST, REST, REST, REST, REST, REST, REST, 12, REST, REST, REST, 11, REST, 8, REST,
    ],
    pad: &[
        4, REST, REST, REST, REST, REST, REST, REST, 1, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, 14, REST, REST, REST, 15, REST, REST, REST, 18, REST, REST, REST, 21, REST,
        REST, REST, 18, REST, REST, REST, 15, REST, REST, REST, 14, REST, REST, REST, 11, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Snare, Silent, Silent, Silent,
    ],
};

const STATIC_PRAYER: SongSpec = SongSpec {
    name: "Static Prayer",
    root: 49.00, // G1
    scale: LOCRIAN,
    bpm: 92.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Sawtooth,
    lead_wave: OscillatorType::Triangle,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Triangle,
    sections: &[
        PRAYER_INTRO,
        PRAYER_VERSE,
        PRAYER_VERSE,
        PRAYER_REFRAIN,
        PRAYER_VERSE,
        PRAYER_BRIDGE,
        PRAYER_REFRAIN,
        PRAYER_REFRAIN,
    ],
    intensity: 0.8,
};

// ---------------------------------------------------------------------------
// SONG 8 — "Mask of Dread" (AGGRESSIVE / heavy BOSS): dread-filled and huge. C
// Locrian (flat 2nd + tritone), slow but crushing; sustained saw bass lurching
// to the tritone, high square wails, enormous slow kicks. The mask watches.
// ---------------------------------------------------------------------------

const MASK_INTRO: Section = Section {
    label: "intro",
    bass: &[
        0, REST, REST, REST, REST, REST, REST, REST, 0, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        7, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, 8, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, 14, REST, 15, REST, REST, REST, REST, REST, 18, REST, 15, REST, 14, REST, REST,
        REST, 14, REST, 15, REST, REST, REST, REST, REST, 18, REST, 15, REST, 14, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Snare, Silent, Silent, Silent,
    ],
};
const MASK_VERSE: Section = Section {
    label: "verse",
    bass: &[
        0, REST, REST, REST, 0, REST, 4, REST, 0, REST, REST, REST, 4, REST, 3, REST,
    ],
    lead: &[
        7, REST, REST, REST, REST, REST, REST, REST, 8, REST, REST, REST, REST, REST, 11, REST,
        REST, REST, REST, REST, 7, REST, REST, REST, 8, REST, REST, REST, REST, REST, 4, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, 14, REST, 15, REST, REST, REST, REST, REST, 18, REST, 15, REST, 14, REST, REST,
        REST, 15, REST, 18, REST, REST, REST, REST, REST, 14, REST, 11, REST, 14, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Silent, Kick,
        Snare, Silent, Snare, Silent,
    ],
};
const MASK_REFRAIN: Section = Section {
    label: "refrain",
    bass: &[
        0, REST, 0, REST, 4, REST, 4, REST, 3, REST, 3, REST, 4, REST, 1, REST,
    ],
    lead: &[
        11, REST, REST, REST, 8, REST, REST, REST, 7, REST, REST, REST, 8, REST, 11, REST, 12,
        REST, REST, REST, 11, REST, REST, REST, 8, REST, REST, REST, 7, REST, 4, REST,
    ],
    pad: &[
        0, REST, REST, REST, REST, REST, REST, REST, 4, REST, REST, REST, 1, REST, REST, REST,
    ],
    arp: &[
        14, REST, 15, REST, 18, REST, 15, REST, 14, REST, 15, REST, 18, REST, 21, REST, 18, REST,
        15, REST, 14, REST, 11, REST, 14, REST, 15, REST, 18, REST, 15, REST,
    ],
    drums: &[
        Kick, Silent, Kick, Silent, Snare, Silent, Kick, Silent, Kick, Silent, Kick, Kick, Snare,
        Silent, Snare, Snare,
    ],
};
const MASK_BRIDGE: Section = Section {
    label: "bridge",
    bass: &[
        4, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    lead: &[
        REST, REST, REST, REST, 8, REST, REST, REST, REST, REST, REST, REST, 7, REST, REST, REST,
        REST, REST, REST, REST, 11, REST, REST, REST, REST, REST, REST, REST, 8, REST, REST, REST,
    ],
    pad: &[
        4, REST, REST, REST, REST, REST, REST, REST, 3, REST, REST, REST, REST, REST, REST, REST,
    ],
    arp: &[
        REST, REST, 14, REST, REST, REST, 15, REST, REST, REST, 18, REST, REST, REST, 21, REST,
        REST, REST, 18, REST, REST, REST, 15, REST, REST, REST, 14, REST, REST, REST, 11, REST,
    ],
    drums: &[
        Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Snare, Silent, Silent, Silent,
    ],
};

const MASK_OF_DREAD: SongSpec = SongSpec {
    name: "Mask of Dread",
    root: 32.70, // C1
    scale: LOCRIAN,
    bpm: 100.0,
    steps_per_beat: 4,
    bass_wave: OscillatorType::Sawtooth,
    lead_wave: OscillatorType::Square,
    pad_wave: OscillatorType::Sawtooth,
    arp_wave: OscillatorType::Square,
    sections: &[
        MASK_INTRO,
        MASK_VERSE,
        MASK_VERSE,
        MASK_REFRAIN,
        MASK_VERSE,
        MASK_BRIDGE,
        MASK_REFRAIN,
        MASK_REFRAIN,
    ],
    intensity: 1.15,
};

/// All songs, in ascending darkness (intro first). Index into this with
/// `play_song`, or map a floor number through `song_for_floor`.
pub const SONGS: &[SongSpec] = &[
    INSERT_COIN,
    NEON_LOUNGE,
    CHROME_VEINS,
    DESCENT,
    BLOOD_RUSH,
    DEEP_STATIC,
    STATIC_PRAYER,
    MASK_OF_DREAD,
];

/// Resolve a scale-degree (root = 0, +1 = next scale note up, +scale.len() = an
/// octave up, negatives drop below root) to a frequency in Hz, in-key.
fn degree_freq(root: f64, scale: Scale, degree: i32) -> f64 {
    if scale.is_empty() {
        return root;
    }
    let n = scale.len() as i32;
    let octave = degree.div_euclid(n);
    let idx = degree.rem_euclid(n) as usize;
    let semitones = octave * 12 + scale[idx];
    root * 2f64.powf(semitones as f64 / 12.0)
}

/// Read a melodic lane at `step` (patterns loop). `None` = rest / empty lane.
fn degree_at(pattern: &[i32], step: usize) -> Option<i32> {
    if pattern.is_empty() {
        return None;
    }
    match pattern[step % pattern.len()] {
        REST => None,
        d => Some(d),
    }
}

/// Read the drum lane at `step` (loops). Empty lane == `Silent`.
fn drum_at(pattern: &[Drum], step: usize) -> Drum {
    if pattern.is_empty() {
        return Silent;
    }
    pattern[step % pattern.len()]
}

/// The playable length of a section: its longest lane (shorter lanes loop
/// inside it). Always at least 1 so the scheduler can never divide by zero.
fn section_len(sec: &Section) -> usize {
    sec.bass
        .len()
        .max(sec.lead.len())
        .max(sec.pad.len())
        .max(sec.arp.len())
        .max(sec.drums.len())
        .max(1)
}

/// Pick a song for a given floor, escalating darkness as you descend. Kept as a
/// plain mapping so the integrator can call it per level.
pub fn song_for_floor(level: usize) -> SongSpec {
    match level {
        0..=1 => NEON_LOUNGE,
        2..=3 => CHROME_VEINS,
        4..=5 => DESCENT,
        6..=7 => BLOOD_RUSH,
        8..=9 => DEEP_STATIC,
        10..=12 => STATIC_PRAYER,
        _ => MASK_OF_DREAD,
    }
}

/// Recipe for one gunshot (see the attacks section comment): a bright
/// crack, a mid-dominant body with a plateau, a low-mid layer, a faint
/// thump — and the room (reverb send) for everything after ~200 ms.
struct RealShot {
    /// Crack level and its rise time (peak lands at +5–15 ms).
    crack: f64,
    crack_rise: f64,
    /// Air (>8 kHz) share of the crack.
    air: f64,
    /// Mid body: bandpass centre, level; holds `plateau` seconds at −5 dB
    /// then falls to −21 dB by `plateau + drop`, then on to silence.
    body_hz: f64,
    body: f64,
    plateau: f64,
    drop: f64,
    /// Hi (2–8 kHz) and air companions of the body plateau, levels.
    body_hi: f64,
    body_air: f64,
    /// Low-mid (130–300 Hz) layer: bandpass centre and level (real "body"
    /// of a big caliber; ~5 dB under the mids for the small ones).
    low_hz: f64,
    low: f64,
    /// Faint sub thump (60–90 Hz), level (≤ −15 dB).
    thump: f64,
    /// Reverb send (the room is most of the sound past 200 ms).
    wet: f64,
}

/// 7.62×39 single (our pistol): mid −3.5 dB dominant, low −8.6, hi −6.6,
/// air −7.7, sub −15.8; crest 20–23 dB; peak at +7.7 ms; centroid 6.2 kHz
/// @5 ms → ~1.3 kHz after; t20 131 ms.
const REAL_762X39: RealShot = RealShot {
    crack: 1.0,
    crack_rise: 0.005,
    air: 0.6,
    body_hz: 1200.0,
    body: 1.0,
    plateau: 0.10,
    drop: 0.10,
    body_hi: 0.26,
    body_air: 0.32,
    low_hz: 230.0,
    low: 1.4,
    thump: 0.03,
    wet: 1.0,
};

/// 5.56 single (our machinegun round): mid −2.7 dominant, low −8.1, hi
/// −8.5, air −8.1, sub −18.8; crest 20–24 dB; peak +6.5 ms; centroid
/// 5.8 kHz @5 ms → 1.2–1.3 kHz; t20 133 ms; roughness ~0.6.
const REAL_556: RealShot = RealShot {
    crack: 0.9,
    crack_rise: 0.004,
    air: 0.65,
    body_hz: 1300.0,
    body: 0.9,
    plateau: 0.09,
    drop: 0.10,
    body_hi: 0.21,
    body_air: 0.30,
    low_hz: 240.0,
    low: 1.1,
    thump: 0.02,
    wet: 1.0,
};

/// 7.62×54R single (our shotgun): the biggest — mid −2.4, a real low-mid
/// body (low −7.4), hi −8.9, air −11.2, sub −14.9; crest 19–22 dB; peak at
/// +14.8 ms; centroid 4.1 kHz @5 ms → 579 Hz @30 → ~1.3 kHz; t20 147 ms.
const REAL_762X54R: RealShot = RealShot {
    crack: 1.1,
    crack_rise: 0.010,
    air: 0.28,
    body_hz: 1000.0,
    body: 1.2,
    plateau: 0.11,
    drop: 0.10,
    body_hi: 0.12,
    body_air: 0.15,
    low_hz: 210.0,
    low: 2.0,
    thump: 0.05,
    wet: 1.0,
};

/// Analysis tables for spectral-modeling resynthesis (see
/// [`AudioEngine::sms_play`]). Generated offline; kept verbatim.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
mod sms_tables {
    // @generated by sms_extract.py from BulletImpactMetal02.wav: 16 partials + 9 noise bands,
    // 120 points at 5 ms (total 600 ms). Amplitudes are linear, relative to the loudest track.
    pub const METAL02_HOP: f32 = 0.005;
    pub const METAL02_PARTIALS: &[(f32, [f32; 120])] = &[
        (
            302.1,
            [
                0.290, 0.360, 0.372, 0.320, 0.223, 0.145, 0.150, 0.207, 0.298, 0.376, 0.398, 0.369,
                0.317, 0.283, 0.289, 0.323, 0.336, 0.297, 0.234, 0.207, 0.219, 0.229, 0.247, 0.219,
                0.176, 0.123, 0.080, 0.066, 0.064, 0.059, 0.050, 0.033, 0.023, 0.020, 0.023, 0.024,
                0.023, 0.020, 0.010, 0.006, 0.009, 0.008, 0.004, 0.003, 0.004, 0.005, 0.005, 0.005,
                0.005, 0.005, 0.004, 0.003, 0.003, 0.003, 0.003, 0.003, 0.003, 0.002, 0.002, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.002, 0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.001, 0.001, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.002, 0.004, 0.007, 0.008, 0.007, 0.004, 0.002, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            517.9,
            [
                0.126, 0.135, 0.116, 0.076, 0.050, 0.048, 0.058, 0.077, 0.100, 0.155, 0.174, 0.145,
                0.146, 0.154, 0.130, 0.194, 0.265, 0.312, 0.326, 0.298, 0.225, 0.142, 0.130, 0.106,
                0.144, 0.165, 0.170, 0.163, 0.140, 0.112, 0.081, 0.059, 0.048, 0.050, 0.053, 0.052,
                0.047, 0.038, 0.028, 0.021, 0.016, 0.012, 0.009, 0.006, 0.004, 0.003, 0.002, 0.002,
                0.003, 0.002, 0.003, 0.002, 0.002, 0.002, 0.001, 0.001, 0.002, 0.002, 0.003, 0.003,
                0.002, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.001, 0.002, 0.004, 0.005, 0.004, 0.002, 0.001, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            867.9,
            [
                0.097, 0.102, 0.087, 0.071, 0.062, 0.059, 0.108, 0.160, 0.189, 0.202, 0.240, 0.238,
                0.191, 0.194, 0.191, 0.184, 0.168, 0.147, 0.126, 0.122, 0.124, 0.117, 0.112, 0.117,
                0.110, 0.098, 0.079, 0.059, 0.042, 0.029, 0.024, 0.019, 0.014, 0.011, 0.014, 0.012,
                0.009, 0.010, 0.010, 0.010, 0.009, 0.008, 0.007, 0.006, 0.006, 0.006, 0.006, 0.005,
                0.004, 0.003, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.001, 0.001, 0.002, 0.003, 0.002, 0.001, 0.001, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            1526.8,
            [
                0.025, 0.040, 0.045, 0.048, 0.066, 0.094, 0.170, 0.218, 0.219, 0.208, 0.219, 0.236,
                0.253, 0.250, 0.211, 0.131, 0.068, 0.065, 0.082, 0.092, 0.101, 0.105, 0.096, 0.077,
                0.056, 0.047, 0.056, 0.071, 0.077, 0.077, 0.073, 0.069, 0.066, 0.063, 0.058, 0.051,
                0.046, 0.044, 0.043, 0.043, 0.041, 0.039, 0.037, 0.036, 0.034, 0.033, 0.031, 0.030,
                0.028, 0.027, 0.026, 0.025, 0.024, 0.024, 0.023, 0.021, 0.020, 0.019, 0.017, 0.016,
                0.015, 0.015, 0.014, 0.013, 0.012, 0.011, 0.010, 0.009, 0.009, 0.008, 0.007, 0.007,
                0.006, 0.005, 0.005, 0.004, 0.004, 0.003, 0.003, 0.002, 0.002, 0.002, 0.002, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            1641.4,
            [
                0.056, 0.068, 0.063, 0.040, 0.067, 0.157, 0.270, 0.340, 0.329, 0.273, 0.208, 0.172,
                0.201, 0.209, 0.163, 0.093, 0.077, 0.054, 0.053, 0.055, 0.052, 0.054, 0.045, 0.035,
                0.030, 0.022, 0.019, 0.019, 0.018, 0.012, 0.008, 0.005, 0.007, 0.007, 0.007, 0.007,
                0.009, 0.009, 0.009, 0.009, 0.008, 0.007, 0.006, 0.004, 0.002, 0.002, 0.001, 0.002,
                0.002, 0.002, 0.001, 0.001, 0.001, 0.002, 0.001, 0.001, 0.001, 0.001, 0.002, 0.002,
                0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            2194.4,
            [
                0.023, 0.026, 0.025, 0.024, 0.040, 0.091, 0.160, 0.215, 0.229, 0.192, 0.126, 0.118,
                0.125, 0.136, 0.120, 0.078, 0.038, 0.033, 0.044, 0.049, 0.042, 0.033, 0.029, 0.021,
                0.018, 0.022, 0.021, 0.016, 0.013, 0.013, 0.015, 0.016, 0.017, 0.018, 0.019, 0.015,
                0.011, 0.007, 0.008, 0.009, 0.009, 0.009, 0.010, 0.010, 0.008, 0.007, 0.007, 0.007,
                0.006, 0.005, 0.005, 0.005, 0.005, 0.004, 0.004, 0.004, 0.004, 0.003, 0.003, 0.003,
                0.003, 0.003, 0.002, 0.002, 0.002, 0.002, 0.001, 0.001, 0.002, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            2394.6,
            [
                0.035, 0.045, 0.046, 0.037, 0.031, 0.041, 0.066, 0.101, 0.144, 0.180, 0.210, 0.230,
                0.196, 0.137, 0.087, 0.061, 0.051, 0.037, 0.038, 0.040, 0.042, 0.033, 0.023, 0.017,
                0.015, 0.013, 0.012, 0.011, 0.009, 0.008, 0.011, 0.013, 0.012, 0.009, 0.006, 0.005,
                0.005, 0.007, 0.007, 0.006, 0.005, 0.003, 0.002, 0.003, 0.004, 0.005, 0.005, 0.004,
                0.003, 0.002, 0.001, 0.001, 0.002, 0.002, 0.003, 0.002, 0.002, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.002, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.001, 0.001, 0.001,
                0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            3051.7,
            [
                0.035, 0.040, 0.042, 0.039, 0.039, 0.057, 0.083, 0.158, 0.230, 0.271, 0.256, 0.196,
                0.118, 0.048, 0.034, 0.044, 0.054, 0.058, 0.048, 0.030, 0.024, 0.028, 0.033, 0.029,
                0.023, 0.019, 0.016, 0.011, 0.009, 0.007, 0.005, 0.004, 0.003, 0.002, 0.002, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            4452.1,
            [
                0.036, 0.050, 0.056, 0.055, 0.047, 0.038, 0.042, 0.064, 0.097, 0.146, 0.198, 0.224,
                0.208, 0.156, 0.108, 0.078, 0.048, 0.039, 0.028, 0.018, 0.021, 0.021, 0.019, 0.015,
                0.010, 0.012, 0.013, 0.008, 0.005, 0.003, 0.003, 0.002, 0.002, 0.002, 0.001, 0.002,
                0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.001, 0.000, 0.001, 0.001, 0.000, 0.000,
                0.001, 0.000, 0.000, 0.000, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            5061.4,
            [
                0.033, 0.035, 0.034, 0.026, 0.026, 0.046, 0.071, 0.087, 0.092, 0.097, 0.138, 0.194,
                0.215, 0.187, 0.159, 0.119, 0.087, 0.076, 0.069, 0.058, 0.047, 0.047, 0.054, 0.055,
                0.045, 0.029, 0.022, 0.014, 0.008, 0.009, 0.008, 0.006, 0.004, 0.004, 0.004, 0.004,
                0.004, 0.004, 0.003, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            5204.6,
            [
                0.044, 0.050, 0.049, 0.040, 0.038, 0.088, 0.156, 0.210, 0.231, 0.218, 0.177, 0.135,
                0.114, 0.098, 0.079, 0.064, 0.047, 0.036, 0.039, 0.048, 0.051, 0.048, 0.040, 0.030,
                0.024, 0.019, 0.014, 0.011, 0.009, 0.005, 0.003, 0.002, 0.002, 0.002, 0.002, 0.002,
                0.002, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            7972.5,
            [
                0.021, 0.028, 0.030, 0.029, 0.030, 0.073, 0.120, 0.193, 0.253, 0.280, 0.269, 0.231,
                0.176, 0.111, 0.079, 0.066, 0.069, 0.071, 0.085, 0.107, 0.124, 0.140, 0.150, 0.151,
                0.141, 0.124, 0.103, 0.083, 0.069, 0.062, 0.062, 0.062, 0.060, 0.054, 0.049, 0.045,
                0.040, 0.035, 0.030, 0.024, 0.019, 0.015, 0.012, 0.009, 0.008, 0.008, 0.007, 0.006,
                0.005, 0.004, 0.004, 0.003, 0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            8227.9,
            [
                0.016, 0.018, 0.016, 0.014, 0.012, 0.035, 0.082, 0.143, 0.195, 0.216, 0.198, 0.161,
                0.132, 0.111, 0.097, 0.090, 0.086, 0.079, 0.073, 0.070, 0.073, 0.084, 0.106, 0.131,
                0.151, 0.162, 0.162, 0.155, 0.143, 0.130, 0.116, 0.101, 0.086, 0.073, 0.064, 0.057,
                0.051, 0.045, 0.039, 0.034, 0.029, 0.024, 0.020, 0.017, 0.014, 0.012, 0.010, 0.009,
                0.008, 0.007, 0.006, 0.005, 0.004, 0.003, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            8414.4,
            [
                0.010, 0.010, 0.013, 0.015, 0.035, 0.106, 0.228, 0.382, 0.512, 0.589, 0.608, 0.587,
                0.562, 0.540, 0.521, 0.504, 0.484, 0.456, 0.418, 0.382, 0.355, 0.339, 0.330, 0.322,
                0.311, 0.296, 0.277, 0.258, 0.237, 0.216, 0.194, 0.174, 0.154, 0.136, 0.117, 0.099,
                0.081, 0.064, 0.050, 0.040, 0.032, 0.026, 0.022, 0.018, 0.015, 0.013, 0.010, 0.008,
                0.007, 0.005, 0.004, 0.003, 0.002, 0.002, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            9820.8,
            [
                0.014, 0.020, 0.023, 0.023, 0.045, 0.100, 0.170, 0.224, 0.232, 0.252, 0.265, 0.268,
                0.264, 0.253, 0.235, 0.218, 0.206, 0.202, 0.197, 0.188, 0.173, 0.153, 0.132, 0.113,
                0.099, 0.088, 0.079, 0.070, 0.061, 0.053, 0.046, 0.040, 0.035, 0.031, 0.027, 0.024,
                0.021, 0.018, 0.016, 0.014, 0.012, 0.010, 0.009, 0.008, 0.007, 0.006, 0.005, 0.005,
                0.004, 0.003, 0.003, 0.003, 0.002, 0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            10777.0,
            [
                0.019, 0.023, 0.022, 0.018, 0.017, 0.049, 0.094, 0.122, 0.155, 0.205, 0.233, 0.231,
                0.243, 0.208, 0.149, 0.090, 0.073, 0.062, 0.057, 0.053, 0.050, 0.044, 0.034, 0.026,
                0.021, 0.016, 0.013, 0.011, 0.008, 0.006, 0.005, 0.004, 0.002, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
    ];
    /// (center Hz, hi/lo ratio -> Q ~ sqrt(r)/(r-1), gain curve)
    pub const METAL02_NOISE: &[(f32, f32, [f32; 120])] = &[
        (
            54.8,
            3.333,
            [
                0.107, 0.116, 0.102, 0.233, 0.156, 0.125, 0.113, 0.119, 0.118, 0.046, 0.117, 0.291,
                0.301, 0.334, 0.392, 0.400, 0.890, 1.000, 0.597, 0.658, 0.463, 0.324, 0.544, 0.687,
                0.506, 0.667, 0.360, 0.456, 0.239, 0.181, 0.056, 0.193, 0.336, 0.180, 0.131, 0.161,
                0.090, 0.078, 0.100, 0.116, 0.085, 0.065, 0.067, 0.084, 0.063, 0.093, 0.093, 0.073,
                0.063, 0.073, 0.087, 0.095, 0.094, 0.096, 0.097, 0.098, 0.097, 0.096, 0.097, 0.098,
                0.097, 0.098, 0.097, 0.097, 0.097, 0.097, 0.097, 0.097, 0.095, 0.094, 0.093, 0.092,
                0.092, 0.092, 0.092, 0.091, 0.091, 0.090, 0.089, 0.089, 0.088, 0.088, 0.087, 0.087,
                0.086, 0.086, 0.085, 0.085, 0.085, 0.084, 0.084, 0.084, 0.083, 0.082, 0.082, 0.082,
                0.082, 0.082, 0.082, 0.082, 0.082, 0.082, 0.089, 0.080, 0.028, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            141.4,
            2.000,
            [
                0.027, 0.020, 0.077, 0.177, 0.161, 0.072, 0.041, 0.019, 0.025, 0.053, 0.145, 0.194,
                0.174, 0.279, 0.199, 0.242, 0.484, 0.462, 0.257, 0.310, 0.179, 0.212, 0.149, 0.197,
                0.386, 0.419, 0.108, 0.233, 0.197, 0.086, 0.092, 0.061, 0.144, 0.139, 0.088, 0.059,
                0.066, 0.031, 0.035, 0.058, 0.034, 0.011, 0.017, 0.021, 0.020, 0.013, 0.006, 0.005,
                0.003, 0.002, 0.002, 0.002, 0.000, 0.002, 0.001, 0.002, 0.001, 0.006, 0.008, 0.007,
                0.005, 0.002, 0.003, 0.003, 0.003, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.011, 0.022, 0.016, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            282.8,
            2.000,
            [
                0.013, 0.041, 0.066, 0.076, 0.113, 0.065, 0.050, 0.035, 0.028, 0.022, 0.071, 0.020,
                0.050, 0.039, 0.077, 0.110, 0.105, 0.012, 0.031, 0.093, 0.093, 0.016, 0.013, 0.043,
                0.051, 0.032, 0.027, 0.024, 0.020, 0.031, 0.022, 0.018, 0.021, 0.036, 0.028, 0.010,
                0.010, 0.014, 0.004, 0.003, 0.002, 0.003, 0.002, 0.000, 0.002, 0.002, 0.002, 0.002,
                0.002, 0.002, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.001, 0.001, 0.001, 0.001,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.002, 0.005, 0.003, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            565.7,
            2.000,
            [
                0.017, 0.026, 0.074, 0.130, 0.104, 0.060, 0.029, 0.016, 0.038, 0.079, 0.093, 0.097,
                0.076, 0.075, 0.118, 0.107, 0.088, 0.090, 0.109, 0.096, 0.107, 0.124, 0.106, 0.056,
                0.056, 0.054, 0.048, 0.040, 0.036, 0.048, 0.047, 0.034, 0.030, 0.032, 0.025, 0.020,
                0.015, 0.013, 0.008, 0.009, 0.006, 0.004, 0.005, 0.005, 0.004, 0.003, 0.002, 0.002,
                0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.002, 0.005, 0.003, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            1131.4,
            2.000,
            [
                0.007, 0.010, 0.025, 0.040, 0.039, 0.028, 0.015, 0.011, 0.048, 0.093, 0.123, 0.111,
                0.088, 0.075, 0.086, 0.099, 0.086, 0.067, 0.060, 0.067, 0.061, 0.050, 0.052, 0.047,
                0.050, 0.043, 0.031, 0.022, 0.013, 0.018, 0.020, 0.017, 0.017, 0.019, 0.018, 0.013,
                0.009, 0.007, 0.007, 0.006, 0.006, 0.007, 0.006, 0.005, 0.004, 0.004, 0.004, 0.004,
                0.004, 0.004, 0.003, 0.003, 0.003, 0.003, 0.003, 0.002, 0.002, 0.002, 0.002, 0.002,
                0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.001, 0.002, 0.001, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            2262.7,
            2.000,
            [
                0.004, 0.011, 0.025, 0.044, 0.044, 0.028, 0.014, 0.010, 0.066, 0.102, 0.080, 0.060,
                0.071, 0.091, 0.082, 0.079, 0.065, 0.059, 0.063, 0.050, 0.040, 0.047, 0.048, 0.031,
                0.024, 0.018, 0.018, 0.021, 0.015, 0.016, 0.016, 0.013, 0.014, 0.011, 0.010, 0.009,
                0.006, 0.005, 0.004, 0.003, 0.003, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.001, 0.001,
                0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            4525.5,
            2.000,
            [
                0.006, 0.013, 0.024, 0.034, 0.039, 0.028, 0.014, 0.009, 0.044, 0.106, 0.111, 0.097,
                0.074, 0.075, 0.080, 0.070, 0.057, 0.045, 0.040, 0.042, 0.039, 0.041, 0.035, 0.027,
                0.022, 0.020, 0.016, 0.013, 0.012, 0.010, 0.010, 0.009, 0.008, 0.007, 0.007, 0.007,
                0.007, 0.007, 0.006, 0.005, 0.005, 0.005, 0.005, 0.005, 0.005, 0.004, 0.004, 0.004,
                0.004, 0.004, 0.004, 0.004, 0.004, 0.004, 0.004, 0.003, 0.003, 0.003, 0.003, 0.003,
                0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.001, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            8763.6,
            1.875,
            [
                0.003, 0.005, 0.012, 0.023, 0.021, 0.014, 0.010, 0.007, 0.043, 0.089, 0.089, 0.080,
                0.069, 0.059, 0.050, 0.044, 0.037, 0.035, 0.032, 0.024, 0.021, 0.018, 0.015, 0.014,
                0.014, 0.013, 0.011, 0.010, 0.010, 0.008, 0.007, 0.007, 0.007, 0.006, 0.006, 0.006,
                0.005, 0.004, 0.004, 0.003, 0.002, 0.003, 0.003, 0.003, 0.003, 0.002, 0.002, 0.002,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
                0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
        (
            15491.9,
            1.667,
            [
                0.002, 0.003, 0.005, 0.008, 0.009, 0.008, 0.006, 0.003, 0.008, 0.020, 0.031, 0.034,
                0.034, 0.028, 0.019, 0.016, 0.013, 0.011, 0.010, 0.008, 0.007, 0.006, 0.004, 0.003,
                0.002, 0.002, 0.002, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
                0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
            ],
        ),
    ];
    /// impulsive transients (time s after onset, amplitude rel. loudest transient) — play as 0.3–1 ms clicks through a 4–12 kHz bandpass
    pub const METAL02_TRANSIENTS: &[(f32, f32)] = &[(0.0272, 0.251), (0.0559, 1.000)];
}

/// How many pre-rendered jitter variants each one-shot SFX kind gets (see
/// [`BakedSfx`]): a random one is picked per play, plus a small
/// `playback_rate` jitter, so repeated shots never sound stamped.
const SFX_VARIANTS: usize = 3;

/// Every pre-renderable one-shot SFX voice. The discriminant indexes
/// [`BakedSfx::bufs`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum SfxKind {
    AttackGun,
    AttackMachinegun,
    AttackShotgun,
    AttackClub,
    HitGun,
    HitMachinegun,
    HitShotgun,
    HitClub,
    EnemyDown,
    PlayerHurt,
    Pickup,
    Throw,
    Death,
    LevelClear,
    MaskCrack,
    Elevator,
}

/// All kinds, in pre-render order (the combat sounds first — they are the
/// expensive ones and the ones a firefight needs early).
const SFX_KINDS: [SfxKind; 16] = [
    SfxKind::AttackGun,
    SfxKind::AttackMachinegun,
    SfxKind::AttackShotgun,
    SfxKind::AttackClub,
    SfxKind::HitGun,
    SfxKind::HitMachinegun,
    SfxKind::HitShotgun,
    SfxKind::HitClub,
    SfxKind::EnemyDown,
    SfxKind::PlayerHurt,
    SfxKind::Pickup,
    SfxKind::Throw,
    SfxKind::Death,
    SfxKind::LevelClear,
    SfxKind::MaskCrack,
    SfxKind::Elevator,
];

/// Where a pre-rendered voice plugs back into the live bus at play time: the
/// same dry input + wet send its live synthesis uses, so the room, compressor
/// and soft-clip behavior stay live and identical (only the *synthesis* is
/// baked — always dry, before the reverb send).
#[derive(Clone, Copy)]
enum SfxRoute {
    /// The gun / hit path ([`AudioEngine::voice_real`]) at this wet level.
    Real(f64),
    /// The melee / misc. compressed path ([`AudioEngine::voice`]) at this
    /// wet level.
    Melee(f64),
    /// The bus's pre-wired default room voice ([`AudioEngine::sfx_out`]).
    Room,
}

/// Per-kind pre-render parameters (mirrors the values inside each `play_*`).
struct SfxSpec {
    /// The live-bus routing reapplied at play time.
    route: SfxRoute,
    /// Seconds of dry voice to render (covers the longest tail incl. jitter;
    /// the reverb tail is added live by the convolver, not baked).
    len: f64,
    /// Per-play `playback_rate` jitter — matches the magnitude of the live
    /// path's top-level per-play pitch jitter (0 for the musical chimes,
    /// which the live path never detunes).
    rate_jitter: f64,
}

impl SfxKind {
    /// The pre-render spec for this kind. `route`/`wet` mirror the `voice*`
    /// call at the top of the kind's `synth_*` builder.
    fn spec(self) -> SfxSpec {
        let (route, len, rate_jitter) = match self {
            SfxKind::AttackGun => (SfxRoute::Real(REAL_762X39.wet), 1.1, 0.05),
            SfxKind::AttackMachinegun => (SfxRoute::Real(REAL_556.wet), 1.4, 0.05),
            SfxKind::AttackShotgun => (SfxRoute::Real(REAL_762X54R.wet), 1.4, 0.05),
            SfxKind::AttackClub => (SfxRoute::Melee(0.16), 0.55, 0.06),
            SfxKind::HitGun => (SfxRoute::Real(0.5), 0.85, 0.05),
            SfxKind::HitMachinegun => (SfxRoute::Real(0.5), 1.35, 0.08),
            SfxKind::HitShotgun => (SfxRoute::Real(0.55), 0.85, 0.05),
            SfxKind::HitClub => (SfxRoute::Real(0.5), 0.95, 0.05),
            SfxKind::EnemyDown => (SfxRoute::Real(0.55), 1.5, 0.05),
            SfxKind::PlayerHurt => (SfxRoute::Melee(0.30), 1.6, 0.05),
            SfxKind::Pickup => (SfxRoute::Room, 0.35, 0.0),
            SfxKind::Throw => (SfxRoute::Room, 0.4, 0.0),
            SfxKind::Death => (SfxRoute::Room, 0.85, 0.0),
            SfxKind::LevelClear => (SfxRoute::Room, 0.6, 0.0),
            SfxKind::MaskCrack => (SfxRoute::Room, 0.55, 0.0),
            SfxKind::Elevator => (SfxRoute::Room, 1.5, 0.0),
        };
        SfxSpec {
            route,
            len,
            rate_jitter,
        }
    }
}

/// How many kinds at the head of [`SFX_KINDS`] are baked BEFORE the music
/// voices: the combat sounds (attacks, hits, enemy-down, hurt, pickup,
/// throw) a first firefight needs — the rare tail (death, level-clear,
/// mask-crack, elevator) bakes after the music voices instead.
const SFX_COMBAT_KINDS: usize = 12;

/// A JS promise callback kept alive in [`BakedSfx::pending`].
type RenderCallback = Closure<dyn FnMut(JsValue)>;

/// The pre-rendered one-shot voices, shared with the async offline-render
/// completion callbacks via `Rc`. A kind switches to its baked buffers —
/// permanently — once all [`SFX_VARIANTS`] of them landed; until then its
/// `play_*` keeps the live per-node synthesis (the fallback), so nothing
/// changes while rendering is still in flight (or unavailable).
struct BakedSfx {
    /// `bufs[kind as usize]` = the finished variants for that kind (mono,
    /// dry, at the live context's sample rate).
    bufs: RefCell<Vec<Vec<AudioBuffer>>>,
    /// Next variant index to kick, `kind * SFX_VARIANTS + variant`.
    next: Cell<usize>,
    /// The in-flight render's completion closures, kept alive until the next
    /// kick replaces them (renders are sequential, so by then they have
    /// fired). Bounded: at most one pair ever lives here.
    pending: RefCell<Vec<RenderCallback>>,
}

/// One pre-renderable MUSIC voice: a tracker channel role × the scale
/// degree it plays (drums carry no pitch; a pad key bakes its whole triad
/// into one buffer). The set of keys a song can ever schedule is FINITE —
/// its lanes are static pattern data — so [`AudioEngine::music_keys`]
/// enumerates it exactly and each key is baked at its exact pitch (no
/// `playback_rate` transposition: the timbre is untouched).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MusicKey {
    /// Bass lane note at this scale degree.
    Bass(i32),
    /// Lead lane note at this scale degree.
    Lead(i32),
    /// Pad lane note: the full triad (root + third + fifth) in one buffer.
    Pad(i32),
    /// Arp lane note at this scale degree.
    Arp(i32),
    /// Drum lane kick.
    Kick,
    /// Drum lane hat.
    Hat,
    /// Drum lane snare.
    Snare,
}

/// One music voice's bake slot: `None` until its offline render lands, then
/// the finished mono buffer (song gain and envelope baked in — velocity/mix
/// are per-song constants, so playback needs no gain node at all).
struct MusicSlot {
    key: MusicKey,
    buf: Option<AudioBuffer>,
}

/// The pre-rendered note voices of the CURRENT song (see the
/// "pre-rendered voices" section of [`AudioEngine`]): `schedule_step`
/// swaps a note's per-node oscillator synthesis for one
/// `AudioBufferSourceNode` into the same music bus the moment its slot is
/// baked; unbaked slots fall back to the live path per note. Shared with
/// the async completion callbacks via `Rc`.
struct BakedMusic {
    /// The finite voice set of the current song, in bake-priority order
    /// (drums first — the densest lane — then bass, lead, arp, pad).
    slots: RefCell<Vec<MusicSlot>>,
    /// Next slot index to kick.
    next: Cell<usize>,
    /// Bumped by every [`AudioEngine::rebuild_music_bake`] (song switch): a
    /// completion callback for a previous song's render sees the mismatch
    /// and drops its buffer instead of landing it in the wrong slot.
    gen: Cell<u32>,
    /// The in-flight render's completion closures (same lifecycle as
    /// [`BakedSfx::pending`]).
    pending: RefCell<Vec<RenderCallback>>,
}

/// The offline render target while a pre-render is being *built*: the voice
/// builders create their nodes in `ctx` and the voice front-end feeds `sink`
/// (the offline destination) instead of the live bus.
struct OfflineRender {
    ctx: BaseAudioContext,
    sink: web_sys::AudioNode,
}

/// The persistent SFX bus. Every one-shot flows in through a per-shot *voice*
/// (see [`AudioEngine::voice`]) that splits into a dry path and a wet send.
/// There are two parallel paths: the melee / misc. one drawn below
/// (compressor plus limiter, darkening IR) and the gun / hit one (dry_real
/// and reverb_real_in: uncompressed but for a −1 dB / 4:1 safety, a longer
/// brighter IR), both summing into the bus soft-clip:
///
/// ```text
///  voice ──(soft-clip)──┬──────────────► dry ─────────────┐
///                       └─ send(wet) ──► reverb_in ─► HPF ─► convolver ─► return ─┤
///                                                                                  ▼
///                                        compressor ─► limiter ─► bus soft-clip ─► trim ─► out
/// ```
///
/// The convolver holds a synthesized stereo impulse response (see
/// [`AudioEngine::make_impulse`]) so every shot and clang gets the room tail a
/// real recording has; the compressor glues the layers of a shot into one
/// punchy transient and the gentle bus clipper adds the "hot mic" edge.
struct SfxBus {
    /// Dry input — sums straight into the compressor.
    dry: GainNode,
    /// Reverb input — feeds the convolver (through a low-cut).
    reverb_in: GainNode,
    /// Gun / hit dry input: bypasses the compressor and the limiter (only a
    /// gentle −1 dB / 4:1 safety) so a crack keeps its 18–22 dB crest.
    dry_real: GainNode,
    /// Gun / hit reverb input: a longer, brighter impulse response.
    reverb_real_in: GainNode,
    /// A pre-wired default voice (dry + a light send) for the misc. SFX that
    /// don't build their own voice (pickup, throw, elevator, ...).
    room: GainNode,
}

/// The self-contained audio engine. Construct once, hand it around, drive
/// `update()` from the game loop.
pub struct AudioEngine {
    /// `None` if the browser refused to give us an audio context.
    ctx: Option<AudioContext>,
    /// Master enable (the SETTINGS sound toggle): off = the whole
    /// `AudioContext` is suspended — music and SFX alike — and the autoplay
    /// unlock refuses to resume it. `Cell` because `play_*` take `&self`.
    enabled: Cell<bool>,
    /// Pre-rendered white noise, reused (via cheap buffer-source nodes) for
    /// every percussive/whoosh sound.
    noise: Option<AudioBuffer>,
    /// The SFX bus (reverb + compressor + soft clip), if it could be built.
    sfx: Option<SfxBus>,
    /// Tiny xorshift state for per-shot randomization (pitch / timing jitter,
    /// ricochet chance). `Cell` because the `play_*` API takes `&self`.
    rng: Cell<u32>,
    /// Input gain for the whole music mix — every music note connects here.
    music_bus: Option<GainNode>,
    /// Lowpass filter on the music bus, cutoff swept once per bar (synthwave).
    music_filter: Option<BiquadFilterNode>,
    music_playing: bool,
    /// Absolute audio-clock time of the next music step to schedule.
    next_note_time: f64,
    /// Index of the currently-playing section within the song's arrangement.
    section: usize,
    /// Current step index inside the currently-playing section.
    step: usize,
    /// The song currently driving the scheduler.
    song: SongSpec,
    /// Per-channel mute flags (bass/lead/pad/arp/drums).
    mute: [bool; NUM_CHANNELS],
    /// Per-channel solo flags. If any is set, only soloed channels sound.
    solo: [bool; NUM_CHANNELS],
    /// Pre-rendered one-shot voices (see [`BakedSfx`]); `Rc` so the async
    /// offline-render completion callbacks can write finished buffers in.
    baked: Rc<BakedSfx>,
    /// Pre-rendered music note voices of the current song ([`BakedMusic`]).
    baked_music: Rc<BakedMusic>,
    /// One offline render in flight at a time — SFX or music (they are
    /// chained frame by frame so graph construction never bursts onto a
    /// single frame). Shared with the completion callbacks via `Rc`.
    renders_in_flight: Rc<Cell<u32>>,
    /// How many offline renders may run concurrently (the game loop sets it
    /// per screen: gentle in-game, aggressive on loading/menu screens where
    /// a construction hitch cannot be seen).
    pump_budget: Cell<u32>,
    /// Set when `OfflineAudioContext` turns out to be unavailable: the whole
    /// bake queue (SFX and music) is abandoned, everything stays live.
    render_dead: Cell<bool>,
    /// `Some` only while an offline pre-render is being built: the voice
    /// builders then target this context/sink instead of the live bus.
    render: RefCell<Option<OfflineRender>>,
}

impl AudioEngine {
    /// Try to create the audio context. Never fails hard — on any error the
    /// engine simply stays silent.
    pub fn new() -> Self {
        let ctx = AudioContext::new().ok();
        let noise = ctx.as_ref().and_then(Self::make_noise);
        let sfx = ctx.as_ref().and_then(Self::make_sfx_bus);
        let (music_bus, music_filter) = ctx
            .as_ref()
            .map(Self::make_music_bus)
            .unwrap_or((None, None));
        let engine = Self {
            ctx,
            noise,
            sfx,
            enabled: Cell::new(true),
            rng: Cell::new(0x2545_F491),
            music_bus,
            music_filter,
            music_playing: false,
            next_note_time: 0.0,
            section: 0,
            step: 0,
            song: SONGS[0],
            mute: [false; NUM_CHANNELS],
            solo: [false; NUM_CHANNELS],
            baked: Rc::new(BakedSfx {
                bufs: RefCell::new(vec![Vec::new(); SFX_KINDS.len()]),
                next: Cell::new(0),
                pending: RefCell::new(Vec::new()),
            }),
            baked_music: Rc::new(BakedMusic {
                slots: RefCell::new(Vec::new()),
                next: Cell::new(0),
                gen: Cell::new(0),
                pending: RefCell::new(Vec::new()),
            }),
            renders_in_flight: Rc::new(Cell::new(0)),
            pump_budget: Cell::new(1),
            render_dead: Cell::new(false),
            render: RefCell::new(None),
        };
        engine.rebuild_music_bake();
        engine
    }

    /// Resume the context. Browsers start it suspended until a user gesture,
    /// so the integrator should call this on the first input (e.g. pressing
    /// Enter to start the game).
    pub fn resume(&self) {
        if !self.enabled.get() {
            return;
        }
        if let Some(ctx) = &self.ctx {
            let _ = ctx.resume();
        }
    }

    /// The SETTINGS sound toggle: `false` suspends the whole `AudioContext`
    /// (music and SFX go silent instantly, nothing else changes — schedulers
    /// idle against the frozen audio clock), `true` resumes it.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.set(on);
        if let Some(ctx) = &self.ctx {
            if on {
                let _ = ctx.resume();
            } else {
                let _ = ctx.suspend();
            }
        }
    }

    /// Whether sound is currently enabled (the SETTINGS checkbox state).
    pub fn is_enabled(&self) -> bool {
        self.enabled.get()
    }

    /// How many offline pre-renders may run concurrently (see `update`).
    pub fn set_pump_budget(&self, n: u32) {
        self.pump_budget.set(n.max(1));
    }

    /// Pre-render progress `(done, total)` across the SFX variants and the
    /// current song's note voices — the loading screen's PRECOMPUTING bar.
    pub fn bake_progress(&self) -> (u32, u32) {
        let sfx_done: usize = self.baked.bufs.borrow().iter().map(Vec::len).sum();
        let sfx_total = SFX_KINDS.len() * SFX_VARIANTS;
        let slots = self.baked_music.slots.borrow();
        let music_done = slots.iter().filter(|s| s.buf.is_some()).count();
        let music_total = slots.len();
        (
            (sfx_done + music_done) as u32,
            (sfx_total + music_total) as u32,
        )
    }

    /// Whether the pre-render queue is finished (or can never finish — no
    /// `OfflineAudioContext`, or renders died: the live path serves forever
    /// and the loading screen must not wait).
    pub fn bake_complete(&self) -> bool {
        if self.ctx.is_none() || self.render_dead.get() {
            return true;
        }
        let (done, total) = self.bake_progress();
        done >= total
    }

    // --- one-shot SFX: attacks ---------------------------------------------
    //
    // The guns follow the measured profile of field-recorded gunshots
    // (.22LR / 5.56 / 7.62×39 / 7.62×54R singles; band energies, envelope
    // and decay times): a huge uncompressed bright crack (crest 18–22 dB,
    // centroid ~5–6 kHz at 5 ms, peak at +5–15 ms), a mid-dominant body
    // that holds a ~100 ms plateau at −5 dB then drops to −21 dB by 200 ms,
    // a low-mid layer a few dB under the mids, almost no sub, no growl, and
    // a long bright diffuse room tail (the reverb) carrying everything past
    // ~200 ms. Each shot is a [`RealShot`] recipe rendered by
    // [`Self::real_shot`] into a per-shot voice on the uncompressed bus, with
    // ±5 % pitch and ±3 ms timing jitter per play; mechanics (slide, bolt,
    // pump) follow.

    /// GUN attack — a 7.62×39 single: bright uncompressed crack,
    /// mid-dominant plateau body, the room, then the slide cycling.
    pub fn play_attack_gun(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::AttackGun) {
            return; // pre-rendered voice fired: 2–3 nodes instead of ~50
        }
        self.synth_attack_gun();
    }

    /// The live synthesis of [`Self::play_attack_gun`] — also what the
    /// offline pre-render runs (see [`Self::render_variant`]).
    fn synth_attack_gun(&self) {
        let t = self.t0();
        let out = match self.voice_real(REAL_762X39.wet, 1.3) {
            Some(v) => v,
            None => return,
        };
        let j = self.jit(0.05);
        self.real_shot(&out, t, j, 1.0, &REAL_762X39);
        // Slide back / slide forward (bright, no low).
        let s1 = t + 0.08 + self.rand() * 0.012;
        let s2 = s1 + 0.055 + self.rand() * 0.012;
        self.tick(&out, s1, 2600.0 * j, 0.16);
        self.tick(&out, s2, 1900.0 * j, 0.13);
        if self.chance(0.35) {
            self.tinkle(&out, t + 0.30 + self.rand() * 0.10, 0.05);
        }
    }

    /// MACHINEGUN attack — 5.56 rounds at the same ~1000 rpm:
    /// each a bright crack + plateau body, their room tails overlapping into
    /// a continuous wash; bolt clacks under each round, clatter after.
    pub fn play_attack_machinegun(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::AttackMachinegun) {
            return;
        }
        self.synth_attack_machinegun();
    }

    /// Live synthesis of [`Self::play_attack_machinegun`] (also pre-rendered).
    fn synth_attack_machinegun(&self) {
        let t = self.t0();
        let out = match self.voice_real(REAL_556.wet, 1.3) {
            Some(v) => v,
            None => return,
        };
        let rounds = 8;
        let spacing = 0.058;
        let mut at = t;
        for i in 0..rounds {
            let j = self.jit(0.05);
            let level = if i == 0 { 1.0 } else { 0.9 };
            self.real_shot(&out, at, j, level, &REAL_556);
            self.tick(&out, at + 0.018, 1800.0 * j, 0.08);
            if i % 3 == 1 {
                self.tinkle(&out, at + 0.12 + self.rand() * 0.05, 0.03);
            }
            at += spacing * self.jit(0.05);
        }
        self.tick(&out, at + 0.02, 2300.0, 0.16);
        self.tick(&out, at + 0.075, 1500.0, 0.13);
        self.noise_env(
            &out,
            at + 0.02,
            0.0,
            0.05,
            0.2,
            BiquadFilterType::Highpass,
            2500.0,
            2500.0,
            0.7,
        );
        self.tinkle(&out, at + 0.24 + self.rand() * 0.08, 0.05);
    }

    /// SHOTGUN attack — a 7.62×54R-sized single with a real
    /// low-mid body, the room, then a real pump reload: ~1 s of multiple
    /// bright 2–8 kHz clacks (pump back at +270 ms, forward, shell), no low.
    pub fn play_attack_shotgun(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::AttackShotgun) {
            return;
        }
        self.synth_attack_shotgun();
    }

    /// Live synthesis of [`Self::play_attack_shotgun`] (also pre-rendered).
    fn synth_attack_shotgun(&self) {
        let t = self.t0();
        let out = match self.voice_real(REAL_762X54R.wet, 1.3) {
            Some(v) => v,
            None => return,
        };
        let j = self.jit(0.05);
        self.real_shot(&out, t, j, 1.0, &REAL_762X54R);
        self.real_pump(&out, t + 0.27 + self.rand() * 0.03, j);
    }

    /// CLUB attack — just the swing: a clean wind-like WHOOSH of air ripping
    /// past the bar. Bandpass noise that swells and sweeps up as the bar
    /// accelerates, then a closing, darker layer as it passes, an airy top
    /// and a faint low doppler-ish dip. No clang, no thud — the impact lives
    /// in [`Self::play_hit_club`]. Style-independent (the round-2 recipe,
    /// restored verbatim).
    pub fn play_attack_club(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::AttackClub) {
            return;
        }
        self.synth_attack_club();
    }

    /// Live synthesis of [`Self::play_attack_club`] (also pre-rendered).
    fn synth_attack_club(&self) {
        let t = self.t0();
        let out = match self.voice(0.16, 1.0) {
            Some(v) => v,
            None => return,
        };
        let j = self.jit(0.06);
        // Opening whoosh: swells over ~90 ms while sweeping up in pitch.
        self.noise_env(
            &out,
            t,
            0.09,
            0.24,
            0.6,
            BiquadFilterType::Bandpass,
            320.0 * j,
            1500.0 * j,
            1.1,
        );
        // Closing tail: darker, sweeping back down as the bar passes.
        self.noise_env(
            &out,
            t + 0.10,
            0.04,
            0.20,
            0.42,
            BiquadFilterType::Bandpass,
            1400.0 * j,
            380.0 * j,
            1.0,
        );
        // Airy top layer.
        self.noise_env(
            &out,
            t + 0.02,
            0.07,
            0.20,
            0.16,
            BiquadFilterType::Highpass,
            1800.0,
            3200.0,
            0.7,
        );
        // Wind body: low-passed rush under the whoosh.
        self.noise_env(
            &out,
            t + 0.01,
            0.09,
            0.24,
            0.4,
            BiquadFilterType::Lowpass,
            520.0 * j,
            240.0,
            0.9,
        );
        // Faint doppler-ish pitch dip as the bar goes by.
        self.tone_out(
            &out,
            135.0 * j,
            88.0 * j,
            t + 0.06,
            0.20,
            0.07,
            0.06,
            OscillatorType::Sine,
        );
    }

    // --- one-shot SFX: hits (impact on a metal bot) ------------------------
    //
    // Analysis-driven resynthesis of a reference bullet-on-metal recording
    // (spectral modeling: the measured sine partials + noise bands +
    // transients in [`sms_tables`], replayed by [`Self::sms_play`]), on the
    // uncompressed bus with the bright room, ±5 % pitch per play. The bigger
    // hits stack / slow / lower the same model and add pellet debris, a bat
    // contact or a hollow knock on top.

    /// GUN hit — analysis-driven resynthesis of the reference
    /// recording BulletImpactMetal02 (see [`sms_tables`]): the 16 measured
    /// partials + 9 noise bands replayed through [`Self::sms_play`] on the
    /// real bus (wet 0.5, drive 1.3), ±5 % pitch; nothing else on top but
    /// the 40 % quiet ricochet.
    pub fn play_hit_gun(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::HitGun) {
            return;
        }
        self.synth_hit_gun();
    }

    /// Live synthesis of [`Self::play_hit_gun`] (also pre-rendered).
    fn synth_hit_gun(&self) {
        let t = self.t0();
        let out = match self.voice_real(0.5, 1.3) {
            Some(v) => v,
            None => return,
        };
        let pitch = self.jit(0.05) as f32;
        self.sms_metal02(&out, t, SMS_HIT_GAIN, pitch, 1.0);
        if self.chance(0.4) {
            self.real_ricochet(&out, t + 0.008, pitch as f64, 0.35);
        }
    }

    /// MACHINEGUN hit — the METAL02 resynthesis per round at the
    /// burst rate, level 0.75, ±8 % pitch, a ricochet on about one round in
    /// three.
    pub fn play_hit_machinegun(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::HitMachinegun) {
            return;
        }
        self.synth_hit_machinegun();
    }

    /// Live synthesis of [`Self::play_hit_machinegun`] (also pre-rendered).
    fn synth_hit_machinegun(&self) {
        let t = self.t0();
        let out = match self.voice_real(0.5, 1.3) {
            Some(v) => v,
            None => return,
        };
        let rounds = 8;
        let spacing = 0.058;
        let mut at = t;
        for _ in 0..rounds {
            let pitch = self.jit(0.08) as f32;
            self.sms_metal02(&out, at, SMS_HIT_GAIN * 0.75, pitch, 1.0);
            if self.chance(0.33) {
                self.real_ricochet(&out, at + 0.008, pitch as f64, 0.25);
            }
            at += spacing * self.jit(0.05);
        }
    }

    /// SHOTGUN hit — 3–4 overlapping METAL02 plays spread over
    /// 25 ms at pitch 0.85–1.0 (a bigger plate) plus the pellet debris ticks.
    pub fn play_hit_shotgun(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::HitShotgun) {
            return;
        }
        self.synth_hit_shotgun();
    }

    /// Live synthesis of [`Self::play_hit_shotgun`] (also pre-rendered).
    fn synth_hit_shotgun(&self) {
        let t = self.t0();
        let out = match self.voice_real(0.55, 1.3) {
            Some(v) => v,
            None => return,
        };
        let plays = 3 + (self.rand() * 2.0) as usize;
        for k in 0..plays {
            let at = if k == 0 { t } else { t + self.rand() * 0.025 };
            let pitch = (0.85 + self.rand() * 0.15) as f32;
            let lvl = if k == 0 { 1.0 } else { 0.7 };
            self.sms_metal02(&out, at, SMS_HIT_GAIN * 0.8 * lvl, pitch, 1.0);
        }
        // Pellet debris: sparse bright ticks over the first ~120 ms.
        for _ in 0..8 {
            let at = t + 0.004 + self.rand() * 0.12;
            let hz = 4000.0 + self.rand() * 6000.0;
            self.noise_full(
                &out,
                at,
                0.0,
                0.001,
                0.001,
                0.5 * (0.3 + self.rand() * 0.7),
                BiquadFilterType::Bandpass,
                hz,
                hz,
                1.0,
            );
        }
    }

    /// CLUB hit — one METAL02 play at pitch 0.7–0.8 and time
    /// scale 1.25 (a bigger, slower body) with the bat contact tick and a
    /// hollow 130 Hz body knock kept on top.
    pub fn play_hit_club(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::HitClub) {
            return;
        }
        self.synth_hit_club();
    }

    /// Live synthesis of [`Self::play_hit_club`] (also pre-rendered).
    fn synth_hit_club(&self) {
        let t = self.t0();
        let out = match self.voice_real(0.5, 1.3) {
            Some(v) => v,
            None => return,
        };
        let j = self.jit(0.05);
        // Bat contact: 2–3 bounces ~8 ms apart.
        let mut at = t;
        for k in 0..3 {
            if k > 0 {
                at += 0.008 * (0.5 + self.rand());
            }
            let amp = if k == 0 { 1.0 } else { 0.4 + self.rand() * 0.6 };
            self.click(&out, at, 0.28 * amp);
            self.noise_full(
                &out,
                at,
                0.0,
                0.0025,
                0.0025,
                0.35 * amp,
                BiquadFilterType::Bandpass,
                5000.0 * j,
                4000.0 * j,
                0.6,
            );
        }
        let pitch = (0.7 + self.rand() * 0.1) as f32;
        self.sms_metal02(&out, t + 0.002, SMS_HIT_GAIN * 1.1, pitch, 1.25);
        self.hollow_knock(&out, t + 0.004, 130.0 * j, 0.08, 0.6);
    }

    /// A rogue AI goes down — a metal robot collapsing — a quiet dying servo, then
    /// three falling METAL02 plays (pitch 0.9 / 0.8 / 0.7, time scale 1.2)
    /// with loose-part rattle between and after, ending on a low rumble.
    pub fn play_enemy_down(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::EnemyDown) {
            return;
        }
        self.synth_enemy_down();
    }

    /// Live synthesis of [`Self::play_enemy_down`] (also pre-rendered).
    fn synth_enemy_down(&self) {
        let t = self.t0();
        let out = match self.voice_real(0.55, 1.3) {
            Some(v) => v,
            None => return,
        };
        let j = self.jit(0.05);
        // Small servo whine sagging (quiet).
        self.swell_tone(
            &out,
            640.0 * j,
            420.0 * j,
            t,
            0.01,
            0.06,
            0.10,
            OscillatorType::Sawtooth,
            0.05,
        );
        // Falling: knee, hip, then the whole hull.
        let c1 = t + 0.05 + self.rand() * 0.03;
        let c2 = c1 + 0.14 + self.rand() * 0.05;
        let c3 = c2 + 0.16 + self.rand() * 0.06;
        let pj = j as f32;
        self.sms_metal02(&out, c1, SMS_HIT_GAIN * 0.6, 0.9 * pj, 1.2);
        self.hollow_knock(&out, c1 + 0.004, 120.0 * j, 0.08, 0.35);
        self.rattle(&out, c1 + 0.04, c2 - 0.01, 0.35 * j);
        self.sms_metal02(&out, c2, SMS_HIT_GAIN * 0.8, 0.8 * pj, 1.2);
        self.hollow_knock(&out, c2 + 0.004, 110.0 * j, 0.08, 0.45);
        self.rattle(&out, c2 + 0.04, c3 - 0.01, 0.4 * j);
        self.sms_metal02(&out, c3, SMS_HIT_GAIN, 0.7 * pj, 1.2);
        self.hollow_knock(&out, c3 + 0.004, 100.0 * j, 0.09, 0.6);
        // Loose parts rattling after, and the low rumble tail.
        self.rattle(&out, c3 + 0.06, c3 + 0.35, 0.3 * j);
        self.noise_full(
            &out,
            c3 + 0.02,
            0.08,
            0.65,
            0.65,
            0.4,
            BiquadFilterType::Bandpass,
            240.0 * j,
            150.0 * j,
            0.9,
        );
    }

    /// A hollow body knock: a short sine (`hz`, gliding down 10 %) plus a
    /// low-passed thump, `len` seconds, at `level`.
    fn hollow_knock(&self, out: &web_sys::AudioNode, t: f64, hz: f64, len: f64, level: f64) {
        let f = hz * self.jit(0.05);
        self.swell_tone(
            out,
            f,
            f * 0.9,
            self.jt(t, 0.002),
            0.004,
            len * 0.25,
            level,
            OscillatorType::Sine,
            0.02,
        );
        self.noise_full(
            out,
            self.jt(t, 0.002),
            0.0,
            len,
            len,
            level * 0.8,
            BiquadFilterType::Lowpass,
            f * 2.2,
            f * 1.2,
            1.0,
        );
    }

    /// The METAL02 model through [`Self::sms_play`].
    fn sms_metal02(
        &self,
        out: &web_sys::AudioNode,
        t0: f64,
        gain: f32,
        pitch: f32,
        time_scale: f32,
    ) {
        self.sms_play(
            out,
            t0,
            sms_tables::METAL02_HOP,
            sms_tables::METAL02_PARTIALS,
            sms_tables::METAL02_NOISE,
            sms_tables::METAL02_TRANSIENTS,
            gain,
            pitch,
            time_scale,
        );
    }

    /// The empirical trim for an SMS noise band centred on `fc` (see
    /// [`SMS_BAND_TRIMS`]).
    fn sms_band_trim(fc: f64) -> f64 {
        SMS_BAND_TRIMS
            .iter()
            .find(|(edge, _)| fc < *edge)
            .map(|(_, k)| *k)
            .unwrap_or(1.0)
    }

    /// Generic spectral-modeling (sines + noise) player. Replays an analysed
    /// sound from its tables: each partial is a sine oscillator at
    /// `freq × pitch` (±0.4 % per-play detune) whose gain follows the
    /// partial's amplitude curve (`N` points every `hop × time_scale`
    /// seconds, via `setValueCurveAtTime`); each noise band is the looped
    /// noise buffer through a bandpass at `center × pitch` with
    /// `Q = sqrt(r) / (r − 1)` (`r` = hi/lo edge ratio, clamped 0.5–4),
    /// whose gain follows the band's curve, normalised so the band's RMS
    /// matches the curve (bandpass output RMS ≈ 0.577·sqrt(fc/Q/(sr/2)) for
    /// the ±1 uniform noise buffer, then the empirical `SMS_BAND_TRIMS`);
    /// partials above `SMS_HIGH_PARTIAL_HZ` get `SMS_HIGH_PARTIAL_GAIN`;
    /// each transient `(t, amp)` is a 0.5 ms click through a random
    /// 4–12 kHz Q 1 bandpass at `t0 + t × time_scale`, level
    /// `amp × gain × 0.6`. `gain` scales everything.
    #[allow(clippy::too_many_arguments)]
    fn sms_play<const N: usize>(
        &self,
        out: &web_sys::AudioNode,
        t0: f64,
        hop: f32,
        partials: &[(f32, [f32; N])],
        noise: &[(f32, f32, [f32; N])],
        transients: &[(f32, f32)],
        gain: f32,
        pitch: f32,
        time_scale: f32,
    ) {
        let (ctx, buf) = match (self.bctx(), &self.noise) {
            (Some(c), Some(b)) => (c, b),
            _ => return,
        };
        if N == 0 {
            return;
        }
        let dur = (N as f64) * (hop as f64) * (time_scale as f64).max(0.05);
        let sr = ctx.sample_rate() as f64;
        let nyq = sr * 0.5;
        // Partials.
        for (freq, table) in partials {
            let f = (*freq as f64) * (pitch as f64) * self.jit(0.004);
            if f < 20.0 || f > nyq * 0.95 {
                continue;
            }
            let (osc, g) = match (ctx.create_oscillator(), ctx.create_gain()) {
                (Ok(o), Ok(g)) => (o, g),
                _ => continue,
            };
            osc.set_type(OscillatorType::Sine);
            let _ = osc.frequency().set_value_at_time(f as f32, t0);
            let pg = if f > SMS_HIGH_PARTIAL_HZ {
                gain * SMS_HIGH_PARTIAL_GAIN
            } else {
                gain
            };
            let mut curve: Vec<f32> = table.iter().map(|a| a * pg).collect();
            let _ = g.gain().set_value_curve_at_time(&mut curve, t0, dur);
            let _ = osc.connect_with_audio_node(&g);
            let _ = g.connect_with_audio_node(out);
            let sched: &web_sys::AudioScheduledSourceNode = osc.as_ref();
            let _ = sched.start_with_when(t0);
            let _ = sched.stop_with_when(t0 + dur + 0.02);
        }
        // Noise bands.
        for (center, ratio, table) in noise {
            let fc = (*center as f64) * (pitch as f64);
            if fc < 15.0 || fc > nyq * 0.95 {
                continue;
            }
            let r = (*ratio as f64).max(1.01);
            let q = (r.sqrt() / (r - 1.0)).clamp(0.5, 4.0);
            let (src, filt, g) = match (
                ctx.create_buffer_source(),
                ctx.create_biquad_filter(),
                ctx.create_gain(),
            ) {
                (Ok(s), Ok(f), Ok(g)) => (s, f, g),
                _ => continue,
            };
            src.set_buffer(Some(buf));
            src.set_loop(true);
            filt.set_type(BiquadFilterType::Bandpass);
            let _ = filt.frequency().set_value_at_time(fc as f32, t0);
            let _ = filt.q().set_value_at_time(q as f32, t0);
            // RMS normalisation of the band-passed uniform noise.
            let rms = 0.577 * (fc / q / nyq).min(1.0).sqrt();
            let norm = (SMS_NOISE_TRIM * Self::sms_band_trim(fc) / rms.max(1e-4)) as f32;
            let mut curve: Vec<f32> = table.iter().map(|a| a * gain * norm).collect();
            let _ = g.gain().set_value_curve_at_time(&mut curve, t0, dur);
            let _ = src.connect_with_audio_node(&filt);
            let _ = filt.connect_with_audio_node(&g);
            let _ = g.connect_with_audio_node(out);
            let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
            let offset = self.rand() * (NOISE_SECONDS - 0.05);
            let _ = src.start_with_when_and_grain_offset(t0, offset);
            let _ = sched.stop_with_when(t0 + dur + 0.02);
        }
        // Transients.
        for (tt, amp) in transients {
            let at = t0 + (*tt as f64) * (time_scale as f64);
            let hz = 4000.0 + self.rand() * 8000.0;
            self.noise_full(
                out,
                at,
                0.0,
                0.0005,
                0.0005,
                (*amp as f64) * (gain as f64) * 0.6,
                BiquadFilterType::Bandpass,
                hz,
                hz,
                1.0,
            );
        }
    }

    /// Loose-part rattle debris between `from` and `to`: a handful of small
    /// bright ticks and micro-clicks at random times, quiet.
    fn rattle(&self, out: &web_sys::AudioNode, from: f64, to: f64, level: f64) {
        let span = (to - from).max(0.02);
        let n = 3 + (self.rand() * 4.0) as usize;
        for _ in 0..n {
            let at = from + self.rand() * span;
            let g = level * (0.3 + self.rand() * 0.7);
            if self.chance(0.5) {
                self.tick(out, at, 700.0 + self.rand() * 900.0, g * 0.25);
            } else {
                let hz = 3500.0 + self.rand() * 5000.0;
                self.noise_full(
                    out,
                    at,
                    0.0,
                    0.003,
                    0.003,
                    g,
                    BiquadFilterType::Bandpass,
                    hz,
                    hz,
                    1.0,
                );
            }
        }
    }

    // --- one-shot SFX: legacy aliases --------------------------------------
    //
    // Kept so existing callers (lib.rs) keep compiling. They forward to the new
    // per-weapon methods above.

    /// Legacy alias — a generic gunshot. Forwards to [`Self::play_attack_gun`].
    pub fn play_shoot(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        self.play_attack_gun();
    }

    /// Legacy alias — a generic bullet impact. Forwards to [`Self::play_hit_gun`].
    pub fn play_hit(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        self.play_hit_gun();
    }

    /// Legacy alias for the old "fist" name — now the CLUB swing.
    pub fn play_attack_fist(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        self.play_attack_club();
    }

    /// Legacy alias for the old "fist" name — now the CLUB impact.
    pub fn play_hit_fist(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        self.play_hit_club();
    }

    // --- one-shot SFX: non-combat ------------------------------------------

    /// Bright rising two-tone — weapon pickup / swap.
    pub fn play_pickup(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::Pickup) {
            return;
        }
        self.synth_pickup();
    }

    /// Live synthesis of [`Self::play_pickup`] (also pre-rendered).
    fn synth_pickup(&self) {
        let t = self.t0();
        self.tone(523.25, 523.25, t, 0.08, 0.20, OscillatorType::Triangle);
        self.tone(
            783.99,
            783.99,
            t + 0.07,
            0.12,
            0.22,
            OscillatorType::Triangle,
        );
    }

    /// Filtered noise whoosh — a thrown weapon.
    pub fn play_throw(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::Throw) {
            return;
        }
        self.synth_throw();
    }

    /// Live synthesis of [`Self::play_throw`] (also pre-rendered).
    fn synth_throw(&self) {
        let t = self.t0();
        self.noise(t, 0.22, 0.22, BiquadFilterType::Highpass, 200.0, 1600.0);
    }

    /// The player takes a hit — mean: a hard body blow (click, low-passed
    /// slam, a big 130 → 45 Hz thump, a crunch band) driven hot, with a
    /// loud, longer clipped grunt.
    pub fn play_player_hurt(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::PlayerHurt) {
            return;
        }
        self.synth_player_hurt();
    }

    /// Live synthesis of [`Self::play_player_hurt`] (also pre-rendered).
    fn synth_player_hurt(&self) {
        let t = self.t0();
        let out = match self.voice(0.30, 4.0) {
            Some(v) => v,
            None => return,
        };
        let j = self.jit(0.05);
        self.click(&out, t, 1.0);
        self.noise_env(
            &out,
            t,
            0.0,
            0.12,
            1.4,
            BiquadFilterType::Lowpass,
            900.0,
            100.0,
            0.8,
        );
        self.noise_env(
            &out,
            t,
            0.0,
            0.08,
            0.9,
            BiquadFilterType::Bandpass,
            600.0 * j,
            200.0,
            0.7,
        );
        self.wham(&out, t, 1.3);
        // Grunt: a rough low buzz through a swept vowel-ish bandpass.
        self.grunt(&out, t + 0.015, 128.0 * j, 0.20, 1.0);
    }

    /// Longer downward dive — the player dies / SYSTEM HALTED.
    pub fn play_death(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::Death) {
            return;
        }
        self.synth_death();
    }

    /// Live synthesis of [`Self::play_death`] (also pre-rendered).
    fn synth_death(&self) {
        let t = self.t0();
        self.tone(420.0, 40.0, t, 0.65, 0.32, OscillatorType::Sawtooth);
        self.noise(t, 0.60, 0.18, BiquadFilterType::Lowpass, 1800.0, 120.0);
    }

    /// Short triumphant arp — SECTOR PURGED.
    pub fn play_level_clear(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::LevelClear) {
            return;
        }
        self.synth_level_clear();
    }

    /// Live synthesis of [`Self::play_level_clear`] (also pre-rendered).
    fn synth_level_clear(&self) {
        let t = self.t0();
        let notes = [523.25, 659.25, 783.99, 1046.50];
        for (i, f) in notes.iter().enumerate() {
            let at = t + i as f64 * 0.09;
            self.tone(*f, *f, at, 0.14, 0.20, OscillatorType::Square);
        }
    }

    /// Nasty shattering noise burst — a boss's mask breaks. A special hit.
    pub fn play_mask_crack(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::MaskCrack) {
            return;
        }
        self.synth_mask_crack();
    }

    /// Live synthesis of [`Self::play_mask_crack`] (also pre-rendered).
    fn synth_mask_crack(&self) {
        let t = self.t0();
        self.noise(t, 0.25, 0.40, BiquadFilterType::Highpass, 6000.0, 800.0);
        self.tone(300.0, 90.0, t, 0.18, 0.22, OscillatorType::Square);
        self.tone(
            1700.0,
            400.0,
            t + 0.03,
            0.10,
            0.18,
            OscillatorType::Sawtooth,
        );
    }

    /// A rising, ominous elevator ding — the doors close and the floor drops.
    /// A swelling detuned drone climbs to a pair of bright bell dings.
    pub fn play_elevator(&self) {
        if !self.enabled.get() {
            return; // sound off: build NO nodes (the context is suspended anyway)
        }
        if self.play_baked(SfxKind::Elevator) {
            return;
        }
        self.synth_elevator();
    }

    /// Live synthesis of [`Self::play_elevator`] (also pre-rendered).
    fn synth_elevator(&self) {
        let t = self.t0();
        // Slow ominous swell rising a fifth.
        self.tone(110.0, 165.0, t, 0.95, 0.16, OscillatorType::Sawtooth);
        self.tone(110.6, 166.5, t, 0.95, 0.10, OscillatorType::Triangle);
        // Airy noise rising underneath the swell.
        self.noise(t, 0.9, 0.05, BiquadFilterType::Highpass, 400.0, 3000.0);
        // The "ding" at the top — two chiming sines a fifth apart.
        self.tone(880.0, 880.0, t + 0.72, 0.5, 0.18, OscillatorType::Sine);
        self.tone(1318.5, 1318.5, t + 0.78, 0.45, 0.11, OscillatorType::Sine);
    }

    // --- pre-rendered voices -----------------------------------------------
    //
    // Building a fresh Web Audio graph per shot (oscillators + envelopes +
    // WaveShaper + sends — ~50 nodes for a gunshot, ~250 for a burst)
    // intermittently stalls the main thread 30–100 ms on macOS Chrome. So at
    // startup each one-shot kind's voice is rendered — by the SAME synthesis
    // code, redirected into an `OfflineAudioContext` — into SFX_VARIANTS dry
    // mono `AudioBuffer`s (background, one render in flight at a time, driven
    // from `update`). Once a kind's variants are all in, its `play_*` becomes
    // ONE `AudioBufferSourceNode` + the 1–2 gain nodes of its bus routing:
    // the room reverb, compressor and bus soft-clip stay live and identical
    // because only the pre-send dry signal is baked. Per-play variety: a
    // random variant + `playback_rate` jitter matching the live pitch jitter.

    /// Play `kind` from its pre-rendered buffers. `false` = not ready yet
    /// (or no context): the caller falls back to live synthesis.
    fn play_baked(&self, kind: SfxKind) -> bool {
        let bufs = self.baked.bufs.borrow();
        let set = &bufs[kind as usize];
        if set.len() < SFX_VARIANTS {
            return false;
        }
        let ctx = match &self.ctx {
            Some(c) => c,
            None => return false,
        };
        let spec = kind.spec();
        let out = match spec.route {
            // Same dry input + wet send the live voice uses; drive 1.0 — the
            // per-voice soft-clip is already baked into the buffer.
            SfxRoute::Real(wet) => self.voice_route(wet, 1.0, true),
            SfxRoute::Melee(wet) => self.voice_route(wet, 1.0, false),
            SfxRoute::Room => self.sfx_out(),
        };
        let out = match out {
            Some(o) => o,
            None => return false,
        };
        let src = match ctx.create_buffer_source() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let variant = ((self.rand() * SFX_VARIANTS as f64) as usize).min(SFX_VARIANTS - 1);
        src.set_buffer(Some(&set[variant]));
        if spec.rate_jitter > 0.0 {
            let _ = src
                .playback_rate()
                .set_value_at_time(self.jit(spec.rate_jitter) as f32, 0.0);
        }
        let _ = src.connect_with_audio_node(&out);
        // The buffer carries the live path's SFX_LEAD of silence at its
        // head, so "as soon as possible" keeps the same transient safety.
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        let _ = sched.start();
        true
    }

    /// Run `kind`'s live synthesis builder (used both by the `play_*`
    /// fallbacks — indirectly — and by the offline pre-render).
    fn synth(&self, kind: SfxKind) {
        match kind {
            SfxKind::AttackGun => self.synth_attack_gun(),
            SfxKind::AttackMachinegun => self.synth_attack_machinegun(),
            SfxKind::AttackShotgun => self.synth_attack_shotgun(),
            SfxKind::AttackClub => self.synth_attack_club(),
            SfxKind::HitGun => self.synth_hit_gun(),
            SfxKind::HitMachinegun => self.synth_hit_machinegun(),
            SfxKind::HitShotgun => self.synth_hit_shotgun(),
            SfxKind::HitClub => self.synth_hit_club(),
            SfxKind::EnemyDown => self.synth_enemy_down(),
            SfxKind::PlayerHurt => self.synth_player_hurt(),
            SfxKind::Pickup => self.synth_pickup(),
            SfxKind::Throw => self.synth_throw(),
            SfxKind::Death => self.synth_death(),
            SfxKind::LevelClear => self.synth_level_clear(),
            SfxKind::MaskCrack => self.synth_mask_crack(),
            SfxKind::Elevator => self.synth_elevator(),
        }
    }

    /// Advance the background pre-render: kick at most one offline render
    /// per call (per frame). Called from [`Self::update`]. Priority order:
    /// the combat one-shots first (the sounds a first firefight needs —
    /// the first [`SFX_COMBAT_KINDS`] of [`SFX_KINDS`]), then the current
    /// song's music note voices, then the rare one-shots (death,
    /// level-clear, mask-crack, elevator). If `OfflineAudioContext` is
    /// unavailable the whole queue is abandoned and everything stays on
    /// live synthesis.
    fn pump_prerender(&self) {
        if self.ctx.is_none()
            || self.renders_in_flight.get() >= self.pump_budget.get().max(1)
            || self.render_dead.get()
        {
            return;
        }
        let combat = SFX_COMBAT_KINDS.min(SFX_KINDS.len()) * SFX_VARIANTS;
        let total = SFX_KINDS.len() * SFX_VARIANTS;
        let i = self.baked.next.get();
        if i < combat {
            self.kick_sfx_render(i);
            return;
        }
        if self.pump_music_bake() {
            return; // a music voice render was kicked this frame
        }
        if i < total {
            self.kick_sfx_render(i);
        }
    }

    /// Kick the offline render of SFX queue entry `i` (one variant of one
    /// kind), advancing the queue on success and abandoning all baking on
    /// failure (graceful: live synthesis forever).
    fn kick_sfx_render(&self, i: usize) {
        if self.render_variant(SFX_KINDS[i / SFX_VARIANTS]) {
            self.baked.next.set(i + 1);
        } else {
            self.render_dead.set(true);
        }
    }

    /// Kick the next unbaked music voice render, if any. `true` = one was
    /// kicked (or baking just died) — the caller should not also kick an
    /// SFX render this frame; `false` = every music slot is baked.
    fn pump_music_bake(&self) -> bool {
        let m = &self.baked_music;
        let len = m.slots.borrow().len();
        let mut i = m.next.get();
        while i < len && m.slots.borrow()[i].buf.is_some() {
            i += 1;
        }
        m.next.set(i);
        if i >= len {
            return false;
        }
        if self.render_music_slot(i) {
            m.next.set(i + 1);
        } else {
            self.render_dead.set(true);
        }
        true
    }

    /// Build one offline variant of `kind`: redirect the voice builders into
    /// a fresh mono `OfflineAudioContext` (same sample rate as the live one,
    /// [`SfxSpec::len`] seconds), run the kind's live synthesis code
    /// unchanged, then start the async render; its completion callback
    /// stores the `AudioBuffer` and releases [`BakedSfx::busy`]. Returns
    /// `false` if the offline context can't even be created.
    fn render_variant(&self, kind: SfxKind) -> bool {
        let live = match &self.ctx {
            Some(c) => c,
            None => return false,
        };
        let sr = live.sample_rate();
        let spec = kind.spec();
        let frames = ((sr as f64) * spec.len).ceil().max(1.0) as u32;
        let off = match OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
            1, frames, sr,
        ) {
            Ok(o) => o,
            Err(_) => return false,
        };
        let sink = AsRef::<web_sys::AudioNode>::as_ref(&off.destination()).clone();
        *self.render.borrow_mut() = Some(OfflineRender {
            ctx: AsRef::<BaseAudioContext>::as_ref(&off).clone(),
            sink,
        });
        self.synth(kind);
        *self.render.borrow_mut() = None;
        let promise = match off.start_rendering() {
            Ok(p) => p,
            Err(_) => return false,
        };
        self.renders_in_flight.set(self.renders_in_flight.get() + 1);
        let store = Rc::clone(&self.baked);
        let inflight = Rc::clone(&self.renders_in_flight);
        let kidx = kind as usize;
        let done = Closure::once(move |v: JsValue| {
            if let Ok(buf) = v.dyn_into::<AudioBuffer>() {
                store.bufs.borrow_mut()[kidx].push(buf);
            }
            inflight.set(inflight.get().saturating_sub(1));
        });
        let inflight = Rc::clone(&self.renders_in_flight);
        // A rejected render skips this variant: the kind never completes its
        // set and permanently keeps the live path (graceful).
        let fail = Closure::once(move |_e: JsValue| inflight.set(inflight.get().saturating_sub(1)));
        let _ = promise.then2(&done, &fail);
        // Keep the pair alive until the next kick (they will have fired by
        // then — renders are strictly sequential).
        // Concurrent renders may be pending: never clear here — the pile is
        // pruned from update() once nothing is in flight.
        let mut pending = self.baked.pending.borrow_mut();
        pending.push(done);
        pending.push(fail);
        true
    }

    /// Build the offline render of music voice slot `i`: same recipe as
    /// [`Self::render_variant`] — redirect the note builders into a fresh
    /// mono `OfflineAudioContext` (live sample rate, [`Self::music_key_len`]
    /// seconds), run the note's LIVE synthesis code unchanged at t = 0, then
    /// start the async render; its completion callback stores the
    /// `AudioBuffer` into the slot (unless the song changed meanwhile — the
    /// [`BakedMusic::gen`] guard) and decrements [`Self::renders_in_flight`].
    fn render_music_slot(&self, i: usize) -> bool {
        let live = match &self.ctx {
            Some(c) => c,
            None => return false,
        };
        let key = match self.baked_music.slots.borrow().get(i) {
            Some(slot) => slot.key,
            None => return false,
        };
        let sr = live.sample_rate();
        let frames = ((sr as f64) * self.music_key_len(key)).ceil().max(1.0) as u32;
        let off = match OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
            1, frames, sr,
        ) {
            Ok(o) => o,
            Err(_) => return false,
        };
        let sink = AsRef::<web_sys::AudioNode>::as_ref(&off.destination()).clone();
        *self.render.borrow_mut() = Some(OfflineRender {
            ctx: AsRef::<BaseAudioContext>::as_ref(&off).clone(),
            sink,
        });
        self.synth_music_note(key, 0.0);
        *self.render.borrow_mut() = None;
        let promise = match off.start_rendering() {
            Ok(p) => p,
            Err(_) => return false,
        };
        self.renders_in_flight.set(self.renders_in_flight.get() + 1);
        let store = Rc::clone(&self.baked_music);
        let inflight = Rc::clone(&self.renders_in_flight);
        let gen = self.baked_music.gen.get();
        let done = Closure::once(move |v: JsValue| {
            if store.gen.get() == gen {
                if let Ok(buf) = v.dyn_into::<AudioBuffer>() {
                    if let Some(slot) = store.slots.borrow_mut().get_mut(i) {
                        slot.buf = Some(buf);
                    }
                }
            }
            inflight.set(inflight.get().saturating_sub(1));
        });
        let inflight = Rc::clone(&self.renders_in_flight);
        // A rejected render leaves the slot unbaked forever: that one voice
        // permanently keeps the live per-note path (graceful).
        let fail = Closure::once(move |_e: JsValue| inflight.set(inflight.get().saturating_sub(1)));
        let _ = promise.then2(&done, &fail);
        // Concurrent renders may be pending: never clear here — the pile is
        // pruned from update() once nothing is in flight.
        let mut pending = self.baked_music.pending.borrow_mut();
        pending.push(done);
        pending.push(fail);
        true
    }

    // --- SFX building blocks -----------------------------------------------
    //
    // Small, physically-motivated layers. Each takes the voice node to render
    // into and an absolute start time; the `play_*` methods stack them.

    /// A realistic ricochet: mostly NOISE through a moving high-Q bandpass
    /// whining down 3 kHz → 600 Hz over 250–400 ms, plus a faint pitched
    /// sweep under it. Quiet.
    fn real_ricochet(&self, out: &web_sys::AudioNode, t: f64, j: f64, peak: f64) {
        let dur = 0.25 + self.rand() * 0.15;
        let f0 = 3000.0 * j * self.jit(0.08);
        let f1 = 600.0 * j * self.jit(0.1);
        self.noise_full(
            out,
            t,
            0.012,
            dur,
            dur * 0.9,
            peak * 3.0,
            BiquadFilterType::Bandpass,
            f0,
            f1,
            14.0,
        );
        self.tone_out(
            out,
            f0 * 1.01,
            f1 * 1.01,
            t + 0.005,
            dur * 0.9,
            peak * 0.18,
            0.012,
            OscillatorType::Sine,
        );
    }

    /// Render one gunshot from a [`RealShot`] recipe at `t`:
    /// crack (bright cluster + 3–8 kHz band + air, rising over `crack_rise`
    /// so the peak lands at +5–15 ms, uncompressed) → mid body plateau
    /// (bandpass ~1.2 kHz, Q 0.5, held ~100 ms at −5 dB, −21 dB by +200 ms)
    /// with hi / air companions → low-mid layer → faint thump. No sub, no
    /// growl, no AM; ±3 ms onset jitter per layer; the room comes from the
    /// bright gun-bus reverb via the voice's wet send.
    fn real_shot(&self, out: &web_sys::AudioNode, t: f64, j: f64, level: f64, s: &RealShot) {
        // Crack.
        let bus = self.crack_bus(out);
        let n = 3 + (self.rand() * 3.0) as usize;
        for k in 0..n {
            let at = if k == 0 { t } else { t + self.rand() * 0.006 };
            self.click(&bus, at, s.crack * level * (0.6 + self.rand() * 0.6));
        }
        let rise = s.crack_rise * self.jit(0.3);
        self.noise_full(
            &bus,
            t,
            rise,
            rise + 0.014,
            rise + 0.014,
            1.8 * s.crack * level,
            BiquadFilterType::Bandpass,
            5500.0 * j,
            3200.0 * j,
            0.5,
        );
        self.noise_full(
            &bus,
            self.jt(t + 0.001, 0.001),
            rise,
            rise + 0.010,
            rise + 0.010,
            1.4 * s.crack * s.air * level,
            BiquadFilterType::Highpass,
            8000.0 * j,
            8000.0 * j,
            0.7,
        );
        self.noise_full(
            &bus,
            self.jt(t + 0.001, 0.001),
            rise,
            rise + 0.022,
            rise + 0.022,
            1.6 * s.crack * level,
            BiquadFilterType::Lowpass,
            12000.0 * j,
            1500.0 * j,
            0.7,
        );
        // Mid body plateau + hi / air companions.
        let b = self.jt(t + 0.002, 0.003);
        self.noise_plateau(
            out,
            b,
            s.body * level,
            s.plateau,
            s.drop,
            BiquadFilterType::Bandpass,
            s.body_hz * j,
            s.body_hz * 0.8 * j,
            0.5,
        );
        self.noise_plateau(
            out,
            self.jt(b, 0.003),
            s.body_hi * level,
            s.plateau * 0.9,
            s.drop,
            BiquadFilterType::Bandpass,
            4200.0 * j,
            3000.0 * j,
            0.6,
        );
        self.noise_plateau(
            out,
            self.jt(b, 0.003),
            s.body_air * level,
            s.plateau * 0.8,
            s.drop,
            BiquadFilterType::Highpass,
            8000.0,
            8000.0,
            0.7,
        );
        // Low-mid layer (130–300 Hz): bandpass noise (Q 0.8, centred on
        // `low_hz`, sagging to ~150 Hz) with the plateau shape, plus a pair
        // of decaying pitched partials (~150 and ~220 Hz, triangle, ~120 ms).
        // The gun-bus reverb low-cut sits at 120 Hz so this reaches the room.
        self.noise_plateau(
            out,
            self.jt(b + 0.002, 0.003),
            s.low * level,
            s.plateau * 1.1,
            s.drop * 1.2,
            BiquadFilterType::Bandpass,
            s.low_hz * 0.95 * j,
            s.low_hz * 0.7 * j,
            0.8,
        );
        for f in [150.0, 220.0] {
            let f = f * j * self.jit(0.02);
            self.swell_tone(
                out,
                f,
                f * 0.97,
                self.jt(b + 0.003, 0.003),
                0.004,
                0.04,
                s.low * 0.22 * level,
                OscillatorType::Triangle,
                0.015,
            );
        }
        // Faint thump (≤ −15 dB): a short, wobbled 60–90 Hz component.
        if s.thump > 0.0 {
            self.swell_tone(
                out,
                75.0 * j * self.jit(0.08),
                70.0 * j,
                self.jt(t + 0.004, 0.003),
                0.006,
                0.05,
                s.thump * level,
                OscillatorType::Sine,
                0.03,
            );
        }
    }

    /// A noise layer with the measured real-gunshot body envelope: 3 ms rise
    /// to `peak`, a plateau that only sags 5 dB over `plateau` seconds, a
    /// drop to −21 dB over the next `drop` seconds, then a slide to silence
    /// (−80 dB) over a further 3× `drop`; filter sweeps `f0 → f1` across
    /// the plateau + drop.
    #[allow(clippy::too_many_arguments)]
    fn noise_plateau(
        &self,
        out: &web_sys::AudioNode,
        start: f64,
        peak: f64,
        plateau: f64,
        drop: f64,
        filter: BiquadFilterType,
        f0: f64,
        f1: f64,
        q: f64,
    ) {
        let (ctx, buf) = match (self.bctx(), &self.noise) {
            (Some(c), Some(b)) => (c, b),
            _ => return,
        };
        let (src, filt, gain) = match (
            ctx.create_buffer_source(),
            ctx.create_biquad_filter(),
            ctx.create_gain(),
        ) {
            (Ok(s), Ok(f), Ok(g)) => (s, f, g),
            _ => return,
        };
        let peak = peak.max(0.0002) as f32;
        let t1 = start + 0.003;
        let t2 = t1 + plateau.max(0.005);
        let t3 = t2 + drop.max(0.005);
        let t4 = t3 + 3.0 * drop.max(0.005);
        src.set_buffer(Some(buf));
        src.set_loop(true);
        filt.set_type(filter);
        let ff = filt.frequency();
        let _ = ff.set_value_at_time(f0 as f32, start);
        if (f1 - f0).abs() > 1.0 {
            let _ = ff.exponential_ramp_to_value_at_time(f1.max(1.0) as f32, t3);
        }
        let _ = filt.q().set_value_at_time(q as f32, start);
        let g = gain.gain();
        let _ = g.set_value_at_time(0.0001, start);
        let _ = g.linear_ramp_to_value_at_time(peak, t1);
        let _ = g.exponential_ramp_to_value_at_time(peak * 0.56, t2); // −5 dB
        let _ = g.exponential_ramp_to_value_at_time(peak * 0.089, t3); // −21 dB
        let _ = g.exponential_ramp_to_value_at_time(0.0001, t4);
        let _ = src.connect_with_audio_node(&filt);
        let _ = filt.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(out);
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        let offset = self.rand() * (NOISE_SECONDS - 0.05);
        let _ = src.start_with_when_and_grain_offset(start, offset);
        let _ = sched.stop_with_when(t4 + 0.02);
    }

    /// A real pump reload starting at `t`: ~1 s of multiple bright 2–8 kHz
    /// clacks and slide scrapes — pump back (two clacks), forward (two),
    /// the shell / lifter — with essentially no low content.
    fn real_pump(&self, out: &web_sys::AudioNode, t: f64, j: f64) {
        // (offset, scrape level, clack base Hz, clack level) — kept ~4 dB
        // under the shot's tail so the rack sits inside it, like a real one.
        let events: [(f64, f64, f64, f64); 6] = [
            (0.0, 0.22, 3300.0, 0.21),
            (0.045, 0.0, 2500.0, 0.16),
            (0.28, 0.20, 2900.0, 0.20),
            (0.32, 0.0, 2100.0, 0.15),
            (0.55, 0.09, 3800.0, 0.11),
            (0.72, 0.0, 2700.0, 0.09),
        ];
        for (off, scrape, base, clack) in events {
            let at = t + off + self.rand() * 0.02;
            if scrape > 0.0 {
                self.noise_full(
                    out,
                    at,
                    0.012,
                    0.11,
                    0.11,
                    scrape,
                    BiquadFilterType::Bandpass,
                    4200.0 * j,
                    3000.0 * j,
                    0.6,
                );
            }
            self.tick(out, at + 0.008, base * j * self.jit(0.05), clack);
            self.noise_full(
                out,
                at + 0.008,
                0.0,
                0.03,
                0.03,
                clack * 0.9,
                BiquadFilterType::Highpass,
                3000.0,
                3000.0,
                0.7,
            );
        }
        self.tinkle(out, t + 0.40 + self.rand() * 0.05, 0.03);
    }

    /// A gain node feeding `out` directly plus, through 2–3 short DelayNodes
    /// (6–25 ms, lowpassed at ~3 kHz, decaying), the early reflections of
    /// whatever is played into it. Falls back to `out` if nodes fail.
    fn crack_bus(&self, out: &web_sys::AudioNode) -> web_sys::AudioNode {
        let ctx = match self.bctx() {
            Some(c) => c,
            None => return out.clone(),
        };
        let bus = match ctx.create_gain() {
            Ok(g) => g,
            Err(_) => return out.clone(),
        };
        let _ = bus.gain().set_value_at_time(1.0, 0.0);
        let _ = bus.connect_with_audio_node(out);
        let taps = [
            (0.006 + self.rand() * 0.004, 0.5),
            (0.013 + self.rand() * 0.006, 0.35),
            (0.020 + self.rand() * 0.006, 0.22),
        ];
        let n_taps = 2 + (self.rand() * 2.0) as usize;
        for (d, g) in taps.iter().take(n_taps) {
            if let (Ok(delay), Ok(lpf), Ok(gain)) = (
                ctx.create_delay_with_max_delay_time(0.05),
                ctx.create_biquad_filter(),
                ctx.create_gain(),
            ) {
                let _ = delay.delay_time().set_value_at_time(*d as f32, 0.0);
                lpf.set_type(BiquadFilterType::Lowpass);
                let _ = lpf.frequency().set_value_at_time(3000.0, 0.0);
                let _ = gain.gain().set_value_at_time(*g as f32, 0.0);
                let _ = bus.connect_with_audio_node(&delay);
                let _ = delay.connect_with_audio_node(&lpf);
                let _ = lpf.connect_with_audio_node(&gain);
                let _ = gain.connect_with_audio_node(out);
            }
        }
        AsRef::<web_sys::AudioNode>::as_ref(&bus).clone()
    }

    /// `t` jittered by ±`a` seconds (layer de-synchronisation).
    fn jt(&self, t: f64, a: f64) -> f64 {
        t + (self.rand() * 2.0 - 1.0) * a
    }

    /// A noise-derived LFO added to `param` for `dur` seconds from `start`:
    /// the noise buffer through a bandpass centred `fc` (Q `q`), scaled so
    /// the modulation has an RMS of `amount` (in the param's units). Used for
    /// envelope roughness (40–90 Hz on a gain) and pitch random-walks (10–30
    /// Hz on a frequency).
    #[allow(clippy::too_many_arguments)]
    fn noise_lfo(
        &self,
        param: &web_sys::AudioParam,
        fc: f64,
        q: f64,
        amount: f64,
        start: f64,
        dur: f64,
        clamp: Option<f64>,
    ) {
        let (ctx, buf) = match (self.bctx(), &self.noise) {
            (Some(c), Some(b)) => (c, b),
            _ => return,
        };
        let (src, filt, gain) = match (
            ctx.create_buffer_source(),
            ctx.create_biquad_filter(),
            ctx.create_gain(),
        ) {
            (Ok(s), Ok(f), Ok(g)) => (s, f, g),
            _ => return,
        };
        // Uniform ±1 noise has RMS 0.577; the bandpass keeps a fraction
        // fc/q of the 0..sr/2 band, so its output RMS is 0.577·sqrt(...).
        let sr = ctx.sample_rate() as f64;
        let rms = 0.577 * (fc / q / (sr * 0.5)).sqrt();
        src.set_buffer(Some(buf));
        src.set_loop(true);
        filt.set_type(BiquadFilterType::Bandpass);
        let _ = filt.frequency().set_value_at_time(fc as f32, start);
        let _ = filt.q().set_value_at_time(q as f32, start);
        let _ = gain
            .gain()
            .set_value_at_time((amount / rms.max(1e-4)) as f32, start);
        let _ = src.connect_with_audio_node(&filt);
        let _ = filt.connect_with_audio_node(&gain);
        // Optional hard clamp of the modulation to ±`clamp` (in the param's
        // units, |clamp| ≤ 1) through a WaveShaper, so an AM peak can never
        // push a gain past 1 + clamp.
        let mut tail: web_sys::AudioNode = AsRef::<web_sys::AudioNode>::as_ref(&gain).clone();
        if let Some(c) = clamp {
            if let Ok(shaper) = ctx.create_wave_shaper() {
                let c = c.clamp(0.01, 1.0) as f32;
                let n = 1024usize;
                let mut curve: Vec<f32> = (0..n)
                    .map(|i| (i as f32 / (n - 1) as f32 * 2.0 - 1.0).clamp(-c, c))
                    .collect();
                shaper.set_curve_opt_f32_slice(Some(curve.as_mut_slice()));
                if tail.connect_with_audio_node(&shaper).is_ok() {
                    tail = AsRef::<web_sys::AudioNode>::as_ref(&shaper).clone();
                }
            }
        }
        let _ = tail.connect_with_audio_param(param);
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        let offset = self.rand() * (NOISE_SECONDS - 0.05);
        let _ = src.start_with_when_and_grain_offset(start, offset);
        let _ = sched.stop_with_when(start + dur + 0.02);
    }

    /// Roughness stage: returns a unity gain node into `out` whose gain is
    /// modulated by a 40–90 Hz (random centre, Q 0.7) noise LFO with an RMS
    /// of 0.45·`depth`, so a layer rendered into it comes out as
    /// `x(t)·(1 + m(t))` — its envelope growls / rattles at 20–150 Hz
    /// instead of decaying smoothly; the modulation is clamped to ±0.5 so
    /// the stage's gain never exceeds 1.5. `depth` ≤ 0 returns `out` itself.
    fn roughen(
        &self,
        out: &web_sys::AudioNode,
        start: f64,
        dur: f64,
        depth: f64,
    ) -> web_sys::AudioNode {
        if depth <= 0.0 {
            return out.clone();
        }
        let ctx = match self.bctx() {
            Some(c) => c,
            None => return out.clone(),
        };
        let stage = match ctx.create_gain() {
            Ok(g) => g,
            Err(_) => return out.clone(),
        };
        let _ = stage.gain().set_value_at_time(1.0, 0.0);
        if stage.connect_with_audio_node(out).is_err() {
            return out.clone();
        }
        let fc = 40.0 + self.rand() * 50.0;
        self.noise_lfo(&stage.gain(), fc, 0.7, 0.45 * depth, start, dur, Some(0.5));
        AsRef::<web_sys::AudioNode>::as_ref(&stage).clone()
    }

    /// FM growl: a low sine (`f`, 55–70 Hz) whose frequency is modulated by
    /// a second oscillator at `rate` (30–60 Hz) with `depth_hz` (40–120 Hz)
    /// of swing — sidebands at ±rate make a snarling roar once saturated —
    /// swelling over `attack` and falling 20 dB over `d20`.
    #[allow(clippy::too_many_arguments)]
    fn growl(
        &self,
        out: &web_sys::AudioNode,
        t: f64,
        f: f64,
        rate: f64,
        depth_hz: f64,
        attack: f64,
        d20: f64,
        peak: f64,
    ) {
        let ctx = match self.bctx() {
            Some(c) => c,
            None => return,
        };
        let (car, modo, mg, gain) = match (
            ctx.create_oscillator(),
            ctx.create_oscillator(),
            ctx.create_gain(),
            ctx.create_gain(),
        ) {
            (Ok(a), Ok(b), Ok(c), Ok(d)) => (a, b, c, d),
            _ => return,
        };
        let attack = attack.max(0.001);
        let end = t + attack + 4.0 * d20.max(0.01);
        car.set_type(OscillatorType::Sine);
        let _ = car.frequency().set_value_at_time(f as f32, t);
        modo.set_type(OscillatorType::Sine);
        let _ = modo.frequency().set_value_at_time(rate as f32, t);
        let _ = mg.gain().set_value_at_time(depth_hz as f32, t);
        let _ = modo.connect_with_audio_node(&mg);
        let _ = mg.connect_with_audio_param(&car.frequency());
        let g = gain.gain();
        let _ = g.set_value_at_time(0.0001, t);
        let _ = g.linear_ramp_to_value_at_time(peak.max(0.0002) as f32, t + attack);
        let _ = g.exponential_ramp_to_value_at_time(0.0001, end);
        let _ = car.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(out);
        for o in [&car, &modo] {
            let sched: &web_sys::AudioScheduledSourceNode = o.as_ref();
            let _ = sched.start_with_when(t);
            let _ = sched.stop_with_when(end + 0.02);
        }
    }

    /// A short, heavy body impact without a clean glide: three close low
    /// components (≈ 61 / 88 / 126 Hz) roughened, plus an FM growl and a
    /// low-passed thump — the "wham" of a big object being hit.
    fn wham(&self, out: &web_sys::AudioNode, t: f64, peak: f64) {
        const C: [(f64, f64); 3] = [(88.0, 1.0), (126.0, 0.5), (61.0, 0.7)];
        for (f, lvl) in C {
            let f = f * self.jit(0.05);
            let at = self.jt(t, 0.003);
            let o = self.roughen(out, at, 0.3, 0.8);
            self.swell_tone(
                &o,
                f,
                f * 0.96,
                at,
                0.004,
                0.06,
                peak * lvl,
                OscillatorType::Sine,
                0.04,
            );
        }
        let o = self.roughen(out, t, 0.3, 0.6);
        self.growl(
            &o,
            self.jt(t, 0.003),
            56.0 * self.jit(0.06),
            35.0 + self.rand() * 25.0,
            60.0 + self.rand() * 60.0,
            0.01,
            0.05,
            peak * 0.7,
        );
        self.noise_env(
            &o,
            t,
            0.0,
            0.10,
            peak * 0.9,
            BiquadFilterType::Lowpass,
            220.0,
            70.0,
            1.2,
        );
    }

    /// A sub-millisecond broadband click — the pressure step that fronts every
    /// shot and impact. Without it nothing sounds "hit".
    fn click(&self, out: &web_sys::AudioNode, t: f64, peak: f64) {
        self.noise_env(
            out,
            t,
            0.0,
            0.0015,
            peak,
            BiquadFilterType::Highpass,
            700.0,
            700.0,
            0.7,
        );
    }

    /// A short mechanical tick — a slide, bolt, pump or a loose part: a
    /// tiny click plus three fast-decaying inharmonic partials.
    fn tick(&self, out: &web_sys::AudioNode, t: f64, base: f64, peak: f64) {
        const R: [f64; 3] = [1.0, 1.83, 2.94];
        self.noise_env(
            out,
            t,
            0.0,
            0.004,
            peak * 0.9,
            BiquadFilterType::Bandpass,
            base * 2.0,
            base * 2.0,
            0.6,
        );
        for (i, r) in R.iter().enumerate() {
            let f = base * r;
            let g = peak * 0.7 / (1.0 + i as f64 * 0.8);
            let d = 0.032 - 0.007 * i as f64;
            self.tone_out(out, f, f * 0.98, t, d, g, 0.001, OscillatorType::Sine);
        }
    }

    /// A brass casing hitting the floor: two bright, tiny pings that bounce
    /// twice with decreasing height.
    fn tinkle(&self, out: &web_sys::AudioNode, t: f64, peak: f64) {
        const R: [f64; 3] = [1.0, 1.42, 2.31];
        let base = 3900.0 * self.jit(0.08);
        let mut at = t;
        let mut g = peak;
        for bounce in 0..3 {
            for (i, r) in R.iter().enumerate() {
                let f = base * r;
                self.tone_out(
                    out,
                    f,
                    f * 0.995,
                    at,
                    0.07 - 0.012 * i as f64,
                    g / (1.0 + i as f64),
                    0.0005,
                    OscillatorType::Sine,
                );
            }
            at += 0.075 + self.rand() * 0.03 - bounce as f64 * 0.015;
            g *= 0.55;
        }
    }

    /// A short vocal grunt: a rough sawtooth (with a detuned partner for
    /// hoarseness) whose pitch sags, through a swept, resonant bandpass so it
    /// reads as an "uh!" rather than a buzz.
    fn grunt(&self, out: &web_sys::AudioNode, t: f64, f: f64, dur: f64, peak: f64) {
        let ctx = match self.bctx() {
            Some(c) => c,
            None => return,
        };
        let filt = match ctx.create_biquad_filter() {
            Ok(f) => f,
            Err(_) => return,
        };
        filt.set_type(BiquadFilterType::Bandpass);
        let ff = filt.frequency();
        let _ = ff.set_value_at_time(760.0, t);
        let _ = ff.exponential_ramp_to_value_at_time(420.0, t + dur);
        let _ = filt.q().set_value_at_time(2.5, t);
        let _ = filt.connect_with_audio_node(out);
        let fo: &web_sys::AudioNode = filt.as_ref();
        self.tone_out(
            fo,
            f,
            f * 0.72,
            t,
            dur,
            peak,
            0.012,
            OscillatorType::Sawtooth,
        );
        self.tone_out(
            fo,
            f * 1.012,
            f * 0.72 * 1.012,
            t,
            dur,
            peak * 0.6,
            0.012,
            OscillatorType::Sawtooth,
        );
    }

    // --- music -------------------------------------------------------------

    /// Begin the looping backing track using the current song (idempotent).
    pub fn start_music(&mut self) {
        if !self.enabled.get() {
            return; // sound off: don't schedule anything
        }
        if self.music_playing {
            return;
        }
        self.music_playing = true;
        self.section = 0;
        self.step = 0;
        self.next_note_time = self.now() + 0.1;
    }

    /// Stop the loop. Already-queued notes ring out; no new ones are scheduled.
    pub fn stop_music(&mut self) {
        self.music_playing = false;
    }

    /// Swap the active song. Takes effect from the next scheduled step, so a
    /// switch while playing is seamless (no gap, no restart of the audio clock).
    /// The arrangement restarts from its first section. A NEW song's note
    /// voices start baking in the background ([`BakedMusic`]); until each
    /// lands its notes fall back to live oscillator synthesis per step.
    pub fn set_song(&mut self, spec: SongSpec) {
        let changed = self.song.name != spec.name;
        self.song = spec;
        self.section = 0;
        self.step = 0;
        if changed {
            self.rebuild_music_bake();
        }
    }

    /// Select a song by index into [`SONGS`] (clamped) and start playing it.
    /// This is the primary entry point for the integrator: `play_song(floor)`
    /// via [`song_for_floor`], or a direct index from the `?viz` tracker.
    pub fn play_song(&mut self, index: usize) {
        let idx = index.min(SONGS.len().saturating_sub(1));
        self.set_song(SONGS[idx]);
        self.start_music();
    }

    // --- tracker API -------------------------------------------------------
    //
    // Read/seek hooks for the `?viz` MUSICS tracker view. All indices are
    // channels 0..NUM_CHANNELS (see CHANNEL_NAMES): 0 bass, 1 lead, 2 pad,
    // 3 arp, 4 drums.
    //
    // IMPORTANT: every read here reflects the *currently-playing section*, so
    // the tracker grid + playhead always mirror what is actually sounding as
    // the arrangement moves from section to section.

    /// The song currently loaded into the scheduler (copyable data).
    pub fn current_song(&self) -> SongSpec {
        self.song
    }

    /// Whether the music scheduler is currently running.
    pub fn is_playing(&self) -> bool {
        self.music_playing
    }

    /// Index of the section currently playing within the song's arrangement.
    pub fn current_section(&self) -> usize {
        self.section
    }

    /// Human-readable label of the currently-playing section (e.g. "refrain").
    pub fn current_section_label(&self) -> &'static str {
        self.section_ref().map(|s| s.label).unwrap_or("")
    }

    /// How many sections the current song's arrangement contains.
    pub fn section_count(&self) -> usize {
        self.song.sections.len()
    }

    /// Number of steps in the currently-playing section's pattern (its longest
    /// lane). This is the width of the live tracker grid.
    pub fn pattern_len(&self) -> usize {
        self.loop_len()
    }

    /// The step currently *sounding* within the current section (accounts for
    /// the scheduler look-ahead), for drawing the moving playhead. `0` when
    /// stopped.
    pub fn current_step(&self) -> usize {
        let loop_len = self.loop_len();
        if !self.music_playing || loop_len == 0 {
            return 0;
        }
        let step_dur = self.step_dur();
        let ahead = ((self.next_note_time - self.now()) / step_dur).ceil();
        let ahead = if ahead.is_finite() && ahead > 0.0 {
            ahead as usize % loop_len
        } else {
            0
        };
        (self.step + loop_len - ahead) % loop_len
    }

    /// Does `channel` have a note/hit at `step` in the current section? Drives
    /// the tracker grid cells.
    pub fn channel_active(&self, channel: usize, step: usize) -> bool {
        match self.section_ref() {
            Some(sec) => Self::cell_active(sec, channel, step),
            None => false,
        }
    }

    // --- section mini-map API ---------------------------------------------
    //
    // For a clickable strip of section miniatures above the main grid: read any
    // section (not just the playing one) and jump the playhead between them.

    /// Human-readable label of section `i` in the arrangement (e.g. "verse"),
    /// or `""` if `i` is out of range. Use to caption each miniature.
    pub fn section_label(&self, i: usize) -> &'static str {
        self.song.sections.get(i).map(|s| s.label).unwrap_or("")
    }

    /// Number of steps in section `i` (its longest lane), or `0` if out of
    /// range. Lets a miniature size its own little grid.
    pub fn section_pattern_len(&self, i: usize) -> usize {
        self.song.sections.get(i).map(section_len).unwrap_or(0)
    }

    /// Sample any section's grid: does `channel` have a note/hit at `step` in
    /// section `section`? A per-section previewer for drawing the miniatures
    /// (the `section == current_section()` one mirrors [`Self::channel_active`]).
    pub fn section_cell(&self, section: usize, channel: usize, step: usize) -> bool {
        match self.song.sections.get(section) {
            Some(sec) => Self::cell_active(sec, channel, step),
            None => false,
        }
    }

    /// Compact density summary of section `i`: the fraction (0.0..=1.0) of all
    /// grid cells that carry a note/hit. A cheap way to shade each miniature by
    /// how busy/intense it is without drawing every cell.
    pub fn section_density(&self, i: usize) -> f32 {
        let sec = match self.song.sections.get(i) {
            Some(s) => s,
            None => return 0.0,
        };
        let steps = section_len(sec);
        if steps == 0 {
            return 0.0;
        }
        let mut active = 0usize;
        for step in 0..steps {
            for chan in 0..NUM_CHANNELS {
                if Self::cell_active(sec, chan, step) {
                    active += 1;
                }
            }
        }
        active as f32 / (steps * NUM_CHANNELS) as f32
    }

    /// Shared cell sampler: does `channel` fire at `step` within `sec`?
    fn cell_active(sec: &Section, channel: usize, step: usize) -> bool {
        match channel {
            0 => degree_at(sec.bass, step).is_some(),
            1 => degree_at(sec.lead, step).is_some(),
            2 => degree_at(sec.pad, step).is_some(),
            3 => degree_at(sec.arp, step).is_some(),
            4 => !matches!(drum_at(sec.drums, step), Silent),
            _ => false,
        }
    }

    /// Jump the playhead to `step` within the current section (wrapped). Music
    /// keeps playing from there on the next scheduled note; the section is not
    /// changed.
    pub fn seek(&mut self, step: usize) {
        let loop_len = self.loop_len();
        self.step = if loop_len == 0 { 0 } else { step % loop_len };
        self.next_note_time = self.now() + 0.02;
    }

    /// Jump the playhead to the start of section `i` in the arrangement,
    /// clamped into range. Music continues seamlessly from that section's first
    /// step on the next scheduled note. Drives clicking a section miniature.
    pub fn jump_to_section(&mut self, i: usize) {
        let n = self.song.sections.len();
        self.section = if n == 0 { 0 } else { i.min(n - 1) };
        self.step = 0;
        self.next_note_time = self.now() + 0.02;
    }

    /// Toggle mute for `channel` (out of range is ignored).
    pub fn toggle_mute(&mut self, channel: usize) {
        if channel < NUM_CHANNELS {
            self.mute[channel] = !self.mute[channel];
        }
    }

    /// Toggle solo for `channel` (out of range is ignored).
    pub fn toggle_solo(&mut self, channel: usize) {
        if channel < NUM_CHANNELS {
            self.solo[channel] = !self.solo[channel];
        }
    }

    /// Is `channel` muted?
    pub fn is_muted(&self, channel: usize) -> bool {
        channel < NUM_CHANNELS && self.mute[channel]
    }

    /// Is `channel` soloed?
    pub fn is_solo(&self, channel: usize) -> bool {
        channel < NUM_CHANNELS && self.solo[channel]
    }

    /// Should `channel` actually be heard right now? Muted channels are silent;
    /// if any channel is soloed, only soloed channels sound.
    fn channel_audible(&self, channel: usize) -> bool {
        if channel >= NUM_CHANNELS || self.mute[channel] {
            return false;
        }
        let any_solo = self.solo.iter().any(|&s| s);
        !any_solo || self.solo[channel]
    }

    /// The section currently playing, if the arrangement is non-empty.
    fn section_ref(&self) -> Option<&Section> {
        self.song.sections.get(self.section)
    }

    /// Length of one sequencer step (seconds) for the current song's tempo.
    fn step_dur(&self) -> f64 {
        let spb = self.song.steps_per_beat.max(1) as f64;
        60.0 / self.song.bpm.max(1.0) / spb
    }

    /// Number of steps in one bar (used to pace the per-bar filter sweep).
    /// Assumes a 4-beat bar.
    fn bar_steps(&self) -> usize {
        (self.song.steps_per_beat.max(1) as usize) * 4
    }

    /// Number of steps before the *current section* repeats.
    fn loop_len(&self) -> usize {
        self.section_ref().map(section_len).unwrap_or(1)
    }

    /// Advance to the next section of the arrangement, wrapping back to the
    /// first when the song's play-through completes.
    fn advance_section(&mut self) {
        let n = self.song.sections.len();
        if n == 0 {
            self.section = 0;
        } else {
            self.section = (self.section + 1) % n;
        }
    }

    /// Drive the look-ahead scheduler. Call every frame; `now_seconds` is
    /// unused (we trust the audio clock), kept for a stable game-loop signature.
    pub fn update(&mut self, _now_seconds: f64) {
        // Chip away at the pre-render queue — combat SFX, then the song's
        // music voices, then rare SFX — independent of the sound toggle, so
        // the baked voices are ready the moment sound comes (back) on. Up to
        // `pump_budget` renders run concurrently (the game loop raises the
        // budget on loading/menu screens to burn the queue down before
        // gameplay, where a construction hitch would be visible).
        if self.renders_in_flight.get() == 0 {
            self.baked.pending.borrow_mut().clear();
            self.baked_music.pending.borrow_mut().clear();
        }
        for _ in 0..self.pump_budget.get().max(1) {
            self.pump_prerender();
        }
        if !self.enabled.get() {
            return; // sound off: the scheduler idles entirely
        }
        if !self.music_playing {
            return;
        }
        let now = self.now();
        // Catch up if we fell behind (e.g. after a tab was backgrounded).
        if self.next_note_time < now {
            self.next_note_time = now + 0.05;
        }
        let step_dur = self.step_dur();
        let bar_steps = self.bar_steps();
        while self.next_note_time < now + LOOKAHEAD {
            let sec_len = self.loop_len();
            let step = self.step;
            let t = self.next_note_time;
            // At the top of each bar, arm the synthwave filter sweep for it.
            // `bar_steps` is always >= 4, so this is safe.
            if step.is_multiple_of(bar_steps) {
                self.schedule_filter_sweep(t, step_dur * bar_steps as f64);
            }
            self.schedule_step(step, t);
            self.next_note_time += step_dur;
            self.step += 1;
            if self.step >= sec_len {
                self.step = 0;
                self.advance_section();
            }
        }
    }

    /// Sweep the music-bus lowpass cutoff up and back down across one bar — the
    /// signature synthwave "filter wah". Darker songs sweep a narrower, lower
    /// band so the mix stays muffled and oppressive.
    fn schedule_filter_sweep(&self, start: f64, bar_dur: f64) {
        let filt = match &self.music_filter {
            Some(f) => f,
            None => return,
        };
        // Higher intensity => lower/tighter peak, for a darker, closed sound.
        let peak_hz = (5200.0 / self.song.intensity.max(0.4)).clamp(1400.0, 6000.0) as f32;
        let low_hz = 420.0f32;
        let f = filt.frequency();
        let _ = f.set_value_at_time(low_hz, start);
        let _ = f.exponential_ramp_to_value_at_time(peak_hz, start + bar_dur * 0.5);
        let _ = f.exponential_ramp_to_value_at_time(low_hz, start + bar_dur);
    }

    /// Schedule one step of the current section (all channels) at time `t`.
    /// Every note goes through [`Self::music_note`]: one pre-baked
    /// `AudioBufferSourceNode` when the voice's buffer is ready, the live
    /// oscillator synthesis otherwise.
    fn schedule_step(&self, step: usize, t: f64) {
        let sec = match self.section_ref() {
            Some(s) => s,
            None => return,
        };
        if self.channel_audible(0) {
            if let Some(d) = degree_at(sec.bass, step) {
                self.music_note(MusicKey::Bass(d), t);
            }
        }
        if self.channel_audible(1) {
            if let Some(d) = degree_at(sec.lead, step) {
                self.music_note(MusicKey::Lead(d), t);
            }
        }
        if self.channel_audible(2) {
            if let Some(d) = degree_at(sec.pad, step) {
                self.music_note(MusicKey::Pad(d), t);
            }
        }
        if self.channel_audible(3) {
            if let Some(d) = degree_at(sec.arp, step) {
                self.music_note(MusicKey::Arp(d), t);
            }
        }
        if self.channel_audible(4) {
            match drum_at(sec.drums, step) {
                Silent => {}
                Kick => self.music_note(MusicKey::Kick, t),
                Hat => self.music_note(MusicKey::Hat, t),
                Snare => self.music_note(MusicKey::Snare, t),
            }
        }
    }

    /// Play one music voice at absolute time `t`: the pre-baked buffer if it
    /// landed (a single source node into the live music bus — the per-bar
    /// filter sweep still shapes it downstream), else the live synthesis.
    fn music_note(&self, key: MusicKey, t: f64) {
        if self.play_music_baked(key, t) {
            return;
        }
        self.synth_music_note(key, t);
    }

    /// Fire `key` from its pre-rendered buffer at time `t`. `false` = not
    /// baked yet (or no context): the caller falls back to live synthesis.
    fn play_music_baked(&self, key: MusicKey, t: f64) -> bool {
        let ctx = match &self.ctx {
            Some(c) => c,
            None => return false,
        };
        let slots = self.baked_music.slots.borrow();
        let buf = match slots
            .iter()
            .find(|s| s.key == key)
            .and_then(|s| s.buf.as_ref())
        {
            Some(b) => b,
            None => return false,
        };
        let out = match self.music_out() {
            Some(o) => o,
            None => return false,
        };
        let src = match ctx.create_buffer_source() {
            Ok(s) => s,
            Err(_) => return false,
        };
        src.set_buffer(Some(buf));
        if src.connect_with_audio_node(&out).is_err() {
            return false;
        }
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        let _ = sched.start_with_when(t);
        true
    }

    /// The LIVE synthesis of one music voice at absolute time `t` — also
    /// what the offline pre-render runs (at t = 0, see
    /// [`Self::render_music_slot`]), so a baked note is the identical
    /// signal, just rendered ahead of time.
    fn synth_music_note(&self, key: MusicKey, t: f64) {
        let s = &self.song;
        let step_dur = self.step_dur();
        let gain = MUSIC_GAIN * s.intensity;
        match key {
            MusicKey::Bass(d) => {
                let f = degree_freq(s.root, s.scale, d);
                self.music_tone(f, f, t, step_dur * 1.9, gain * 1.3, s.bass_wave);
            }
            MusicKey::Lead(d) => {
                let f = degree_freq(s.root, s.scale, d);
                self.music_tone(f, f, t, step_dur * 0.9, gain, s.lead_wave);
            }
            MusicKey::Pad(d) => {
                // Bloom the pad note into a triad (root + third + fifth), held
                // across several steps with a slow attack for a chord bed.
                for interval in [0, 2, 4] {
                    let f = degree_freq(s.root, s.scale, d + interval);
                    self.music_pad(f, t, step_dur * 4.0, gain * 0.45, s.pad_wave);
                }
            }
            MusicKey::Arp(d) => {
                let f = degree_freq(s.root, s.scale, d);
                self.music_tone(f, f, t, step_dur * 0.7, gain * 0.7, s.arp_wave);
            }
            MusicKey::Kick => self.drum(Kick, t, gain),
            MusicKey::Hat => self.drum(Hat, t, gain),
            MusicKey::Snare => self.drum(Snare, t, gain),
        }
    }

    /// Seconds of dry signal one music voice needs when baked: the note
    /// duration its live envelope uses (a function of the song's step
    /// length — see [`Self::synth_music_note`]) plus the builders' small
    /// stop margin.
    fn music_key_len(&self, key: MusicKey) -> f64 {
        let sd = self.step_dur();
        match key {
            MusicKey::Bass(_) => sd * 1.9 + 0.03,
            MusicKey::Lead(_) => sd * 0.9 + 0.03,
            MusicKey::Pad(_) => sd * 4.0 + 0.03,
            MusicKey::Arp(_) => sd * 0.7 + 0.03,
            MusicKey::Kick => 0.21, // 0.18 s tone + stop margin (noise is 0.05)
            MusicKey::Hat => 0.06,  // 0.03 s noise tick + margin
            MusicKey::Snare => 0.16, // 0.13 s noise + margin (tone is 0.10)
        }
    }

    /// Enumerate the exact, finite voice set `song` can ever schedule: the
    /// distinct scale degrees of each melodic lane across every section,
    /// plus the up-to-three drum voices — in bake-priority order (drums
    /// first, then bass, lead, arp, pad). Typically 30–45 keys per song.
    fn music_keys(song: &SongSpec) -> Vec<MusicKey> {
        fn add(keys: &mut Vec<MusicKey>, k: MusicKey) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        let mut keys = Vec::new();
        for sec in song.sections {
            for &d in sec.drums {
                match d {
                    Silent => {}
                    Kick => add(&mut keys, MusicKey::Kick),
                    Hat => add(&mut keys, MusicKey::Hat),
                    Snare => add(&mut keys, MusicKey::Snare),
                }
            }
        }
        type Lane = (fn(&Section) -> &'static [i32], fn(i32) -> MusicKey);
        const LANES: [Lane; 4] = [
            (|s| s.bass, MusicKey::Bass),
            (|s| s.lead, MusicKey::Lead),
            (|s| s.arp, MusicKey::Arp),
            (|s| s.pad, MusicKey::Pad),
        ];
        for (pattern, mk) in LANES {
            for sec in song.sections {
                for &d in pattern(sec) {
                    if d != REST {
                        add(&mut keys, mk(d));
                    }
                }
            }
        }
        keys
    }

    /// Reset the music bake queue for the current song: enumerate its voice
    /// set fresh (all unbaked — notes fall back live until each render
    /// lands) and invalidate any in-flight render of the previous song.
    fn rebuild_music_bake(&self) {
        let m = &self.baked_music;
        m.gen.set(m.gen.get().wrapping_add(1));
        *m.slots.borrow_mut() = Self::music_keys(&self.song)
            .into_iter()
            .map(|key| MusicSlot { key, buf: None })
            .collect();
        m.next.set(0);
    }

    /// Render one synthesized drum hit at absolute time `t` (routed to the bus).
    fn drum(&self, hit: Drum, t: f64, gain: f64) {
        match hit {
            Silent => {}
            Kick => {
                self.music_tone(140.0, 45.0, t, 0.18, gain * 1.6, OscillatorType::Sine);
                self.music_noise(t, 0.05, gain * 0.4, BiquadFilterType::Lowpass, 400.0, 80.0);
            }
            Hat => {
                self.music_noise(
                    t,
                    0.03,
                    gain * 0.5,
                    BiquadFilterType::Highpass,
                    9000.0,
                    9000.0,
                );
            }
            Snare => {
                self.music_noise(
                    t,
                    0.13,
                    gain * 0.7,
                    BiquadFilterType::Highpass,
                    1800.0,
                    1400.0,
                );
                self.music_tone(220.0, 170.0, t, 0.10, gain * 0.5, OscillatorType::Triangle);
            }
        }
    }

    // --- helpers -----------------------------------------------------------

    /// Current audio-clock time, or 0.0 if we have no context. During an
    /// offline pre-render this is the offline clock (0.0 — rendering has not
    /// started), so [`Self::t0`] lands the voice [`SFX_LEAD`] into the buffer,
    /// exactly the lead it gets live.
    fn now(&self) -> f64 {
        self.bctx().map(|c| c.current_time()).unwrap_or(0.0)
    }

    /// The context nodes are currently built in: the [`OfflineAudioContext`]
    /// while a pre-render is being assembled, the live [`AudioContext`]
    /// otherwise. Node-creation methods live on [`BaseAudioContext`], which
    /// both deref to — this is the whole trick that lets every voice builder
    /// target either context unchanged.
    fn bctx(&self) -> Option<BaseAudioContext> {
        if let Some(r) = self.render.borrow().as_ref() {
            return Some(r.ctx.clone());
        }
        self.ctx
            .as_ref()
            .map(|c| AsRef::<BaseAudioContext>::as_ref(c).clone())
    }

    fn destination(&self) -> Option<AudioDestinationNode> {
        self.ctx.as_ref().map(|c| c.destination())
    }

    /// The node music voices connect to: the filtered music bus if we built it,
    /// otherwise the raw destination (graceful fallback). During an offline
    /// pre-render: the offline destination, so a note bakes its dry signal
    /// only — the bus (and its per-bar filter sweep) stays live and is
    /// reapplied at play time by [`Self::play_music_baked`].
    fn music_out(&self) -> Option<web_sys::AudioNode> {
        if let Some(r) = self.render.borrow().as_ref() {
            return Some(r.sink.clone());
        }
        if let Some(bus) = &self.music_bus {
            Some(AsRef::<web_sys::AudioNode>::as_ref(bus).clone())
        } else {
            self.destination()
                .map(|d| AsRef::<web_sys::AudioNode>::as_ref(&d).clone())
        }
    }

    /// Start time for a freshly-triggered SFX: a hair after "now" so that the
    /// sub-millisecond transients are never scheduled in the past (and thus
    /// silently skipped) — 12 ms is well under any perceptible latency.
    fn t0(&self) -> f64 {
        self.now() + SFX_LEAD
    }

    /// Next pseudo-random number in `[0, 1)` (xorshift32, deterministic).
    fn rand(&self) -> f64 {
        let mut s = self.rng.get();
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.rng.set(s);
        (s >> 8) as f64 / (1u32 << 24) as f64
    }

    /// A random multiplier in `[1 - amount, 1 + amount]` (pitch/timing jitter).
    fn jit(&self, amount: f64) -> f64 {
        1.0 + (self.rand() * 2.0 - 1.0) * amount
    }

    /// `true` with probability `p`.
    fn chance(&self, p: f64) -> bool {
        self.rand() < p
    }

    /// The generic SFX output: the bus's default "room" voice if we have one,
    /// otherwise the raw destination (graceful fallback). During an offline
    /// pre-render: the offline destination, so the room-voice kinds render
    /// their dry signal (their light room send is pre-wired into `room` and
    /// stays live).
    fn sfx_out(&self) -> Option<web_sys::AudioNode> {
        if let Some(r) = self.render.borrow().as_ref() {
            return Some(r.sink.clone());
        }
        if let Some(bus) = &self.sfx {
            Some(AsRef::<web_sys::AudioNode>::as_ref(&bus.room).clone())
        } else {
            self.destination()
                .map(|d| AsRef::<web_sys::AudioNode>::as_ref(&d).clone())
        }
    }

    /// Build a per-shot voice: an input gain that (optionally through a
    /// soft-clip WaveShaper, see [`Self::soft_clipper`] — `drive` ≤ 1 = clean)
    /// feeds the dry bus and, at level `wet`, the reverb send. All the layers
    /// of one sound connect to the returned node so they clip and reverberate
    /// *together* like a single recorded event. Falls back to the plain SFX
    /// output when the bus is unavailable; `None` only if there is no context.
    fn voice(&self, wet: f64, drive: f64) -> Option<web_sys::AudioNode> {
        self.voice_route(wet, drive, false)
    }

    /// A voice on the gun / hit bus path: no compressor / limiter (crest
    /// preserved) and the longer, brighter reverb.
    fn voice_real(&self, wet: f64, drive: f64) -> Option<web_sys::AudioNode> {
        self.voice_route(wet, drive, true)
    }

    /// The voice builder behind [`Self::voice`] / [`Self::voice_real`]:
    /// `real` selects the bus path (dry + reverb) the voice feeds.
    ///
    /// During an offline pre-render the voice is built DRY: the same input
    /// gain and per-voice soft-clip, but feeding the offline destination
    /// with no wet send — the send (and the whole live room / compressor /
    /// bus clip) is reapplied at play time by [`Self::play_baked`], so the
    /// baked buffer captures exactly the signal that live synthesis hands
    /// to the bus.
    fn voice_route(&self, wet: f64, drive: f64, real: bool) -> Option<web_sys::AudioNode> {
        if let Some(r) = self.render.borrow().as_ref() {
            let input = r.ctx.create_gain().ok()?;
            let _ = input.gain().set_value_at_time(1.0, 0.0);
            let mut post: web_sys::AudioNode = AsRef::<web_sys::AudioNode>::as_ref(&input).clone();
            if drive > 1.0 {
                if let Some(clip) = Self::soft_clipper(&r.ctx, &post, (1.0 / drive) as f32) {
                    post = clip;
                }
            }
            let _ = post.connect_with_audio_node(&r.sink);
            return Some(AsRef::<web_sys::AudioNode>::as_ref(&input).clone());
        }
        let (ctx, bus) = match (&self.ctx, &self.sfx) {
            (Some(c), Some(b)) => (c, b),
            _ => return self.sfx_out(),
        };
        let (dry, reverb_in) = if real {
            (&bus.dry_real, &bus.reverb_real_in)
        } else {
            (&bus.dry, &bus.reverb_in)
        };
        let input = ctx.create_gain().ok()?;
        let _ = input.gain().set_value_at_time(1.0, 0.0);
        let mut post: web_sys::AudioNode = AsRef::<web_sys::AudioNode>::as_ref(&input).clone();
        if drive > 1.0 {
            if let Some(clip) = Self::soft_clipper(ctx, &post, (1.0 / drive) as f32) {
                post = clip;
            }
        }
        let _ = post.connect_with_audio_node(dry);
        if wet > 0.0 {
            if let Ok(send) = ctx.create_gain() {
                let _ = send.gain().set_value_at_time(wet as f32, 0.0);
                let _ = post.connect_with_audio_node(&send);
                let _ = send.connect_with_audio_node(reverb_in);
            }
        }
        Some(AsRef::<web_sys::AudioNode>::as_ref(&input).clone())
    }

    /// Insert a soft clipper after `from`: a 0.5 pre-gain into a WaveShaper
    /// whose curve is the identity below `knee` and a `tanh` squash above it,
    /// with a ceiling of 1 (the pre-gain lets the curve cover ±2 so peaks up
    /// to 2 saturate smoothly instead of hard-clipping at the curve's edge).
    /// A low `knee` (0.2–0.3) is a hot, overloaded crunch on every transient;
    /// 0.7+ only rounds off the loudest peaks. Returns the shaper as the new
    /// tail of the chain, or `None` (chain untouched) if a node fails.
    fn soft_clipper(
        ctx: &BaseAudioContext,
        from: &web_sys::AudioNode,
        knee: f32,
    ) -> Option<web_sys::AudioNode> {
        let pre = ctx.create_gain().ok()?;
        let _ = pre.gain().set_value_at_time(0.5, 0.0);
        let shaper = ctx.create_wave_shaper().ok()?;
        let mut curve = Self::softclip_curve(knee, 4096);
        shaper.set_curve_opt_f32_slice(Some(curve.as_mut_slice()));
        shaper.set_oversample(OverSampleType::N2x);
        from.connect_with_audio_node(&pre).ok()?;
        pre.connect_with_audio_node(&shaper).ok()?;
        Some(AsRef::<web_sys::AudioNode>::as_ref(&shaper).clone())
    }

    /// The soft-clip transfer curve for [`Self::soft_clipper`], sampled over an
    /// input range of ±2: `y = u` for `|u| < knee`, then
    /// `knee + (1 - knee)·tanh((|u| - knee) / (1 - knee))` — continuous in
    /// value and slope at the knee, asymptotically 1.
    fn softclip_curve(knee: f32, n: usize) -> Vec<f32> {
        let a = knee.clamp(0.05, 0.95);
        (0..n)
            .map(|i| {
                let u = (i as f32 / (n - 1) as f32 * 2.0 - 1.0) * 2.0;
                let m = u.abs();
                let y = if m < a {
                    m
                } else {
                    a + (1.0 - a) * ((m - a) / (1.0 - a)).tanh()
                };
                y.copysign(u)
            })
            .collect()
    }

    /// SFX tone — enveloped oscillator into the SFX bus.
    fn tone(&self, f0: f64, f1: f64, start: f64, dur: f64, peak: f64, wave: OscillatorType) {
        if let Some(out) = self.sfx_out() {
            self.tone_out(&out, f0, f1, start, dur, peak, 0.005, wave);
        }
    }

    /// Music tone — enveloped oscillator into the filtered music bus.
    fn music_tone(&self, f0: f64, f1: f64, start: f64, dur: f64, peak: f64, wave: OscillatorType) {
        if let Some(out) = self.music_out() {
            self.tone_out(&out, f0, f1, start, dur, peak, 0.005, wave);
        }
    }

    /// Music pad tone — slow attack, long release, into the filtered bus.
    fn music_pad(&self, f: f64, start: f64, dur: f64, peak: f64, wave: OscillatorType) {
        if let Some(out) = self.music_out() {
            self.tone_out(&out, f, f, start, dur, peak, 0.06, wave);
        }
    }

    /// A single enveloped oscillator tone connected to `out`. If `f0 != f1` the
    /// pitch glides (exponentially) from `f0` to `f1` over `dur` for glitchy
    /// dives/sweeps. Rises to `peak` over `attack` then decays to near-silence.
    #[allow(clippy::too_many_arguments)]
    fn tone_out(
        &self,
        out: &web_sys::AudioNode,
        f0: f64,
        f1: f64,
        start: f64,
        dur: f64,
        peak: f64,
        attack: f64,
        wave: OscillatorType,
    ) {
        let ctx = match self.bctx() {
            Some(c) => c,
            None => return,
        };
        let (osc, gain) = match (ctx.create_oscillator(), ctx.create_gain()) {
            (Ok(o), Ok(g)) => (o, g),
            _ => return,
        };
        osc.set_type(wave);
        let freq = osc.frequency();
        let _ = freq.set_value_at_time(f0 as f32, start);
        if (f1 - f0).abs() > 0.01 {
            let _ = freq.exponential_ramp_to_value_at_time(f1.max(1.0) as f32, start + dur);
        }
        let g = gain.gain();
        let _ = g.set_value_at_time(0.0001, start);
        let _ = g.exponential_ramp_to_value_at_time(peak.max(0.0002) as f32, start + attack);
        let _ = g.exponential_ramp_to_value_at_time(0.0001, start + dur);
        let _ = osc.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(out);
        let sched: &web_sys::AudioScheduledSourceNode = osc.as_ref();
        let _ = sched.start_with_when(start);
        let _ = sched.stop_with_when(start + dur + 0.02);
    }

    /// SFX noise burst — into the SFX bus.
    fn noise(&self, start: f64, dur: f64, peak: f64, filter: BiquadFilterType, f0: f64, f1: f64) {
        if let Some(out) = self.sfx_out() {
            self.noise_out(&out, start, dur, peak, filter, f0, f1);
        }
    }

    /// Music noise burst — into the filtered music bus.
    fn music_noise(
        &self,
        start: f64,
        dur: f64,
        peak: f64,
        filter: BiquadFilterType,
        f0: f64,
        f1: f64,
    ) {
        if let Some(out) = self.music_out() {
            self.noise_out(&out, start, dur, peak, filter, f0, f1);
        }
    }

    /// A burst of the shared white-noise buffer through a sweeping biquad
    /// filter and a decaying gain envelope, connected to `out` — used for hits,
    /// whooshes, cracks, and the drum lane. Instant attack, filter Q of 1.
    #[allow(clippy::too_many_arguments)]
    fn noise_out(
        &self,
        out: &web_sys::AudioNode,
        start: f64,
        dur: f64,
        peak: f64,
        filter: BiquadFilterType,
        f0: f64,
        f1: f64,
    ) {
        self.noise_env(out, start, 0.0, dur, peak, filter, f0, f1, 1.0);
    }

    /// The general noise layer: `noise_full` with the filter sweep spanning
    /// the whole duration.
    #[allow(clippy::too_many_arguments)]
    fn noise_env(
        &self,
        out: &web_sys::AudioNode,
        start: f64,
        attack: f64,
        dur: f64,
        peak: f64,
        filter: BiquadFilterType,
        f0: f64,
        f1: f64,
        q: f64,
    ) {
        self.noise_full(out, start, attack, dur, dur, peak, filter, f0, f1, q);
    }

    /// The fully general noise layer: the shared noise buffer, read from a
    /// random offset (so no two bursts share a waveform), through a biquad
    /// of type `filter` sweeping `f0 → f1` over `sweep` seconds with
    /// resonance `q`, shaped by an envelope that rises to `peak` over
    /// `attack` (0 = instant) and decays exponentially to silence at
    /// `start + dur`.
    #[allow(clippy::too_many_arguments)]
    fn noise_full(
        &self,
        out: &web_sys::AudioNode,
        start: f64,
        attack: f64,
        dur: f64,
        sweep: f64,
        peak: f64,
        filter: BiquadFilterType,
        f0: f64,
        f1: f64,
        q: f64,
    ) {
        let (ctx, buf) = match (self.bctx(), &self.noise) {
            (Some(c), Some(b)) => (c, b),
            _ => return,
        };
        let (src, filt, gain) = match (
            ctx.create_buffer_source(),
            ctx.create_biquad_filter(),
            ctx.create_gain(),
        ) {
            (Ok(s), Ok(f), Ok(g)) => (s, f, g),
            _ => return,
        };
        src.set_buffer(Some(buf));
        src.set_loop(true);
        filt.set_type(filter);
        let ff = filt.frequency();
        let _ = ff.set_value_at_time(f0 as f32, start);
        if (f1 - f0).abs() > 1.0 {
            let _ =
                ff.exponential_ramp_to_value_at_time(f1.max(1.0) as f32, start + sweep.max(0.001));
        }
        let _ = filt.q().set_value_at_time(q as f32, start);
        let g = gain.gain();
        let peak = peak.max(0.0002) as f32;
        if attack > 0.0 {
            let _ = g.set_value_at_time(0.0001, start);
            let _ = g.linear_ramp_to_value_at_time(peak, start + attack);
        } else {
            let _ = g.set_value_at_time(peak, start);
        }
        let _ = g.exponential_ramp_to_value_at_time(0.0001, start + dur);
        let _ = src.connect_with_audio_node(&filt);
        let _ = filt.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(out);
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        // Random read offset into the (looped) noise buffer.
        let offset = self.rand() * (NOISE_SECONDS - 0.05);
        let _ = src.start_with_when_and_grain_offset(start, offset);
        let _ = sched.stop_with_when(start + dur + 0.02);
    }

    /// A swelling tone: linear attack from silence to `peak` over `attack`,
    /// then an exponential decay that is 20 dB down `d20` seconds after the
    /// peak (silence at 4× that); the pitch glides `f0 → f1` over the whole
    /// life (keep it tiny — clean glides read as cartoon), and `wobble` > 0
    /// adds a 10–30 Hz random walk to the frequency with that RMS fraction
    /// so it never sits still as a clean tone.
    #[allow(clippy::too_many_arguments)]
    fn swell_tone(
        &self,
        out: &web_sys::AudioNode,
        f0: f64,
        f1: f64,
        start: f64,
        attack: f64,
        d20: f64,
        peak: f64,
        wave: OscillatorType,
        wobble: f64,
    ) {
        let ctx = match self.bctx() {
            Some(c) => c,
            None => return,
        };
        let (osc, gain) = match (ctx.create_oscillator(), ctx.create_gain()) {
            (Ok(o), Ok(g)) => (o, g),
            _ => return,
        };
        let attack = attack.max(0.001);
        let d20 = d20.max(0.01);
        let end = start + attack + 4.0 * d20;
        osc.set_type(wave);
        let freq = osc.frequency();
        let _ = freq.set_value_at_time(f0 as f32, start);
        if (f1 - f0).abs() > 0.01 {
            let _ = freq.exponential_ramp_to_value_at_time(f1.max(1.0) as f32, end);
        }
        if wobble > 0.0 {
            let fc = 10.0 + self.rand() * 20.0;
            self.noise_lfo(&freq, fc, 0.7, wobble * f0, start, end - start, None);
        }
        let g = gain.gain();
        let _ = g.set_value_at_time(0.0001, start);
        let _ = g.linear_ramp_to_value_at_time(peak.max(0.0002) as f32, start + attack);
        let _ = g.exponential_ramp_to_value_at_time(0.0001, end);
        let _ = osc.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(out);
        let sched: &web_sys::AudioScheduledSourceNode = osc.as_ref();
        let _ = sched.start_with_when(start);
        let _ = sched.stop_with_when(end + 0.02);
    }

    /// Build the persistent music bus: a gain node feeding a lowpass biquad
    /// (whose cutoff we sweep per bar) into the destination. Returns
    /// `(None, None)` if any node fails to build.
    fn make_music_bus(ctx: &AudioContext) -> (Option<GainNode>, Option<BiquadFilterNode>) {
        let (gain, filt) = match (ctx.create_gain(), ctx.create_biquad_filter()) {
            (Ok(g), Ok(f)) => (g, f),
            _ => return (None, None),
        };
        filt.set_type(BiquadFilterType::Lowpass);
        let _ = filt.frequency().set_value_at_time(3000.0, 0.0);
        // A little resonance makes the sweep sing (that synthwave edge).
        let _ = filt.q().set_value_at_time(3.0, 0.0);
        let _ = gain.gain().set_value_at_time(1.0, 0.0);
        let _ = gain.connect_with_audio_node(&filt);
        let _ = filt.connect_with_audio_node(&ctx.destination());
        (Some(gain), Some(filt))
    }

    /// Build ~0.5s of white noise into an `AudioBuffer` we can reuse forever.
    /// Build the persistent SFX bus (see [`SfxBus`]): dry + reverb paths
    /// summing into a compressor, then a gentle soft-clip and an output trim.
    /// Returns `None` if any essential node fails to build (the reverb is
    /// optional: without it the bus is dry but still compressed).
    fn make_sfx_bus(ctx: &AudioContext) -> Option<SfxBus> {
        let comp = ctx.create_dynamics_compressor().ok()?;
        // Fast and firm: grabs the crack of a shot and audibly pumps back
        // over ~180 ms; the mechanical tails sit below threshold and keep
        // their dynamics.
        let _ = comp.threshold().set_value_at_time(-14.0, 0.0);
        let _ = comp.knee().set_value_at_time(6.0, 0.0);
        let _ = comp.ratio().set_value_at_time(4.0, 0.0);
        let _ = comp.attack().set_value_at_time(0.002, 0.0);
        let _ = comp.release().set_value_at_time(0.18, 0.0);

        // Bus limiter after the compressor: fills the wall (waveform
        // kurtosis ~2–2.5, crest ~8–12 dB on a single shot, ~12–15 dB on a
        // hit) without brick-walling.
        let mut last: web_sys::AudioNode = AsRef::<web_sys::AudioNode>::as_ref(&comp).clone();
        if let Ok(lim) = ctx.create_dynamics_compressor() {
            let _ = lim.threshold().set_value_at_time(-6.0, 0.0);
            let _ = lim.knee().set_value_at_time(0.0, 0.0);
            let _ = lim.ratio().set_value_at_time(20.0, 0.0);
            let _ = lim.attack().set_value_at_time(0.001, 0.0);
            let _ = lim.release().set_value_at_time(0.10, 0.0);
            if last.connect_with_audio_node(&lim).is_ok() {
                last = AsRef::<web_sys::AudioNode>::as_ref(&lim).clone();
            }
        }
        // Both paths sum here, into the bus soft-clipper.
        let sum = ctx.create_gain().ok()?;
        let _ = sum.gain().set_value_at_time(1.0, 0.0);
        let _ = last.connect_with_audio_node(&sum);

        // Gun / hit path: no compressor, no limiter — only a gentle
        // −1 dB / 4:1 safety so a crack keeps its 18–22 dB crest.
        let dry_real = ctx.create_gain().ok()?;
        let _ = dry_real.gain().set_value_at_time(1.0, 0.0);
        let real_sum: web_sys::AudioNode = match ctx.create_dynamics_compressor() {
            Ok(safe) => {
                let _ = safe.threshold().set_value_at_time(-1.0, 0.0);
                let _ = safe.knee().set_value_at_time(1.0, 0.0);
                let _ = safe.ratio().set_value_at_time(4.0, 0.0);
                let _ = safe.attack().set_value_at_time(0.0005, 0.0);
                let _ = safe.release().set_value_at_time(0.08, 0.0);
                let _ = safe.connect_with_audio_node(&sum);
                AsRef::<web_sys::AudioNode>::as_ref(&safe).clone()
            }
            Err(_) => AsRef::<web_sys::AudioNode>::as_ref(&sum).clone(),
        };
        let _ = dry_real.connect_with_audio_node(&real_sum);

        // Bus soft-clipper: barely touches normal peaks, rounds off the sum
        // of simultaneous shots instead of letting the DAC hard-clip.
        let trim = ctx.create_gain().ok()?;
        let _ = trim.gain().set_value_at_time(SFX_GAIN as f32, 0.0);
        last = AsRef::<web_sys::AudioNode>::as_ref(&sum).clone();
        if let Some(clip) = Self::soft_clipper(ctx, &last, 0.7) {
            last = clip;
        }
        let _ = last.connect_with_audio_node(&trim);
        let _ = trim.connect_with_audio_node(&ctx.destination());

        let dry = ctx.create_gain().ok()?;
        let _ = dry.gain().set_value_at_time(1.0, 0.0);
        let _ = dry.connect_with_audio_node(&comp);

        // Reverb: send -> low-cut -> convolver(IR) -> return -> compressor.
        let reverb_in = ctx.create_gain().ok()?;
        let _ = reverb_in.gain().set_value_at_time(1.0, 0.0);
        let wired = (|| {
            let hpf = ctx.create_biquad_filter().ok()?;
            hpf.set_type(BiquadFilterType::Highpass);
            let _ = hpf.frequency().set_value_at_time(180.0, 0.0);
            let _ = hpf.q().set_value_at_time(0.7, 0.0);
            let conv = ctx.create_convolver().ok()?;
            conv.set_normalize(true);
            conv.set_buffer(Some(&Self::make_impulse(ctx)?));
            let ret = ctx.create_gain().ok()?;
            let _ = ret.gain().set_value_at_time(REVERB_RETURN as f32, 0.0);
            reverb_in.connect_with_audio_node(&hpf).ok()?;
            hpf.connect_with_audio_node(&conv).ok()?;
            conv.connect_with_audio_node(&ret).ok()?;
            ret.connect_with_audio_node(&comp).ok()?;
            Some(())
        })();

        // Gun / hit reverb: send -> low-cut -> convolver(bright, long IR)
        // -> return -> the real path's safety limiter.
        let reverb_real_in = ctx.create_gain().ok()?;
        let _ = reverb_real_in.gain().set_value_at_time(1.0, 0.0);
        let _ = (|| {
            let hpf = ctx.create_biquad_filter().ok()?;
            hpf.set_type(BiquadFilterType::Highpass);
            let _ = hpf.frequency().set_value_at_time(120.0, 0.0);
            let _ = hpf.q().set_value_at_time(0.7, 0.0);
            let conv = ctx.create_convolver().ok()?;
            conv.set_normalize(true);
            conv.set_buffer(Some(&Self::make_impulse_real(ctx)?));
            let ret = ctx.create_gain().ok()?;
            let _ = ret.gain().set_value_at_time(REVERB_REAL_RETURN as f32, 0.0);
            reverb_real_in.connect_with_audio_node(&hpf).ok()?;
            hpf.connect_with_audio_node(&conv).ok()?;
            conv.connect_with_audio_node(&ret).ok()?;
            ret.connect_with_audio_node(&real_sum).ok()?;
            Some(())
        })();

        // Default voice for the misc. SFX: dry + a light room send.
        let room = ctx.create_gain().ok()?;
        let _ = room.gain().set_value_at_time(1.0, 0.0);
        let _ = room.connect_with_audio_node(&dry);
        if wired.is_some() {
            if let Ok(send) = ctx.create_gain() {
                let _ = send.gain().set_value_at_time(0.18, 0.0);
                let _ = room.connect_with_audio_node(&send);
                let _ = send.connect_with_audio_node(&reverb_in);
            }
        }

        Some(SfxBus {
            dry,
            reverb_in,
            dry_real,
            reverb_real_in,
            room,
        })
    }

    /// The gun / hit impulse response: `IR_REAL_SECONDS` of stereo noise
    /// with the measured field-recording room envelope after the peak —
    /// −10 dB @100 ms, −21 @200, −25 @300, −31 @600, −41 @1000, −50 @1500 —
    /// (piecewise-linear in dB), hotter sparse early reflections, and a
    /// FIXED one-pole lowpass (~3 kHz) so the tail stays mid-bright
    /// (centroid ~1.1–1.5 kHz) instead of darkening.
    fn make_impulse_real(ctx: &AudioContext) -> Option<AudioBuffer> {
        let sr = ctx.sample_rate();
        let len = (sr as f64 * IR_REAL_SECONDS) as u32;
        if len == 0 {
            return None;
        }
        let buf = ctx.create_buffer(2, len, sr).ok()?;
        let predelay = (sr as f64 * 0.006) as usize;
        let early = (sr as f64 * 0.04) as usize;
        const PTS: [(f64, f64); 8] = [
            (0.0, 0.0),
            (0.1, -10.0),
            (0.2, -21.0),
            (0.3, -25.0),
            (0.6, -31.0),
            (1.0, -41.0),
            (1.5, -50.0),
            (1.7, -60.0),
        ];
        let env_db = |t: f64| -> f64 {
            for w in PTS.windows(2) {
                let (t0, d0) = w[0];
                let (t1, d1) = w[1];
                if t <= t1 {
                    return d0 + (d1 - d0) * ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
                }
            }
            -60.0
        };
        let a = (-2.0 * std::f64::consts::PI * 3000.0 / sr as f64).exp() as f32;
        let mut data = vec![0f32; len as usize];
        for ch in 0..2u32 {
            let mut state: u32 = 0x7F4A_7C15 ^ (ch.wrapping_mul(0x9E37_79B9) + 3);
            let mut lp = 0f32;
            for (i, x) in data.iter_mut().enumerate() {
                if i < predelay {
                    *x = 0.0;
                    continue;
                }
                let n = i - predelay;
                let t = n as f64 / sr as f64;
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let white = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let mut env = 10f64.powf(env_db(t) / 20.0);
                if n < early {
                    let sparse = if state & 3 == 0 { 1.0 } else { 0.4 };
                    env *= (1.0 + 0.5 * (1.0 - n as f64 / early as f64)) * sparse;
                }
                lp = a * lp + (1.0 - a) * white;
                *x = lp * env as f32;
            }
            buf.copy_to_channel(&data, ch as i32).ok()?;
        }
        Some(buf)
    }

    /// Synthesize a stereo room impulse response: `IR_SECONDS` of white noise
    /// (independent per channel, for width) under an exponential decay, with a
    /// short pre-delay, a denser/louder first 40 ms of early reflections, and
    /// a one-pole lowpass whose cutoff falls over the tail (air absorption —
    /// the highs die first, exactly as in a real medium-sized concrete room).
    fn make_impulse(ctx: &AudioContext) -> Option<AudioBuffer> {
        let sr = ctx.sample_rate();
        let len = (sr as f64 * IR_SECONDS) as u32;
        if len == 0 {
            return None;
        }
        let buf = ctx.create_buffer(2, len, sr).ok()?;
        let predelay = (sr as f64 * 0.009) as usize;
        let early = (sr as f64 * 0.04) as usize;
        // RT60 ≈ 6.9·tau; tau chosen for a ~0.9 s tail.
        let tau = 0.13f64;
        let mut data = vec![0f32; len as usize];
        for ch in 0..2u32 {
            let mut state: u32 = 0xA511_E9B3 ^ (ch.wrapping_mul(0x6C8E_9CF5) + 1);
            let mut lp = 0f32;
            for (i, x) in data.iter_mut().enumerate() {
                if i < predelay {
                    *x = 0.0;
                    continue;
                }
                let n = i - predelay;
                let t = n as f64 / sr as f64;
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let white = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let mut env = (-t / tau).exp();
                if n < early {
                    // Sparse-ish, hotter early reflections, blending into
                    // the diffuse tail by the end of the window.
                    let sparse = if state & 3 == 0 { 1.0 } else { 0.35 };
                    let hot = 1.0 + 0.7 * (1.0 - n as f64 / early as f64);
                    env *= hot * sparse;
                }
                // Air absorption: cutoff slides from ~7 kHz to ~1.5 kHz.
                let fc = 7000.0 * (-t / 0.45).exp() + 1500.0;
                let a = (-2.0 * std::f64::consts::PI * fc / sr as f64).exp() as f32;
                lp = a * lp + (1.0 - a) * white;
                *x = lp * env as f32;
            }
            buf.copy_to_channel(&data, ch as i32).ok()?;
        }
        Some(buf)
    }

    /// Build `NOISE_SECONDS` of white noise into an `AudioBuffer` we can reuse
    /// forever (bursts read it from random offsets). Uses a tiny xorshift PRNG
    /// so we need no `rand`/`js_sys` dependency.
    fn make_noise(ctx: &AudioContext) -> Option<AudioBuffer> {
        let sr = ctx.sample_rate();
        let len = (sr as f64 * NOISE_SECONDS) as u32;
        if len == 0 {
            return None;
        }
        let buf = ctx.create_buffer(1, len, sr).ok()?;
        let mut data = vec![0f32; len as usize];
        let mut state: u32 = 0x9E37_79B9;
        for x in data.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *x = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
        buf.copy_to_channel(&data, 0).ok()?;
        Some(buf)
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every note any song can ever schedule must map to an enumerated
    /// [`MusicKey`], and the per-song voice set must stay small enough that
    /// baking each exact pitch (no `playback_rate` transposition) is cheap.
    /// Run with `--nocapture` to see the per-song counts.
    #[test]
    fn music_voice_sets_are_small_and_complete() {
        for song in SONGS {
            let keys = AudioEngine::music_keys(song);
            assert!(!keys.is_empty(), "{}: empty voice set", song.name);
            assert!(
                keys.len() <= 64,
                "{}: {} voices — too many to bake each exact pitch",
                song.name,
                keys.len()
            );
            // No duplicates (the queue bakes each key exactly once).
            for (i, k) in keys.iter().enumerate() {
                assert!(!keys[..i].contains(k), "{}: duplicate {:?}", song.name, k);
            }
            // Completeness: every schedulable note has a key.
            for sec in song.sections {
                for &d in sec.bass {
                    assert!(d == REST || keys.contains(&MusicKey::Bass(d)));
                }
                for &d in sec.lead {
                    assert!(d == REST || keys.contains(&MusicKey::Lead(d)));
                }
                for &d in sec.pad {
                    assert!(d == REST || keys.contains(&MusicKey::Pad(d)));
                }
                for &d in sec.arp {
                    assert!(d == REST || keys.contains(&MusicKey::Arp(d)));
                }
                for &dr in sec.drums {
                    let key = match dr {
                        Silent => continue,
                        Kick => MusicKey::Kick,
                        Hat => MusicKey::Hat,
                        Snare => MusicKey::Snare,
                    };
                    assert!(keys.contains(&key));
                }
            }
            let drums = keys
                .iter()
                .filter(|k| matches!(k, MusicKey::Kick | MusicKey::Hat | MusicKey::Snare))
                .count();
            let count = |f: fn(&MusicKey) -> bool| keys.iter().filter(|k| f(k)).count();
            println!(
                "{:14} {:2} voices (drums {} bass {:2} lead {:2} arp {:2} pad {:2})",
                song.name,
                keys.len(),
                drums,
                count(|k| matches!(k, MusicKey::Bass(_))),
                count(|k| matches!(k, MusicKey::Lead(_))),
                count(|k| matches!(k, MusicKey::Arp(_))),
                count(|k| matches!(k, MusicKey::Pad(_))),
            );
        }
    }

    /// The bake-priority split must keep every attack and hit kind in the
    /// combat prefix that renders before the music voices.
    #[test]
    fn combat_sfx_prefix_covers_attacks_and_hits() {
        assert!(SFX_COMBAT_KINDS <= SFX_KINDS.len());
        for kind in &SFX_KINDS[..SFX_COMBAT_KINDS] {
            assert!(!matches!(
                kind,
                SfxKind::Death | SfxKind::LevelClear | SfxKind::MaskCrack | SfxKind::Elevator
            ));
        }
        for combat in [
            SfxKind::AttackGun,
            SfxKind::AttackMachinegun,
            SfxKind::AttackShotgun,
            SfxKind::AttackClub,
            SfxKind::HitGun,
            SfxKind::HitMachinegun,
            SfxKind::HitShotgun,
            SfxKind::HitClub,
        ] {
            let pos = SFX_KINDS.iter().position(|k| *k == combat).unwrap();
            assert!(pos < SFX_COMBAT_KINDS);
        }
    }
}
