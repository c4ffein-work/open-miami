//! The music DATA of Open Miami // Rogue Purge: the song format, the music
//! theory helpers that read it, and every song. No Web Audio in here — this
//! module is plain data + arithmetic, compiled and unit-tested on the host;
//! `audio.rs` (wasm-only) is the synthesizer that plays it.
//!
//! A song is *plain, `const`-able data*: a key (root frequency + scale), a
//! tempo, four [`Voice`]s (the melodic instruments), and an ordered list of
//! SECTIONS.
//!
//! Each [`Section`] is its own multi-bar block of five step-sequenced channels
//! (bass, lead, pad, arp, drums). A [`SongSpec`] strings sections together into
//! a real arrangement — intro / verse / refrain / bridge / variation — so a
//! full play-through develops over time and the refrain *returns* instead of a
//! single bar looping forever. Sections are just `&'static` slices of
//! patterns, so a section can appear several times in the order (that is how a
//! refrain comes back) at zero extra cost.
//!
//! Melodic patterns are written as *scale degrees* (see [`degree_freq`]): `0`
//! is the root, `1` the next scale note up, `7` an octave up (for a 7-note
//! scale), negative degrees drop below the root. [`REST`] means silence for
//! that step; [`HOLD`] TIES the previous note through the step (a note's
//! length = 1 + the `HOLD`s that follow it: `0, HOLD, HOLD, HOLD` is one
//! quarter-note root, `0, 0, 0, 0` is four retriggered sixteenths). This keeps
//! a song readable and in-key no matter which root/scale it uses.
//!
//! Every lane has an optional parallel VELOCITY lane (`bass_vel` …
//! `drums_vel`): one `0..=`[`MAX_VEL`] per step, looping like the notes (an
//! empty lane = every note at full velocity). Velocity scales the note's
//! amplitude linearly — accents, ghost notes, and the retriggered
//! "side-chain pump" (`vel: &[3, 6, 8, 9]` under `bass: &[0, 0, 0, 0]`).
//!
//! Lanes inside a section may differ in length: a short 16-step bass simply
//! repeats under a longer 32-step lead. A section's length is its longest
//! lane, so authoring a 2-bar section only means writing one lane at 32 steps.
//!
//! The `pad` lane is special: each note blooms into a full triad (root + third
//! + fifth taken from the scale) with a slow attack, for sustained chord beds.

/// Sentinel used inside a pattern to mean "rest" (no note this step).
pub const REST: i32 = i32::MIN;
/// Sentinel used inside a pattern to mean "tie": the previous note of the
/// lane sustains through this step instead of a new one starting.
pub const HOLD: i32 = i32::MIN + 1;
/// Full velocity: the top of a velocity lane's `0..=MAX_VEL` scale (tracker
/// style, one digit per step). An empty velocity lane plays everything here.
pub const MAX_VEL: u8 = 9;

/// Number of sequenced channels (rows in the tracker view).
pub const NUM_CHANNELS: usize = 6;
/// Channel (lane) indices, `0..NUM_CHANNELS`.
pub const BASS: usize = 0;
pub const LEAD: usize = 1;
pub const PAD: usize = 2;
pub const ARP: usize = 3;
pub const DRUMS: usize = 4;
/// The second percussion lane: same kit as `DRUMS`, so a hat can ride over
/// a kick, a clap can layer a snare, a crash can top a downbeat.
pub const PERC: usize = 5;

/// Human-readable channel names, indexed 0..[`NUM_CHANNELS`].
pub const CHANNEL_NAMES: [&str; NUM_CHANNELS] = ["BASS", "LEAD", "PAD", "ARP", "DRUMS", "PERC"];

/// Scale = semitone offsets from the root, one octave's worth. Darker modes
/// (flat 2nd, tritone) read as more menacing — we escalate them across floors.
pub type Scale = &'static [i32];

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

/// Oscillator shape of a [`Voice`] (mirrors Web Audio's basic waveforms; the
/// synthesizer maps it to `OscillatorType`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wave {
    Sine,
    Triangle,
    Square,
    Sawtooth,
}

/// One melodic instrument of a song: what a lane's notes are synthesized
/// with. Cheap on purpose — a voice is baked once per pitch it plays.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Voice {
    /// Oscillator shape.
    pub wave: Wave,
    /// Stereo position of the lane, `-1.0` (hard left) … `1.0` (hard right).
    pub pan: f64,
    /// Unison detune in cents: `> 0` doubles the oscillator into a pair at
    /// `±detune` (the fat / supersaw-ish thickness of synthwave pads and
    /// leads); `0` = a single oscillator.
    pub detune: f64,
    /// Stereo WIDTH of the unison pair, `0.0` (both at the lane's pan) …
    /// `1.0` (spread hard left / right around it). Only with `detune > 0`;
    /// a wide voice bakes to a stereo buffer.
    pub width: f64,
}

