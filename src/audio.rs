//! Procedural audio engine for Open Miami // Rogue Purge.
//!
//! Everything here is synthesized at runtime with the Web Audio API (via
//! `web-sys`): oscillators for tones, a white-noise buffer for hits and
//! whooshes, all shaped by gain envelopes for a punchy, glitchy synthwave feel.
//! No audio files, no extra dependencies.
//!
//! The music runs through a dedicated bus — every note flows into a shared
//! lowpass [`web_sys::BiquadFilterNode`] whose cutoff is swept once per bar for
//! that classic synthwave/darksynth filter motion — while one-shot SFX bypass
//! the bus and hit the destination directly (so they always cut through).
//!
//! Robustness first: if the `AudioContext` (or any node) fails to build we
//! silently degrade to silence. Nothing in here ever panics or unwraps a
//! fallible Web Audio call — every `Result` is swallowed so the game runs fine
//! even when audio is unavailable or blocked by the browser.

use web_sys::{
    AudioBuffer, AudioContext, AudioDestinationNode, BiquadFilterNode, BiquadFilterType, GainNode,
    OscillatorType,
};

/// Look-ahead window (seconds) for the music scheduler: we queue notes this far
/// in advance of the audio clock so playback never gaps between frames.
const LOOKAHEAD: f64 = 0.15;

/// Master music level — kept low so the looping backing never buries the SFX.
const MUSIC_GAIN: f64 = 0.07;

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

/// The self-contained audio engine. Construct once, hand it around, drive
/// `update()` from the game loop.
pub struct AudioEngine {
    /// `None` if the browser refused to give us an audio context.
    ctx: Option<AudioContext>,
    /// Pre-rendered white noise, reused (via cheap buffer-source nodes) for
    /// every percussive/whoosh sound.
    noise: Option<AudioBuffer>,
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
}

impl AudioEngine {
    /// Try to create the audio context. Never fails hard — on any error the
    /// engine simply stays silent.
    pub fn new() -> Self {
        let ctx = AudioContext::new().ok();
        let noise = ctx.as_ref().and_then(Self::make_noise);
        let (music_bus, music_filter) = ctx
            .as_ref()
            .map(Self::make_music_bus)
            .unwrap_or((None, None));
        Self {
            ctx,
            noise,
            music_bus,
            music_filter,
            music_playing: false,
            next_note_time: 0.0,
            section: 0,
            step: 0,
            song: SONGS[0],
            mute: [false; NUM_CHANNELS],
            solo: [false; NUM_CHANNELS],
        }
    }

    /// Resume the context. Browsers start it suspended until a user gesture,
    /// so the integrator should call this on the first input (e.g. pressing
    /// Enter to start the game).
    pub fn resume(&self) {
        if let Some(ctx) = &self.ctx {
            let _ = ctx.resume();
        }
    }

    // --- one-shot SFX: attacks ---------------------------------------------
    //
    // A per-weapon taxonomy, each built by *layering* synthesized elements —
    // noise transients + fast pitch/amplitude envelopes + short filtered bursts
    // — for a realistic, weighty feel. Attacks are the weapon firing/swinging;
    // hits (below) are the impact landing on a metal bot.

    /// CLUB attack — swinging a heavy metal bar: a ripping air-whoosh that
    /// resolves into a heavy metallic CLANG with a low body thud. Reads as a
    /// weighty two-handed swing landing on something solid.
    pub fn play_attack_club(&self) {
        let t = self.now();
        // Swing-whoosh: air ripping past the bar, sweeping up fast (two bands
        // layered for a fuller "vwoomp").
        self.noise(t, 0.12, 0.30, BiquadFilterType::Bandpass, 400.0, 2600.0);
        self.noise(t, 0.10, 0.20, BiquadFilterType::Highpass, 600.0, 3400.0);
        // Heavy metal CLANG of the bar at the end of the swing.
        self.metal_clang(t + 0.10, 320.0, 0.34, 0.30, OscillatorType::Square);
        // Low body thud so the blow lands with real weight.
        self.tone(150.0, 52.0, t + 0.10, 0.16, 0.38, OscillatorType::Sine);
        self.noise(t + 0.10, 0.07, 0.22, BiquadFilterType::Lowpass, 520.0, 90.0);
    }