impl Voice {
    /// A single centred oscillator — the pre-stereo sound of every song.
    pub const fn mono(wave: Wave) -> Self {
        Self {
            wave,
            pan: 0.0,
            detune: 0.0,
            width: 0.0,
        }
    }

    /// A single oscillator placed at `pan`.
    pub const fn panned(wave: Wave, pan: f64) -> Self {
        Self {
            wave,
            pan,
            detune: 0.0,
            width: 0.0,
        }
    }

    /// A detuned unison pair (`detune` cents) at `pan`, spread by `width`.
    pub const fn wide(wave: Wave, pan: f64, detune: f64, width: f64) -> Self {
        Self {
            wave,
            pan,
            detune,
            width,
        }
    }
}

/// One step of a percussion lane (`drums` / `perc`). Rendered from
/// synthesized noise/tones only.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Drum {
    /// No percussion this step.
    Silent,
    /// Pitched sine thump + a lick of low noise. Drives the side-chain.
    Kick,
    /// Very short high-passed noise tick (closed hat).
    Hat,
    /// Noise burst + a short body tone on the backbeat.
    Snare,
    /// Three tight noise slaps and a short tail — the 808-style hand clap
    /// that layers a snare or answers it.
    Clap,
    /// A longer, sizzling high-passed noise — the open hat on the off-beat.
    OpenHat,
    /// A pitched tom: sine dropping an octave with a knock on top.
    Tom,
    /// Rimshot / click: a tiny bright ping.
    Rim,
    /// A long bright wash — the crash on a downbeat.
    Crash,
}
use Drum::{Clap, Crash, Hat, Kick, OpenHat, Rim, Silent, Snare, Tom};

impl Drum {
    /// Every sounding drum, in bake order (the four-on-the-floor core first).
    pub const KIT: [Drum; 8] = [Kick, Hat, Snare, Clap, OpenHat, Tom, Rim, Crash];
}

/// One block of an arrangement: a self-contained, multi-bar pattern across all
/// five channels. Songs are built by ordering these (a refrain section can be
/// listed several times so the hook comes back). A section's playable length is
/// the length of its longest lane; shorter lanes loop within it.
///
/// Author a section with `..Section::EMPTY` so lanes you don't write (the
/// velocity lanes, typically) default to empty.
#[derive(Clone, Copy)]
pub struct Section {
    /// Human-readable role (intro / verse / refrain / bridge / outro). Purely
    /// documentation + exposed via the tracker API; the scheduler ignores it.
    pub label: &'static str,
    /// Bass lane, one scale-degree (or `REST` / `HOLD`) per step.
    pub bass: &'static [i32],
    /// Lead/melody lane, one scale-degree (or `REST` / `HOLD`) per step.
    pub lead: &'static [i32],
    /// Pad/chord lane: each note blooms into a slow triad; `HOLD` sustains it.
    pub pad: &'static [i32],
    /// Arp lane — a faster, higher counter-melody.
    pub arp: &'static [i32],
    /// Percussion lane, one `Drum` per step.
    pub drums: &'static [Drum],
    /// Second percussion lane (same kit) — for what has to hit together.
    pub perc: &'static [Drum],
    /// Velocity lanes (`0..=MAX_VEL` per step, looping; empty = all full).
    pub bass_vel: &'static [u8],
    pub lead_vel: &'static [u8],
    pub pad_vel: &'static [u8],
    pub arp_vel: &'static [u8],
    pub drums_vel: &'static [u8],
    pub perc_vel: &'static [u8],
}

impl Section {
    /// The all-empty section: the `..Section::EMPTY` base of every literal.
    pub const EMPTY: Section = Section {
        label: "",
        bass: &[],
        lead: &[],
        pad: &[],
        arp: &[],
        drums: &[],
        perc: &[],
        bass_vel: &[],
        lead_vel: &[],
        pad_vel: &[],
        arp_vel: &[],
        drums_vel: &[],
        perc_vel: &[],
    };

    /// The note lane of melodic channel `lane` ([`BASS`] … [`ARP`]); empty
    /// for the drums or an unknown index.
    pub fn lane(&self, lane: usize) -> &'static [i32] {
        match lane {
            BASS => self.bass,
            LEAD => self.lead,
            PAD => self.pad,
            ARP => self.arp,
            _ => &[],
        }
    }

    /// The percussion lane of channel `lane` ([`DRUMS`] / [`PERC`]); empty
    /// for a melodic channel.
    pub fn drum_lane(&self, lane: usize) -> &'static [Drum] {
        match lane {
            DRUMS => self.drums,
            PERC => self.perc,
            _ => &[],
        }
    }

    /// The velocity lane of channel `lane` (all six).
    pub fn vel_lane(&self, lane: usize) -> &'static [u8] {
        match lane {
            BASS => self.bass_vel,
            LEAD => self.lead_vel,
            PAD => self.pad_vel,
            ARP => self.arp_vel,
            DRUMS => self.drums_vel,
            PERC => self.perc_vel,
            _ => &[],
        }
    }
}

/// The music bus's SIDE-CHAIN ducker: every kick pulls the melodic lanes
/// down by `depth` in ~4 ms and lets them swell back with an exponential
/// release — the pumping that glues a synthwave mix to its four-on-the-floor.
/// The drums themselves are never ducked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sidechain {
    /// How far the lanes drop on a kick, `0.0` (off) … `1.0` (to silence).
    pub depth: f64,
    /// Recovery time in BEATS (tempo-synced): the lanes are ~95 % back this
    /// long after the kick. `1.0` = a full beat of pump.
    pub release_beats: f64,
}

impl Sidechain {
    /// No ducking.
    pub const OFF: Sidechain = Sidechain {
        depth: 0.0,
        release_beats: 1.0,
    };

    /// A ducker of `depth` recovering over `release_beats`.
    pub const fn new(depth: f64, release_beats: f64) -> Self {
        Self {
            depth,
            release_beats,
        }
    }

    /// Whether the ducker does anything.
    pub fn active(&self) -> bool {
        self.depth > 0.0
    }
}

/// The ducker's gain `dt` seconds into its exponential recovery (time
/// constant `tau`), having dropped to `1 - depth` at `dt = 0`.
pub fn duck_level(depth: f64, tau: f64, dt: f64) -> f64 {
    if tau <= 0.0 {
        return 1.0;
    }
    (1.0 - depth * (-dt.max(0.0) / tau).exp()).clamp(0.0, 1.0)
}

/// How late step `step` fires under `swing` (0 = straight … 1 = full
/// triplet shuffle): every odd sixteenth is delayed by up to a third of a
/// step, the even ones stay on the grid.
pub fn swing_delay(swing: f64, step: usize, step_dur: f64) -> f64 {
    if step % 2 == 1 {
        swing.clamp(0.0, 1.0) * step_dur / 3.0
    } else {
        0.0
    }
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
    /// The four melodic instruments, indexed by lane ([`BASS`], [`LEAD`],
    /// [`PAD`], [`ARP`]): oscillator shape, stereo position, unison detune.
    pub voices: [Voice; 4],
    /// The arrangement: an ordered list of sections played back to back, then
    /// looped as a whole. This is what makes a song long and developing.
    pub sections: &'static [Section],
    /// Overall punch/loudness feel (~0.5 lounge .. ~1.2 boss).
    pub intensity: f64,
    /// Shuffle, `0.0` (straight sixteenths) … `1.0` (full triplet swing):
    /// see [`swing_delay`].
    pub swing: f64,
    /// The kick-driven ducker on the melodic lanes ([`Sidechain::OFF`] = none).
    pub sidechain: Sidechain,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const INSERT_COIN: SongSpec = SongSpec {
    name: "Insert Coin",
    root: 55.0, // A1
    scale: MINOR,
    bpm: 84.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Triangle),                // bass
        Voice::panned(Wave::Sine, 0.2),             // lead
        Voice::wide(Wave::Triangle, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Sine, -0.3),            // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const NEON_LOUNGE: SongSpec = SongSpec {
    name: "Neon Lounge",
    root: 55.0, // A1
    scale: MINOR,
    bpm: 108.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Triangle),                // bass
        Voice::panned(Wave::Triangle, 0.2),         // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Triangle, -0.3),        // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const CHROME_VEINS: SongSpec = SongSpec {
    name: "Chrome Veins",
    root: 61.74, // B1
    scale: DORIAN,
    bpm: 118.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Square),                  // bass
        Voice::panned(Wave::Sawtooth, 0.2),         // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Square, -0.3),          // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const DESCENT: SongSpec = SongSpec {
    name: "Descent",
    root: 36.71, // D1
    scale: PHRYGIAN,
    bpm: 132.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Square),                  // bass
        Voice::panned(Wave::Sawtooth, 0.2),         // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Square, -0.3),          // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const BLOOD_RUSH: SongSpec = SongSpec {
    name: "Blood Rush",
    root: 46.25, // F#1
    scale: HARMONIC_MINOR,
    bpm: 140.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Sawtooth),                // bass
        Voice::panned(Wave::Square, 0.2),           // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Sawtooth, -0.3),        // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const DEEP_STATIC: SongSpec = SongSpec {
    name: "Deep Static",
    root: 41.20, // E1
    scale: PHRYGIAN_DOMINANT,
    bpm: 144.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Sawtooth),                // bass
        Voice::panned(Wave::Sawtooth, 0.2),         // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Square, -0.3),          // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const STATIC_PRAYER: SongSpec = SongSpec {
    name: "Static Prayer",
    root: 49.00, // G1
    scale: LOCRIAN,
    bpm: 92.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Sawtooth),                // bass
        Voice::panned(Wave::Triangle, 0.2),         // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Triangle, -0.3),        // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
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
    ..Section::EMPTY
};