    /// GUN attack — a bright, snappy pistol crack: ultra-sharp transient, a
    /// diving muzzle snap, and a punchy low recoil pop.
    pub fn play_attack_gun(&self) {
        let t = self.now();
        // Ultra-sharp high crack transient.
        self.noise(t, 0.03, 0.60, BiquadFilterType::Highpass, 3800.0, 1400.0);
        // Bright muzzle snap diving instantly.
        self.tone(2000.0, 260.0, t, 0.05, 0.34, OscillatorType::Square);
        // Mid crack body for grit.
        self.tone(700.0, 150.0, t, 0.05, 0.22, OscillatorType::Sawtooth);
        // Low recoil pop / punch underneath.
        self.tone(180.0, 68.0, t, 0.10, 0.30, OscillatorType::Sawtooth);
        self.noise(t, 0.05, 0.22, BiquadFilterType::Lowpass, 700.0, 120.0);
    }

    /// MACHINEGUN attack — a fast, tight rattling burst of many sharp rounds.
    pub fn play_attack_machinegun(&self) {
        let t = self.now();
        let rounds = 9;
        let spacing = 0.033;
        for i in 0..rounds {
            let at = t + i as f64 * spacing;
            self.noise(at, 0.025, 0.42, BiquadFilterType::Highpass, 2800.0, 1500.0);
            self.tone(1500.0, 320.0, at, 0.03, 0.24, OscillatorType::Square);
            self.tone(200.0, 78.0, at, 0.045, 0.22, OscillatorType::Sawtooth);
            self.noise(at, 0.03, 0.14, BiquadFilterType::Lowpass, 600.0, 110.0);
        }
    }

    /// SHOTGUN attack — an enormous, cavernous low boom with a bright crack: a
    /// triple-layered sub-bass drop under a wide body blast.
    pub fn play_attack_shotgun(&self) {
        let t = self.now();
        // Enormous low boom — three detuned layers diving deep for weight.
        self.tone(140.0, 36.0, t, 0.40, 0.44, OscillatorType::Sawtooth);
        self.tone(95.0, 30.0, t, 0.46, 0.42, OscillatorType::Sine);
        self.tone(70.0, 26.0, t, 0.50, 0.34, OscillatorType::Sine);
        // Wide low-passed body blast — the shell's guts.
        self.noise(t, 0.36, 0.46, BiquadFilterType::Lowpass, 1600.0, 220.0);
        // Sharp bright crack on the leading edge.
        self.noise(t, 0.05, 0.34, BiquadFilterType::Highpass, 2600.0, 1500.0);
    }

    // --- one-shot SFX: hits (impact on a metal bot) ------------------------

    /// CLUB hit — a heavy metal bar crashing into metal plating: a broadband
    /// impact transient over a big low clang and a crushing low body.
    pub fn play_hit_club(&self) {
        let t = self.now();
        // Broadband metallic impact transient (two bands).
        self.noise(t, 0.06, 0.34, BiquadFilterType::Bandpass, 1400.0, 700.0);
        self.noise(t, 0.04, 0.26, BiquadFilterType::Highpass, 2000.0, 900.0);
        // The big low clang of bar-on-bot.
        self.metal_clang(t, 240.0, 0.34, 0.30, OscillatorType::Square);
        // Heavy low body so it reads as a crushing blow.
        self.tone(128.0, 52.0, t, 0.13, 0.32, OscillatorType::Sine);
    }

    /// GUN hit — a bright, snappy metallic ping off the bot's plating.
    pub fn play_hit_gun(&self) {
        let t = self.now();
        self.noise(t, 0.045, 0.46, BiquadFilterType::Highpass, 2800.0, 1500.0);
        self.metal_clang(t, 560.0, 0.32, 0.20, OscillatorType::Square);
        self.tone(160.0, 58.0, t, 0.08, 0.26, OscillatorType::Square);
    }

    /// MACHINEGUN hit — a fast, tight stutter of small metallic pings.
    pub fn play_hit_machinegun(&self) {
        let t = self.now();
        let rounds = 9;
        let spacing = 0.033;
        for i in 0..rounds {
            let at = t + i as f64 * spacing;
            self.noise(at, 0.025, 0.28, BiquadFilterType::Highpass, 2800.0, 1700.0);
            self.tone(
                820.0 + i as f64 * 14.0,
                400.0,
                at,
                0.04,
                0.22,
                OscillatorType::Square,
            );
            self.tone(1240.0, 600.0, at, 0.03, 0.14, OscillatorType::Sawtooth);
        }
    }

    /// SHOTGUN hit — a huge ringing metallic crash; the heaviest impact, with a
    /// double clang and a deep sub-boom.
    pub fn play_hit_shotgun(&self) {
        let t = self.now();
        self.noise(t, 0.26, 0.50, BiquadFilterType::Highpass, 1600.0, 500.0);
        self.metal_clang(t, 280.0, 0.38, 0.38, OscillatorType::Sawtooth);
        self.metal_clang(t + 0.02, 190.0, 0.26, 0.42, OscillatorType::Square);
        self.tone(80.0, 32.0, t, 0.24, 0.34, OscillatorType::Sine);
    }