const MASK_OF_DREAD: SongSpec = SongSpec {
    name: "Mask of Dread",
    root: 32.70, // C1
    scale: LOCRIAN,
    bpm: 100.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Sawtooth),                // bass
        Voice::panned(Wave::Square, 0.2),           // lead
        Voice::wide(Wave::Sawtooth, 0.0, 7.0, 0.7), // pad
        Voice::panned(Wave::Square, -0.3),          // arp
    ],
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
    swing: 0.0,
    sidechain: Sidechain::OFF,
};

// ---------------------------------------------------------------------------
// SONG 9 — "Sodium Lights" (WAVY / driving): the slow-burn night drive. D
// minor, i–VI–III–VII, a wide detuned saw pad held a bar per chord, a
// side-chain-pumped bass (retriggered sixteenths under a rising velocity
// ramp), a tied square lead and an accented arp. Also the showcase of the
// format's ties (`HOLD`), velocity lanes and stereo voices.
// ---------------------------------------------------------------------------

/// A bar of pad chord: struck once, held twelve steps, released for four.
const SODIUM_PAD: &[i32] = &[
    0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST, 5,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST, 2,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST, 6,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST,
];
/// The pumped bass: every sixteenth retriggers the chord root, the velocity
/// ramp ducking on each kick and swelling back before the next.
const SODIUM_BASS: &[i32] = &[
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2,
    -2, -2, -2, -2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1,
];
const SODIUM_PUMP: &[u8] = &[3, 6, 8, 9];
const SODIUM_DRUMS: &[Drum] = &[
    Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
    Snare, Silent, Silent, Silent,
];
/// Off-beat closed hats riding over the kicks, a clap under each snare, an
/// open hat pushing into the next bar.
const SODIUM_PERC: &[Drum] = &[
    Hat, Silent, Hat, Silent, Clap, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Clap, Silent,
    OpenHat, Silent,
];
const SODIUM_PERC_VEL: &[u8] = &[4, 0, 6, 0, 8, 0, 6, 0, 4, 0, 6, 0, 8, 0, 7, 0];

const SODIUM_INTRO: Section = Section {
    label: "intro",
    pad: SODIUM_PAD,
    arp: &[
        REST, REST, REST, REST, 14, REST, 16, REST, REST, REST, REST, REST, 18, REST, 16, REST,
    ],
    arp_vel: &[9, 0, 5, 0],
    drums: &[
        Silent, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Silent, Silent, Hat, Silent,
        Silent, Silent, Hat, Hat,
    ],
    drums_vel: &[0, 0, 5, 0, 0, 0, 5, 0, 0, 0, 5, 0, 0, 0, 5, 3],
    ..Section::EMPTY
};
const SODIUM_VERSE: Section = Section {
    label: "verse",
    bass: SODIUM_BASS,
    bass_vel: SODIUM_PUMP,
    lead: &[
        11, HOLD, HOLD, HOLD, HOLD, HOLD, 9, HOLD, 7, HOLD, HOLD, HOLD, REST, REST, REST, REST, 12,
        HOLD, HOLD, HOLD, HOLD, HOLD, 11, HOLD, 9, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, 9,
        HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD, 13, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 14,
        HOLD, HOLD, HOLD, HOLD, HOLD, 13, HOLD, 11, HOLD, HOLD, HOLD, REST, REST, REST, REST,
    ],
    lead_vel: &[
        8, 9, 9, 9, 9, 9, 6, 9, 7, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 6, 9, 7, 9, 9, 9, 9, 9,
        9, 9, 7, 9, 9, 9, 8, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 7, 9, 6, 9, 9, 9,
        9, 9, 9, 9,
    ],
    pad: SODIUM_PAD,
    drums: SODIUM_DRUMS,
    perc: SODIUM_PERC,
    perc_vel: SODIUM_PERC_VEL,
    ..Section::EMPTY
};
const SODIUM_REFRAIN: Section = Section {
    label: "refrain",
    bass: SODIUM_BASS,
    bass_vel: SODIUM_PUMP,
    lead: &[
        14, HOLD, HOLD, 13, HOLD, HOLD, 11, HOLD, 9, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD, 12,
        HOLD, HOLD, 14, HOLD, HOLD, 12, HOLD, 11, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 13,
        HOLD, HOLD, 14, HOLD, HOLD, 16, HOLD, 14, HOLD, HOLD, HOLD, 13, HOLD, HOLD, HOLD, 14, HOLD,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD, REST, REST, REST, REST,
    ],
    pad: SODIUM_PAD,
    arp: &[
        14, 16, 18, 16, 14, 16, 18, 21, 14, 16, 18, 16, 21, 18, 16, 14, 12, 14, 16, 14, 12, 14, 16,
        19, 12, 14, 16, 14, 19, 16, 14, 12, 16, 18, 20, 18, 16, 18, 20, 23, 16, 18, 20, 18, 23, 20,
        18, 16, 13, 15, 17, 15, 13, 15, 17, 20, 13, 15, 17, 15, 20, 17, 15, 13,
    ],
    arp_vel: &[9, 5, 7, 5, 8, 5, 7, 6],
    drums: SODIUM_DRUMS,
    perc: SODIUM_PERC,
    perc_vel: SODIUM_PERC_VEL,
    ..Section::EMPTY
};
const SODIUM_BREAK: Section = Section {
    label: "break",
    bass: &[
        0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, -2, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, 2, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, HOLD, -1, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, HOLD, HOLD,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, REST, REST, 7, HOLD, HOLD, HOLD, 9, HOLD, HOLD, HOLD,
        11, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, REST, REST, REST, REST, REST, REST, REST, REST, 9, HOLD, HOLD, HOLD, 11, HOLD, HOLD,
        HOLD, 13, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD,
    ],
    lead_vel: &[6],
    pad: SODIUM_PAD,
    drums: &[
        Kick, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Silent, Silent, Hat, Silent,
    ],
    drums_vel: &[7, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 4, 0],
    ..Section::EMPTY
};

const SODIUM_LIGHTS: SongSpec = SongSpec {
    name: "Sodium Lights",
    root: 73.42, // D2
    scale: MINOR,
    bpm: 96.0,
    steps_per_beat: 4,
    voices: [
        Voice::mono(Wave::Sawtooth),                  // bass: a centred sub
        Voice::wide(Wave::Square, 0.25, 6.0, 0.3),    // lead: a little right, gently doubled
        Voice::wide(Wave::Sawtooth, 0.0, 14.0, 0.85), // pad: the wide detuned saw bed
        Voice::panned(Wave::Triangle, -0.35),         // arp: answering from the left
    ],
    sections: &[
        SODIUM_INTRO,
        SODIUM_VERSE,
        SODIUM_REFRAIN,
        SODIUM_VERSE,
        SODIUM_REFRAIN,
        SODIUM_BREAK,
        SODIUM_REFRAIN,
        SODIUM_REFRAIN,
    ],
    intensity: 0.8,
    swing: 0.0,
    sidechain: Sidechain::new(0.55, 0.9),
};

/// All songs, in ascending darkness (intro first). Index into this with
/// `play_song`, or map a floor number through `song_for_floor`.
pub const SONGS: &[SongSpec] = &[
    INSERT_COIN,
    NEON_LOUNGE,
    SODIUM_LIGHTS,
    CHROME_VEINS,
    DESCENT,
    BLOOD_RUSH,
    DEEP_STATIC,
    STATIC_PRAYER,
    MASK_OF_DREAD,
];

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

// --- reading the format ------------------------------------------------------

/// Resolve a scale-degree (root = 0, +1 = next scale note up, +scale.len() = an
/// octave up, negatives drop below root) to a frequency in Hz, in-key.
pub fn degree_freq(root: f64, scale: Scale, degree: i32) -> f64 {
    if scale.is_empty() {
        return root;
    }
    let n = scale.len() as i32;
    let octave = degree.div_euclid(n);
    let idx = degree.rem_euclid(n) as usize;
    let semitones = octave * 12 + scale[idx];
    root * 2f64.powf(semitones as f64 / 12.0)
}

/// A note starting at some step of a melodic lane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NoteOn {
    /// Scale degree.
    pub degree: i32,
    /// Length in steps: 1 + the `HOLD`s tied onto it.
    pub len: u16,
}

/// Read a melodic lane at `step` (patterns loop): `Some` only where a note
/// STARTS — a `REST`, a `HOLD` (the tail of an earlier note) or an empty
/// lane is `None`. The length counts the `HOLD`s that follow, wrapping
/// around the looping lane, so a note tied across the lane's end sustains
/// into its next repeat.
pub fn note_at(pattern: &[i32], step: usize) -> Option<NoteOn> {
    if pattern.is_empty() {
        return None;
    }
    let n = pattern.len();
    let degree = pattern[step % n];
    if degree == REST || degree == HOLD {
        return None;
    }
    let mut len = 1;
    while len < n && pattern[(step + len) % n] == HOLD {
        len += 1;
    }
    Some(NoteOn {
        degree,
        len: len.min(u16::MAX as usize) as u16,
    })
}

/// Read a velocity lane at `step` (loops): `MAX_VEL` for an empty lane,
/// otherwise the step's value clamped to `MAX_VEL`.
pub fn vel_at(vels: &[u8], step: usize) -> u8 {
    if vels.is_empty() {
        return MAX_VEL;
    }
    vels[step % vels.len()].min(MAX_VEL)
}