    // --- one-shot SFX: legacy aliases --------------------------------------
    //
    // Kept so existing callers (lib.rs) keep compiling. They forward to the new
    // per-weapon methods above.

    /// Legacy alias — a generic gunshot. Forwards to [`Self::play_attack_gun`].
    pub fn play_shoot(&self) {
        self.play_attack_gun();
    }

    /// Legacy alias — a generic bullet impact. Forwards to [`Self::play_hit_gun`].
    pub fn play_hit(&self) {
        self.play_hit_gun();
    }

    /// Legacy alias for the old "fist" name — now the CLUB swing.
    pub fn play_attack_fist(&self) {
        self.play_attack_club();
    }

    /// Legacy alias for the old "fist" name — now the CLUB impact.
    pub fn play_hit_fist(&self) {
        self.play_hit_club();
    }

    // --- one-shot SFX: non-combat ------------------------------------------

    /// Short descending glitch — a rogue AI goes down.
    pub fn play_enemy_down(&self) {
        let t = self.now();
        self.tone(600.0, 70.0, t, 0.20, 0.28, OscillatorType::Sawtooth);
        self.tone(606.0, 66.0, t, 0.20, 0.14, OscillatorType::Square);
    }

    /// Bright rising two-tone — weapon pickup / swap.
    pub fn play_pickup(&self) {
        let t = self.now();
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
        let t = self.now();
        self.noise(t, 0.22, 0.22, BiquadFilterType::Highpass, 200.0, 1600.0);
    }

    /// Harsh low buzz — the player takes damage.
    pub fn play_player_hurt(&self) {
        let t = self.now();
        self.tone(95.0, 70.0, t, 0.18, 0.30, OscillatorType::Sawtooth);
        self.tone(140.0, 90.0, t, 0.18, 0.16, OscillatorType::Square);
    }

    /// Longer downward dive — the player dies / SYSTEM HALTED.
    pub fn play_death(&self) {
        let t = self.now();
        self.tone(420.0, 40.0, t, 0.65, 0.32, OscillatorType::Sawtooth);
        self.noise(t, 0.60, 0.18, BiquadFilterType::Lowpass, 1800.0, 120.0);
    }

    /// Short triumphant arp — SECTOR PURGED.
    pub fn play_level_clear(&self) {
        let t = self.now();
        let notes = [523.25, 659.25, 783.99, 1046.50];
        for (i, f) in notes.iter().enumerate() {
            let at = t + i as f64 * 0.09;
            self.tone(*f, *f, at, 0.14, 0.20, OscillatorType::Square);
        }
    }

    /// Nasty shattering noise burst — a boss's mask breaks. A special hit.
    pub fn play_mask_crack(&self) {
        let t = self.now();
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
        let t = self.now();
        // Slow ominous swell rising a fifth.
        self.tone(110.0, 165.0, t, 0.95, 0.16, OscillatorType::Sawtooth);
        self.tone(110.6, 166.5, t, 0.95, 0.10, OscillatorType::Triangle);
        // Airy noise rising underneath the swell.
        self.noise(t, 0.9, 0.05, BiquadFilterType::Highpass, 400.0, 3000.0);
        // The "ding" at the top — two chiming sines a fifth apart.
        self.tone(880.0, 880.0, t + 0.72, 0.5, 0.18, OscillatorType::Sine);
        self.tone(1318.5, 1318.5, t + 0.78, 0.45, 0.11, OscillatorType::Sine);
    }

    /// A struck-metal clang: a dense stack of *inharmonic* partials (ratios that
    /// are not simple integers) so it rings as unpitched metal, each partial
    /// diving and decaying fast. Six partials + a bright noise transient give it
    /// real bite and body. Layered by the club/gun/shotgun hit methods.
    fn metal_clang(&self, t: f64, base: f64, peak: f64, dur: f64, wave: OscillatorType) {
        const PARTIALS: [f64; 6] = [1.0, 1.41, 2.11, 2.71, 3.63, 4.51];
        // A short bright noise chink stapled to the front for attack.
        self.noise(
            t,
            0.02,
            peak * 0.6,
            BiquadFilterType::Bandpass,
            base * 3.0,
            base * 1.5,
        );
        for (i, r) in PARTIALS.iter().enumerate() {
            let g = peak * 0.86f64.powi(i as i32) / (i as f64 * 0.6 + 1.0);
            let f = base * r;
            self.tone(f, f * 0.55, t, dur * (0.45 + 0.13 * i as f64), g, wave);
        }
    }