/// Read the drum lane at `step` (loops). Empty lane == `Silent`.
pub fn drum_at(pattern: &[Drum], step: usize) -> Drum {
    if pattern.is_empty() {
        return Silent;
    }
    pattern[step % pattern.len()]
}

/// The playable length of a section: its longest note lane (shorter lanes
/// loop inside it). Always at least 1 so the scheduler can never divide by
/// zero. Velocity lanes don't count — they only decorate the notes.
pub fn section_len(sec: &Section) -> usize {
    sec.bass
        .len()
        .max(sec.lead.len())
        .max(sec.pad.len())
        .max(sec.arp.len())
        .max(sec.drums.len())
        .max(sec.perc.len())
        .max(1)
}

/// What a tracker cell shows for one channel at one step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    /// Nothing sounds (a rest, or an empty lane).
    Off,
    /// A note (or drum) starts here, at this velocity.
    On(u8),
    /// A note started earlier is tied through this step.
    Hold,
}

/// Sample the tracker cell of `channel` at `step` within `sec`.
pub fn cell_at(sec: &Section, channel: usize, step: usize) -> Cell {
    if channel == DRUMS || channel == PERC {
        return match drum_at(sec.drum_lane(channel), step) {
            Silent => Cell::Off,
            _ => Cell::On(vel_at(sec.vel_lane(channel), step)),
        };
    }
    let lane = sec.lane(channel);
    if lane.is_empty() {
        return Cell::Off;
    }
    match lane[step % lane.len()] {
        REST => Cell::Off,
        HOLD => {
            // A HOLD only sustains if some note precedes it in the loop.
            if lane.iter().any(|&d| d != REST && d != HOLD) {
                Cell::Hold
            } else {
                Cell::Off
            }
        }
        _ => Cell::On(vel_at(sec.vel_lane(channel), step)),
    }
}

/// One pre-renderable music voice: what the synthesizer bakes once per song
/// and fires per scheduled note. Velocity is NOT part of the key (it is a
/// playback gain); pitch AND length are (the envelope is baked in).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MusicKey {
    /// A melodic lane ([`BASS`] … [`ARP`]) note at this scale degree, this
    /// many steps long (1 = untied).
    Note { lane: usize, degree: i32, len: u16 },
    /// One kit piece (never `Silent`) — shared by both percussion lanes.
    Drum(Drum),
}

/// Enumerate the exact, finite voice set `song` can ever schedule: the
/// distinct (degree, length) pairs of each melodic lane across every
/// section, plus the kit pieces its percussion lanes use — in bake-priority
/// order (drums first — the densest lanes — then bass, lead, arp, pad).
/// Typically 30–50 keys per song.
pub fn music_keys(song: &SongSpec) -> Vec<MusicKey> {
    fn add(keys: &mut Vec<MusicKey>, k: MusicKey) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut keys = Vec::new();
    // Drums in kit order (kick first), only the pieces the song uses.
    for drum in Drum::KIT {
        let used = song
            .sections
            .iter()
            .any(|sec| sec.drums.contains(&drum) || sec.perc.contains(&drum));
        if used {
            add(&mut keys, MusicKey::Drum(drum));
        }
    }
    for lane in [BASS, LEAD, ARP, PAD] {
        for sec in song.sections {
            let pattern = sec.lane(lane);
            for step in 0..pattern.len() {
                if let Some(n) = note_at(pattern, step) {
                    add(
                        &mut keys,
                        MusicKey::Note {
                            lane,
                            degree: n.degree,
                            len: n.len,
                        },
                    );
                }
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degrees_resolve_in_key() {
        let a1 = 55.0;
        assert!((degree_freq(a1, MINOR, 0) - 55.0).abs() < 1e-9);
        assert!((degree_freq(a1, MINOR, 7) - 110.0).abs() < 1e-9);
        assert!((degree_freq(a1, MINOR, -7) - 27.5).abs() < 1e-9);
        // Degree 2 of A minor is C (3 semitones up).
        assert!((degree_freq(a1, MINOR, 2) - 55.0 * 2f64.powf(0.25)).abs() < 1e-9);
        assert_eq!(degree_freq(a1, &[], 5), a1);
    }

    #[test]
    fn ties_extend_the_note_they_follow() {
        let lane = [0, HOLD, HOLD, HOLD, 3, REST, HOLD, 5];
        assert_eq!(note_at(&lane, 0), Some(NoteOn { degree: 0, len: 4 }));
        assert_eq!(note_at(&lane, 1), None, "a HOLD is not a note start");
        assert_eq!(note_at(&lane, 4), Some(NoteOn { degree: 3, len: 1 }));
        assert_eq!(note_at(&lane, 5), None);
        assert_eq!(note_at(&lane, 6), None, "a HOLD after a REST is silent");
        // Counting wraps around the lane but stops at the next note start
        // (step 0 here), so the last note is one step long.
        assert_eq!(note_at(&lane, 7), Some(NoteOn { degree: 5, len: 1 }));
        // Wrapping tie: a note at the end sustains into the lane's repeat.
        let wrap = [HOLD, HOLD, 2, 4];
        assert_eq!(note_at(&wrap, 3), Some(NoteOn { degree: 4, len: 3 }));
        // Steps beyond the lane length loop.
        assert_eq!(note_at(&wrap, 7), note_at(&wrap, 3));
        assert_eq!(note_at(&[], 0), None);
        // An all-HOLD lane never sounds (and never loops forever counting).
        assert_eq!(note_at(&[HOLD, HOLD], 0), None);
    }

    #[test]
    fn velocity_lanes_loop_and_default_to_full() {
        assert_eq!(vel_at(&[], 12), MAX_VEL);
        assert_eq!(vel_at(&[3, 6, 8, 9], 0), 3);
        assert_eq!(vel_at(&[3, 6, 8, 9], 7), 9);
        assert_eq!(vel_at(&[200], 0), MAX_VEL, "clamped");
    }

    #[test]
    fn tracker_cells_reflect_notes_ties_and_velocity() {
        let sec = Section {
            lead: &[7, HOLD, REST, 9],
            lead_vel: &[9, 9, 9, 4],
            drums: &[Kick, Silent],
            drums_vel: &[6],
            ..Section::EMPTY
        };
        assert_eq!(cell_at(&sec, LEAD, 0), Cell::On(9));
        assert_eq!(cell_at(&sec, LEAD, 1), Cell::Hold);
        assert_eq!(cell_at(&sec, LEAD, 2), Cell::Off);
        assert_eq!(cell_at(&sec, LEAD, 3), Cell::On(4));
        assert_eq!(cell_at(&sec, BASS, 0), Cell::Off, "empty lane");
        assert_eq!(cell_at(&sec, DRUMS, 0), Cell::On(6));
        assert_eq!(cell_at(&sec, DRUMS, 1), Cell::Off);
        assert_eq!(cell_at(&sec, PERC, 0), Cell::Off, "empty perc lane");
        let orphan = Section {
            arp: &[HOLD, HOLD],
            ..Section::EMPTY
        };
        assert_eq!(cell_at(&orphan, ARP, 1), Cell::Off);
    }

    #[test]
    fn the_kit_lists_every_sounding_drum_once() {
        assert!(!Drum::KIT.contains(&Silent));
        for (i, d) in Drum::KIT.iter().enumerate() {
            assert!(!Drum::KIT[..i].contains(d), "{d:?} twice");
        }
        // A song using every piece on either lane enumerates all of them.
        const SEC: Section = Section {
            drums: &[Kick, Hat, Snare, Clap],
            perc: &[OpenHat, Tom, Rim, Crash],
            ..Section::EMPTY
        };
        let song = SongSpec {
            sections: &[SEC],
            ..SONGS[0]
        };
        let keys = music_keys(&song);
        for d in Drum::KIT {
            assert!(keys.contains(&MusicKey::Drum(d)), "{d:?}");
        }
        assert_eq!(cell_at(&SEC, PERC, 3), Cell::On(MAX_VEL));
        assert_eq!(section_len(&SEC), 4);
    }

    #[test]
    fn section_len_is_the_longest_note_lane() {
        assert_eq!(section_len(&Section::EMPTY), 1);
        let sec = Section {
            bass: &[0; 16],
            lead: &[REST; 32],
            bass_vel: &[9; 64],
            ..Section::EMPTY
        };
        assert_eq!(section_len(&sec), 32);
    }

    /// Every song must be well-formed data: a non-empty arrangement, sane
    /// tempo / resolution, a non-empty scale, velocities within range, and
    /// no `HOLD` that has nothing to hold.
    #[test]
    fn songs_are_well_formed() {
        for song in SONGS {
            assert!(!song.sections.is_empty(), "{}: no sections", song.name);
            assert!(song.bpm > 0.0 && song.steps_per_beat > 0, "{}", song.name);
            assert!(!song.scale.is_empty(), "{}", song.name);
            assert!(song.root > 0.0, "{}", song.name);
            assert!((0.0..=1.0).contains(&song.swing), "{}: swing", song.name);
            assert!(
                (0.0..=1.0).contains(&song.sidechain.depth),
                "{}: duck",
                song.name
            );
            assert!(
                song.sidechain.release_beats > 0.0,
                "{}: duck release",
                song.name
            );
            for v in song.voices {
                assert!((-1.0..=1.0).contains(&v.pan), "{}: pan", song.name);
                assert!((0.0..=1.0).contains(&v.width), "{}: width", song.name);
                assert!(v.detune >= 0.0, "{}: detune", song.name);
            }
            for sec in song.sections {
                for lane in [BASS, LEAD, PAD, ARP] {
                    let p = sec.lane(lane);
                    // An all-REST lane is fine (it pads the section's
                    // length); a HOLD with no note anywhere to hold is a typo.
                    if p.contains(&HOLD) {
                        assert!(
                            p.iter().any(|&d| d != REST && d != HOLD),
                            "{} / {}: lane {} ties nothing",
                            song.name,
                            sec.label,
                            CHANNEL_NAMES[lane]
                        );
                    }
                }
                for ch in 0..NUM_CHANNELS {
                    for &v in sec.vel_lane(ch) {
                        assert!(
                            v <= MAX_VEL,
                            "{} / {}: velocity {}",
                            song.name,
                            sec.label,
                            v
                        );
                    }
                }
            }
        }
        // Song names double as the engine's change-detection key.
        for (i, a) in SONGS.iter().enumerate() {
            assert!(!SONGS[..i].iter().any(|b| b.name == a.name), "{}", a.name);
        }
    }

    /// Every note any song can ever schedule must map to an enumerated
    /// [`MusicKey`], and the per-song voice set must stay small enough that
    /// baking each exact pitch × length (no `playback_rate` transposition)
    /// is cheap. Run with `--nocapture` to see the per-song counts.
    #[test]
    fn music_voice_sets_are_small_and_complete() {
        for song in SONGS {
            let keys = music_keys(song);
            assert!(!keys.is_empty(), "{}: empty voice set", song.name);
            assert!(
                keys.len() <= 96,
                "{}: {} voices — too many to bake each exact pitch",
                song.name,
                keys.len()
            );
            for (i, k) in keys.iter().enumerate() {
                assert!(!keys[..i].contains(k), "{}: duplicate {:?}", song.name, k);
            }
            for sec in song.sections {
                for lane in [BASS, LEAD, PAD, ARP] {
                    let p = sec.lane(lane);
                    for step in 0..p.len() {
                        if let Some(n) = note_at(p, step) {
                            let key = MusicKey::Note {
                                lane,
                                degree: n.degree,
                                len: n.len,
                            };
                            assert!(keys.contains(&key), "{}: missing {:?}", song.name, key);
                        }
                    }
                }
                for &dr in sec.drums.iter().chain(sec.perc) {
                    if dr != Silent {
                        assert!(
                            keys.contains(&MusicKey::Drum(dr)),
                            "{}: {:?}",
                            song.name,
                            dr
                        );
                    }
                }
            }
            let count = |f: fn(&MusicKey) -> bool| keys.iter().filter(|k| f(k)).count();
            println!(
                "{:14} {:2} voices (drums {} bass {:2} lead {:2} arp {:2} pad {:2})",
                song.name,
                keys.len(),
                count(|k| matches!(k, MusicKey::Drum(_))),
                count(|k| matches!(k, MusicKey::Note { lane: BASS, .. })),
                count(|k| matches!(k, MusicKey::Note { lane: LEAD, .. })),
                count(|k| matches!(k, MusicKey::Note { lane: ARP, .. })),
                count(|k| matches!(k, MusicKey::Note { lane: PAD, .. })),
            );
        }
    }

    #[test]
    fn duck_level_recovers_exponentially() {
        assert!((duck_level(0.6, 0.2, 0.0) - 0.4).abs() < 1e-9);
        let mid = duck_level(0.6, 0.2, 0.2);
        assert!(mid > 0.4 && mid < 1.0);
        assert!(duck_level(0.6, 0.2, 5.0) > 0.999);
        assert_eq!(duck_level(0.6, 0.0, 0.1), 1.0, "no time constant = no duck");
        assert!(
            (duck_level(0.6, 0.2, -1.0) - 0.4).abs() < 1e-9,
            "before = at"
        );
    }

    #[test]
    fn swing_delays_only_the_off_sixteenths() {
        assert_eq!(swing_delay(0.0, 1, 0.1), 0.0);
        assert_eq!(swing_delay(1.0, 0, 0.1), 0.0);
        assert!((swing_delay(1.0, 1, 0.3) - 0.1).abs() < 1e-9);
        assert!((swing_delay(0.5, 3, 0.3) - 0.05).abs() < 1e-9);
        assert!((swing_delay(7.0, 1, 0.3) - 0.1).abs() < 1e-9, "clamped");
    }

    #[test]
    fn floors_escalate_without_gaps() {
        for floor in 0..20 {
            let s = song_for_floor(floor);
            assert!(SONGS.iter().any(|x| x.name == s.name), "floor {floor}");
        }
        assert!(song_for_floor(0).intensity <= song_for_floor(14).intensity);
    }
}