    // --- music -------------------------------------------------------------

    /// Begin the looping backing track using the current song (idempotent).
    pub fn start_music(&mut self) {
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
    /// The arrangement restarts from its first section.
    pub fn set_song(&mut self, spec: SongSpec) {
        self.song = spec;
        self.section = 0;
        self.step = 0;
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
    fn schedule_step(&self, step: usize, t: f64) {
        let sec = match self.section_ref() {
            Some(s) => s,
            None => return,
        };
        let s = &self.song;
        let step_dur = self.step_dur();
        let gain = MUSIC_GAIN * s.intensity;

        if self.channel_audible(0) {
            if let Some(d) = degree_at(sec.bass, step) {
                let f = degree_freq(s.root, s.scale, d);
                self.music_tone(f, f, t, step_dur * 1.9, gain * 1.3, s.bass_wave);
            }
        }
        if self.channel_audible(1) {
            if let Some(d) = degree_at(sec.lead, step) {
                let f = degree_freq(s.root, s.scale, d);
                self.music_tone(f, f, t, step_dur * 0.9, gain, s.lead_wave);
            }
        }
        if self.channel_audible(2) {
            if let Some(d) = degree_at(sec.pad, step) {
                // Bloom the pad note into a triad (root + third + fifth), held
                // across several steps with a slow attack for a chord bed.
                for interval in [0, 2, 4] {
                    let f = degree_freq(s.root, s.scale, d + interval);
                    self.music_pad(f, t, step_dur * 4.0, gain * 0.45, s.pad_wave);
                }
            }
        }
        if self.channel_audible(3) {
            if let Some(d) = degree_at(sec.arp, step) {
                let f = degree_freq(s.root, s.scale, d);
                self.music_tone(f, f, t, step_dur * 0.7, gain * 0.7, s.arp_wave);
            }
        }
        if self.channel_audible(4) {
            self.drum(drum_at(sec.drums, step), t, gain);
        }
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

    /// Current audio-clock time, or 0.0 if we have no context.
    fn now(&self) -> f64 {
        self.ctx.as_ref().map(|c| c.current_time()).unwrap_or(0.0)
    }

    fn destination(&self) -> Option<AudioDestinationNode> {
        self.ctx.as_ref().map(|c| c.destination())
    }

    /// The node music voices connect to: the filtered music bus if we built it,
    /// otherwise the raw destination (graceful fallback).
    fn music_out(&self) -> Option<web_sys::AudioNode> {
        if let Some(bus) = &self.music_bus {
            Some(AsRef::<web_sys::AudioNode>::as_ref(bus).clone())
        } else {
            self.destination()
                .map(|d| AsRef::<web_sys::AudioNode>::as_ref(&d).clone())
        }
    }

    /// SFX tone — enveloped oscillator straight to the destination.
    fn tone(&self, f0: f64, f1: f64, start: f64, dur: f64, peak: f64, wave: OscillatorType) {
        if let Some(dest) = self.destination() {
            let out = AsRef::<web_sys::AudioNode>::as_ref(&dest).clone();
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
        let ctx = match &self.ctx {
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

    /// SFX noise burst — straight to the destination.
    fn noise(&self, start: f64, dur: f64, peak: f64, filter: BiquadFilterType, f0: f64, f1: f64) {
        if let Some(dest) = self.destination() {
            let out = AsRef::<web_sys::AudioNode>::as_ref(&dest).clone();
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
    /// whooshes, cracks, and the drum lane.
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
        let (ctx, buf) = match (&self.ctx, &self.noise) {
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
        filt.set_type(filter);
        let ff = filt.frequency();
        let _ = ff.set_value_at_time(f0 as f32, start);
        if (f1 - f0).abs() > 1.0 {
            let _ = ff.exponential_ramp_to_value_at_time(f1.max(1.0) as f32, start + dur);
        }
        let g = gain.gain();
        let _ = g.set_value_at_time(peak.max(0.0002) as f32, start);
        let _ = g.exponential_ramp_to_value_at_time(0.0001, start + dur);
        let _ = src.connect_with_audio_node(&filt);
        let _ = filt.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(out);
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        let _ = sched.start_with_when(start);
        let _ = sched.stop_with_when(start + dur + 0.02);
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
    /// Uses a tiny xorshift PRNG so we need no `rand`/`js_sys` dependency.
    fn make_noise(ctx: &AudioContext) -> Option<AudioBuffer> {
        let sr = ctx.sample_rate();
        let len = (sr as f64 * 0.5) as u32;
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
        buf.copy_to_channel(&mut data, 0).ok()?;
        Some(buf)
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}
