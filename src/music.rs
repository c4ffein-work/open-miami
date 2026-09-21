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
pub const NUM_CHANNELS: usize = 7;
/// Channel (lane) indices, `0..NUM_CHANNELS`.
pub const BASS: usize = 0;
pub const LEAD: usize = 1;
pub const PAD: usize = 2;
pub const ARP: usize = 3;
/// The fifth melodic lane: chord stabs, a second lead, a counter-line —
/// whatever the four classic lanes leave no room for.
pub const KEYS: usize = 4;
pub const DRUMS: usize = 5;
/// The second percussion lane: same kit as `DRUMS`, so a hat can ride over
/// a kick, a clap can layer a snare, a crash can top a downbeat.
pub const PERC: usize = 6;
/// The melodic lanes, in bake-priority order (densest / most exposed first).
pub const MELODIC: [usize; 5] = [BASS, LEAD, ARP, KEYS, PAD];
/// How many melodic lanes there are (= `SongSpec::voices.len()`).
pub const NUM_VOICES: usize = MELODIC.len();

/// Human-readable channel names, indexed 0..[`NUM_CHANNELS`].
pub const CHANNEL_NAMES: [&str; NUM_CHANNELS] =
    ["BASS", "LEAD", "PAD", "ARP", "KEYS", "DRUMS", "PERC"];

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
    /// White noise instead of a pitched oscillator: the note's degree is
    /// ignored, its envelope and the voice's [`Filter`] shape the sound —
    /// a riser (long tie, slow filter attack), a snare-ish hit, wind.
    Noise,
}

/// How a lane note is voiced: which scale degrees sound, relative to the
/// written one. In-key by construction — a `Triad` on the 5th degree of a
/// minor scale is whatever chord the scale spells there — so voicings move
/// with the key like the notes do. The pad's default is `Triad`, every
/// other lane's is `Single`; a `*_chord` lane changes it per step.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Chord {
    /// Just the written degree.
    Single,
    /// Root + the octave above — the thick unison lead / bass.
    Octave,
    /// Root, fifth, octave: the power chord (no third; heavy).
    Power,
    /// Root, third, fifth in root position — the default chord bed.
    Triad,
    /// Root, second, fifth: suspended, open, unresolved.
    Sus2,
    /// Root, fourth, fifth: suspended, leaning to resolve.
    Sus4,
    /// Root, third, fifth, seventh: the lush four-note chord.
    Seventh,
    /// Root, third, fifth + the ninth on top: wide and dreamy.
    Add9,
    /// First inversion: third, fifth, root-up-an-octave (smoother voice
    /// leading between neighbouring chords).
    Inv1,
    /// Second inversion: fifth, root, third all up — bright and floating.
    Inv2,
    /// Root, fifth, tenth (the third an octave up): the wide-open voicing.
    Open,
}

impl Chord {
    /// The scale-degree offsets that sound, lowest first.
    pub const fn degrees(self) -> &'static [i32] {
        match self {
            Chord::Single => &[0],
            Chord::Octave => &[0, 7],
            Chord::Power => &[0, 4, 7],
            Chord::Triad => &[0, 2, 4],
            Chord::Sus2 => &[0, 1, 4],
            Chord::Sus4 => &[0, 3, 4],
            Chord::Seventh => &[0, 2, 4, 6],
            Chord::Add9 => &[0, 2, 4, 8],
            Chord::Inv1 => &[2, 4, 7],
            Chord::Inv2 => &[4, 7, 9],
            Chord::Open => &[0, 4, 9],
        }
    }

    /// The voicing a lane uses where its chord lane is empty.
    pub const fn default_for(lane: usize) -> Chord {
        if lane == PAD {
            Chord::Triad
        } else {
            Chord::Single
        }
    }
}

/// A voice's amplitude envelope, overriding the lane's built-in shape:
/// `attack` seconds up to peak, then (after the tied steps, held at peak)
/// an exponential decay to silence over `gate` STEPS. The lane defaults
/// are bass 1.9 / lead 0.9 / pad 4.0 / arp 0.7 / keys 1.2 steps of tail.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Env {
    pub attack: f64,
    pub gate: f64,
}

/// A per-note LOWPASS filter envelope — the "wow" of a synth stab and the
/// slow bloom of a pad both live here. The cutoff starts at `cutoff`, opens
/// to `peak` over `attack` seconds (instantly when `attack == 0`), then
/// falls back to `cutoff` over `decay` seconds (stays open when `decay ==
/// 0`). `q` is the resonance (0.7 flat … 8 screaming).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Filter {
    pub cutoff: f64,
    pub peak: f64,
    pub attack: f64,
    pub decay: f64,
    pub q: f64,
}

/// Pitch vibrato: a sine LFO at `rate` Hz, `depth` cents peak, fading in
/// over `delay` seconds after the note starts (the singer's late vibrato).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Vibrato {
    pub rate: f64,
    pub depth: f64,
    pub delay: f64,
}

/// One melodic instrument of a song: what a lane's notes are synthesized
/// with. Cheap on purpose — a voice is baked once per pitch it plays. Build
/// one with [`Voice::mono`] / [`Voice::panned`] / [`Voice::wide`] /
/// [`Voice::stack`] and refine it with the `with_*` builders.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Voice {
    /// Oscillator shape.
    pub wave: Wave,
    /// Stereo position of the lane, `-1.0` (hard left) … `1.0` (hard right).
    pub pan: f64,
    /// Unison detune in cents: with `unison >= 2` the oscillators spread
    /// evenly over `±detune` (the fat / supersaw thickness of synthwave
    /// pads and leads); `0` = every oscillator at pitch (a single one).
    pub detune: f64,
    /// Stereo WIDTH of the unison stack, `0.0` (all at the lane's pan) …
    /// `1.0` (spread hard left / right around it). Only with `detune > 0`;
    /// a wide voice bakes to a stereo buffer.
    pub width: f64,
    /// Oscillators per note, `1` … `7`. Two is the classic pair; five to
    /// seven is the supersaw.
    pub unison: u8,
    /// Lane-level soft-clip drive, `0.0` (clean) … `1.0` (crushed): a
    /// tanh waveshaper on the lane's summed output (chords intermodulate
    /// through it) that also lifts its quiet parts — density, grit, edge.
    pub drive: f64,
    /// Amplitude envelope override (`None` = the lane's built-in shape).
    pub env: Option<Env>,
    /// Per-note lowpass filter envelope (`None` = unfiltered).
    pub filter: Option<Filter>,
    /// Pitch vibrato (`None` = none).
    pub vibrato: Option<Vibrato>,
    /// Send level into the song's [`Echo`], `0.0` (dry) … `1.0`.
    pub echo: f64,
    /// Send level into the music hall reverb, `0.0` (dry) … `1.0`.
    pub reverb: f64,
    /// A sine SUB-oscillator one octave below the note's lowest partial, at
    /// this level relative to the note (`0.0` = none). Centred, unfiltered,
    /// undetuned: the weight under a bass.
    pub sub: f64,
    /// Portamento time in seconds: a note that starts exactly where the
    /// lane's previous note ends (legato — no rest between) GLIDES into
    /// its pitch from the previous one over this long. `0.0` = no glide.
    pub glide: f64,
}

impl Voice {
    /// A single centred oscillator — the pre-stereo sound of every song.
    pub const fn mono(wave: Wave) -> Self {
        Self {
            wave,
            pan: 0.0,
            detune: 0.0,
            width: 0.0,
            unison: 1,
            drive: 0.0,
            env: None,
            filter: None,
            vibrato: None,
            echo: 0.0,
            reverb: 0.0,
            sub: 0.0,
            glide: 0.0,
        }
    }

    /// A single oscillator placed at `pan`.
    pub const fn panned(wave: Wave, pan: f64) -> Self {
        Self {
            pan,
            ..Self::mono(wave)
        }
    }

    /// A detuned unison pair (`detune` cents) at `pan`, spread by `width`.
    pub const fn wide(wave: Wave, pan: f64, detune: f64, width: f64) -> Self {
        Self {
            pan,
            detune,
            width,
            unison: 2,
            ..Self::mono(wave)
        }
    }

    /// A unison STACK of `unison` oscillators spread over `±detune` cents
    /// and `±width` around `pan` — the supersaw.
    pub const fn stack(wave: Wave, pan: f64, detune: f64, width: f64, unison: u8) -> Self {
        Self {
            pan,
            detune,
            width,
            unison,
            ..Self::mono(wave)
        }
    }

    /// With an amplitude envelope override.
    pub const fn with_env(self, attack: f64, gate: f64) -> Self {
        Self {
            env: Some(Env { attack, gate }),
            ..self
        }
    }

    /// With a per-note lowpass filter envelope (see [`Filter`]).
    pub const fn with_filter(
        self,
        cutoff: f64,
        peak: f64,
        attack: f64,
        decay: f64,
        q: f64,
    ) -> Self {
        Self {
            filter: Some(Filter {
                cutoff,
                peak,
                attack,
                decay,
                q,
            }),
            ..self
        }
    }

    /// With pitch vibrato (see [`Vibrato`]).
    pub const fn with_vibrato(self, rate: f64, depth: f64, delay: f64) -> Self {
        Self {
            vibrato: Some(Vibrato { rate, depth, delay }),
            ..self
        }
    }

    /// With lane drive (see [`Voice::drive`]).
    pub const fn with_drive(self, drive: f64) -> Self {
        Self { drive, ..self }
    }

    /// With a send into the song's echo (see [`Echo`]).
    pub const fn with_echo(self, echo: f64) -> Self {
        Self { echo, ..self }
    }

    /// With a send into the hall reverb.
    pub const fn with_reverb(self, reverb: f64) -> Self {
        Self { reverb, ..self }
    }

    /// With a sine sub-oscillator an octave down at `sub` of the note.
    pub const fn with_sub(self, sub: f64) -> Self {
        Self { sub, ..self }
    }

    /// With legato portamento over `glide` seconds (see [`Voice::glide`]).
    pub const fn with_glide(self, glide: f64) -> Self {
        Self { glide, ..self }
    }

    /// How many oscillators a note of this voice actually runs: the stack
    /// only exists with a detune to spread it over.
    pub fn oscillators(&self) -> usize {
        if self.detune > 0.0 && self.wave != Wave::Noise {
            (self.unison.clamp(1, 7)) as usize
        } else {
            1
        }
    }

    /// Whether a note of this voice is a stereo image of its own (a spread
    /// stack), as opposed to a point the lane's panner places.
    pub fn is_wide(&self) -> bool {
        self.oscillators() > 1 && self.width > 0.0
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
/// seven channels. Songs are built by ordering these (a refrain section can be
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
    /// Keys lane — stabs, a second lead, a counter-line.
    pub keys: &'static [i32],
    /// Percussion lane, one `Drum` per step.
    pub drums: &'static [Drum],
    /// Second percussion lane (same kit) — for what has to hit together.
    pub perc: &'static [Drum],
    /// Velocity lanes (`0..=MAX_VEL` per step, looping; empty = all full).
    pub bass_vel: &'static [u8],
    pub lead_vel: &'static [u8],
    pub pad_vel: &'static [u8],
    pub arp_vel: &'static [u8],
    pub keys_vel: &'static [u8],
    pub drums_vel: &'static [u8],
    pub perc_vel: &'static [u8],
    /// Chord (voicing) lanes: one [`Chord`] per step, looping; empty = the
    /// lane's default ([`Chord::default_for`]). Read where a note STARTS.
    pub bass_chord: &'static [Chord],
    pub lead_chord: &'static [Chord],
    pub pad_chord: &'static [Chord],
    pub arp_chord: &'static [Chord],
    pub keys_chord: &'static [Chord],
}

impl Section {
    /// The all-empty section: the `..Section::EMPTY` base of every literal.
    pub const EMPTY: Section = Section {
        label: "",
        bass: &[],
        lead: &[],
        pad: &[],
        arp: &[],
        keys: &[],
        drums: &[],
        perc: &[],
        bass_vel: &[],
        lead_vel: &[],
        pad_vel: &[],
        arp_vel: &[],
        keys_vel: &[],
        drums_vel: &[],
        perc_vel: &[],
        bass_chord: &[],
        lead_chord: &[],
        pad_chord: &[],
        arp_chord: &[],
        keys_chord: &[],
    };

    /// The note lane of melodic channel `lane` ([`BASS`] … [`ARP`]); empty
    /// for the drums or an unknown index.
    pub fn lane(&self, lane: usize) -> &'static [i32] {
        match lane {
            BASS => self.bass,
            LEAD => self.lead,
            PAD => self.pad,
            ARP => self.arp,
            KEYS => self.keys,
            _ => &[],
        }
    }

    /// The chord lane of melodic channel `lane`; empty for the drums.
    pub fn chord_lane(&self, lane: usize) -> &'static [Chord] {
        match lane {
            BASS => self.bass_chord,
            LEAD => self.lead_chord,
            PAD => self.pad_chord,
            ARP => self.arp_chord,
            KEYS => self.keys_chord,
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
            KEYS => self.keys_vel,
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

/// The song's tempo-synced ECHO: one shared stereo delay line the lanes send
/// into ([`Voice::echo`]), its repeats fed back through a darkening
/// lowpass. Dotted-eighth repeats (`steps: 3.0` at four steps a beat) are
/// the synthwave lead / arp echo; a beat (`4.0`) the dub throw.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Echo {
    /// Delay time in STEPS (fractional allowed; clamped to 2 s).
    pub steps: f64,
    /// Repeat feedback, `0.0` (one repeat) … `0.9` (long trails).
    pub feedback: f64,
    /// Lowpass on the repeats, Hz (each repeat darker than the last).
    pub tone: f64,
}

impl Echo {
    /// Dotted-eighth repeats, three of them or so, slightly dark. Inert
    /// until a voice sends into it.
    pub const DOTTED: Echo = Echo {
        steps: 3.0,
        feedback: 0.35,
        tone: 3200.0,
    };

    /// A custom echo.
    pub const fn new(steps: f64, feedback: f64, tone: f64) -> Self {
        Self {
            steps,
            feedback,
            tone,
        }
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
    /// The five melodic instruments, indexed by lane ([`BASS`], [`LEAD`],
    /// [`PAD`], [`ARP`], [`KEYS`]).
    pub voices: [Voice; NUM_VOICES],
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
    /// The shared echo line the voices' `echo` sends feed.
    pub echo: Echo,
    /// Timing HUMANIZE: every note except the kicks lands up to this many
    /// seconds early or late (uniform; clamped to 20 ms). `0.0` = machine
    /// tight; 3–6 ms loosens a groove without smearing it.
    pub humanize: f64,
    /// Depth of the bus lowpass's once-per-bar sweep, `1.0` (the classic
    /// synthwave wah, closing to 420 Hz at the bar lines) … `0.0` (the bus
    /// filter stays open — for songs whose voices carry their own filter
    /// motion).
    pub sweep: f64,
}

/// A shared PERC ride for the driving songs' refrains: closed hats on the
/// off sixteenths (between the drum lane's own even-step hats — never
/// doubling them) and an open hat pushing into the next bar.
const PERC_RIDE: &[Drum] = &[
    Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat,
    Silent, OpenHat,
];
const PERC_RIDE_VEL: &[u8] = &[0, 4, 0, 3, 0, 4, 0, 3, 0, 4, 0, 3, 0, 4, 0, 5];

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
        // bass: a soft triangle sub
        Voice::mono(Wave::Triangle),
        // lead: a sine with a slow, late vibrato, echoing into the hall
        Voice::panned(Wave::Sine, 0.2)
            .with_vibrato(4.5, 8.0, 0.4)
            .with_echo(0.3)
            .with_reverb(0.35),
        // pad: three soft triangles blooming open over a second and a half
        Voice::stack(Wave::Triangle, 0.0, 7.0, 0.8, 3)
            .with_filter(600.0, 1800.0, 1.5, 0.0, 0.9)
            .with_reverb(0.5),
        // arp: a sine falling through dotted-eighth echoes
        Voice::panned(Wave::Sine, -0.3)
            .with_echo(0.4)
            .with_reverb(0.2),
        Voice::mono(Wave::Square), // keys: unused
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
    echo: Echo::DOTTED,
    humanize: 0.006,
    sweep: 1.0,
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
        // bass: a triangle with a soft, round pluck
        Voice::mono(Wave::Triangle).with_filter(200.0, 700.0, 0.0, 0.15, 2.0),
        // lead: a mellow triangle, light vibrato, a touch of echo and room
        Voice::panned(Wave::Triangle, 0.2)
            .with_vibrato(5.0, 10.0, 0.3)
            .with_echo(0.3)
            .with_reverb(0.2),
        // pad: a warm three-saw bed opening over a second
        Voice::stack(Wave::Sawtooth, 0.0, 8.0, 0.8, 3)
            .with_filter(500.0, 1600.0, 0.9, 0.0, 1.0)
            .with_reverb(0.4),
        // arp: a triangle with dotted-eighth echoes
        Voice::panned(Wave::Triangle, -0.3).with_echo(0.3),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.25, 0.8),
    echo: Echo::DOTTED,
    humanize: 0.005,
    sweep: 1.0,
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
    perc: PERC_RIDE,
    perc_vel: PERC_RIDE_VEL,
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
        // bass: a square with a resonant pluck and a little grit
        Voice::mono(Wave::Square)
            .with_filter(260.0, 1400.0, 0.0, 0.1, 3.5)
            .with_drive(0.25),
        // lead: a doubled saw with a filter wow, vibrato and echo
        Voice::wide(Wave::Sawtooth, 0.2, 8.0, 0.4)
            .with_filter(1500.0, 5000.0, 0.0, 0.2, 2.0)
            .with_vibrato(5.5, 10.0, 0.25)
            .with_echo(0.3),
        // pad: a five-saw supersaw blooming over most of a second
        Voice::stack(Wave::Sawtooth, 0.0, 10.0, 0.85, 5)
            .with_filter(650.0, 2200.0, 0.8, 0.0, 1.1)
            .with_reverb(0.4),
        // arp: a bright square with dotted-eighth echoes
        Voice::panned(Wave::Square, -0.3).with_echo(0.35),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.4, 0.8),
    echo: Echo::DOTTED,
    humanize: 0.0,
    sweep: 1.0,
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
    perc: PERC_RIDE,
    perc_vel: PERC_RIDE_VEL,
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
        // bass: a hammering square, tight resonant pluck, driven
        Voice::mono(Wave::Square)
            .with_filter(220.0, 1600.0, 0.0, 0.09, 4.0)
            .with_drive(0.35),
        // lead: a doubled saw, fast wow, nervous vibrato, a little drive
        Voice::wide(Wave::Sawtooth, 0.2, 9.0, 0.4)
            .with_filter(1200.0, 5500.0, 0.0, 0.15, 2.5)
            .with_vibrato(6.0, 14.0, 0.2)
            .with_echo(0.3)
            .with_drive(0.2),
        // pad: a wide supersaw, darker, opening over ~0.7 s
        Voice::stack(Wave::Sawtooth, 0.0, 12.0, 0.9, 5)
            .with_filter(500.0, 2000.0, 0.7, 0.0, 1.2)
            .with_reverb(0.45),
        // arp: a restless saw with echoes
        Voice::panned(Wave::Sawtooth, -0.3).with_echo(0.35),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.5, 0.7),
    echo: Echo::DOTTED,
    humanize: 0.0,
    sweep: 1.0,
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
    perc: PERC_RIDE,
    perc_vel: PERC_RIDE_VEL,
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
        // bass: a jagged saw, sharp pluck, driven hard
        Voice::mono(Wave::Sawtooth)
            .with_filter(240.0, 1800.0, 0.0, 0.08, 4.5)
            .with_drive(0.45),
        // lead: three squares wailing — wide vibrato, wow, drive, echo
        Voice::stack(Wave::Square, 0.2, 9.0, 0.5, 3)
            .with_filter(1400.0, 6000.0, 0.0, 0.14, 2.5)
            .with_vibrato(6.5, 18.0, 0.15)
            .with_echo(0.3)
            .with_drive(0.3),
        // pad: a supersaw bed with a quick half-second bloom
        Voice::stack(Wave::Sawtooth, 0.0, 12.0, 0.9, 5)
            .with_filter(550.0, 2400.0, 0.5, 0.0, 1.3)
            .with_reverb(0.4),
        // arp: a stabbing saw with echoes
        Voice::panned(Wave::Sawtooth, -0.3).with_echo(0.4),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.5, 0.6),
    echo: Echo::DOTTED,
    humanize: 0.0,
    sweep: 1.0,
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
    perc: PERC_RIDE,
    perc_vel: PERC_RIDE_VEL,
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
        // bass: a relentless saw, tight and heavily driven, a sine sub under it
        Voice::mono(Wave::Sawtooth)
            .with_filter(180.0, 1200.0, 0.0, 0.07, 5.0)
            .with_drive(0.55)
            .with_sub(0.4),
        // lead: a three-saw stack with a screaming resonant wow
        Voice::stack(Wave::Sawtooth, 0.2, 10.0, 0.5, 3)
            .with_filter(1000.0, 5000.0, 0.0, 0.12, 3.0)
            .with_vibrato(6.0, 16.0, 0.2)
            .with_echo(0.25)
            .with_drive(0.35),
        // pad: a dark, wide supersaw pressure bed
        Voice::stack(Wave::Sawtooth, 0.0, 14.0, 0.9, 5)
            .with_filter(400.0, 1800.0, 0.6, 0.0, 1.4)
            .with_reverb(0.45),
        // arp: dissonant square stabs, a little grit, echoes
        Voice::panned(Wave::Square, -0.3)
            .with_echo(0.35)
            .with_drive(0.2),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.55, 0.6),
    echo: Echo::DOTTED,
    humanize: 0.0,
    sweep: 1.0,
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
        // bass: a slow, dull saw lurch
        Voice::mono(Wave::Sawtooth).with_filter(150.0, 600.0, 0.0, 0.3, 1.5),
        // lead: a detuned, wide triangle wail with a slow deep vibrato,
        // sliding into each touching note
        Voice::wide(Wave::Triangle, 0.2, 10.0, 0.6)
            .with_glide(0.15)
            .with_vibrato(4.5, 20.0, 0.5)
            .with_echo(0.4)
            .with_reverb(0.5),
        // pad: a mournful supersaw drone taking two seconds to open
        Voice::stack(Wave::Sawtooth, 0.0, 15.0, 0.9, 5)
            .with_filter(350.0, 1200.0, 2.0, 0.0, 1.0)
            .with_reverb(0.6),
        // arp: sparse triangle wails drowning in echo and hall
        Voice::panned(Wave::Triangle, -0.3)
            .with_echo(0.45)
            .with_reverb(0.3),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.2, 1.2),
    echo: Echo::DOTTED,
    humanize: 0.0,
    sweep: 1.0,
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
    perc: PERC_RIDE,
    perc_vel: PERC_RIDE_VEL,
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
        // bass: a doubled saw, slow resonant bite, crushed, a sub beneath
        Voice::wide(Wave::Sawtooth, 0.0, 6.0, 0.3)
            .with_filter(160.0, 900.0, 0.0, 0.2, 3.0)
            .with_drive(0.6)
            .with_sub(0.35),
        // lead: three high squares, huge vibrato, driven, echoing in the hall
        Voice::stack(Wave::Square, 0.2, 12.0, 0.6, 3)
            .with_filter(900.0, 4500.0, 0.0, 0.25, 2.5)
            .with_vibrato(5.0, 25.0, 0.3)
            .with_echo(0.3)
            .with_reverb(0.3)
            .with_drive(0.35),
        // pad: a seven-saw wall opening over a second, gritty, deep in the hall
        Voice::stack(Wave::Sawtooth, 0.0, 16.0, 0.9, 7)
            .with_filter(300.0, 1500.0, 1.2, 0.0, 1.3)
            .with_reverb(0.55)
            .with_drive(0.2),
        // arp: driven square stabs with echoes
        Voice::panned(Wave::Square, -0.3)
            .with_echo(0.4)
            .with_drive(0.2),
        Voice::mono(Wave::Square), // keys: unused
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
    sidechain: Sidechain::new(0.5, 0.9),
    echo: Echo::DOTTED,
    humanize: 0.0,
    sweep: 1.0,
};

// ---------------------------------------------------------------------------
// SONG — "Last Exit" (WAVY / slow): the 3 a.m. ballad. A minor, 91 bpm,
// i–VI–III–VII in sevenths, a wide pad blooming a bar per chord, a gliding
// sine sub bass in half notes, a long tied lead line with late vibrato in
// dotted-eighth echoes, kick on 1 and 3, a clap on 2 and 4, eighth hats.
// ---------------------------------------------------------------------------

/// Two half notes a bar: the chord root and its fifth (or the root again),
/// legato — the bass slides between them.
const EXIT_BASS: &[i32] = &[
    0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 4, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, -2,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 2, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, -5,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, -1, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, -1,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 3, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
];
/// A bar per chord, struck once, held 14 steps (the bloom needs the time).
const EXIT_PAD: &[i32] = &[
    0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, 5,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, 2,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, 6,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST,
];
const EXIT_PAD_CHORDS: &[Chord] = &[Chord::Seventh];
const EXIT_DRUMS: &[Drum] = &[
    Kick, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
    Silent, Silent, Silent, Silent,
];
const EXIT_PERC: &[Drum] = &[
    Hat, Silent, Hat, Silent, Clap, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Clap, Silent,
    Hat, Silent,
];
const EXIT_PERC_VEL: &[u8] = &[5, 0, 3, 0, 8, 0, 3, 0, 5, 0, 3, 0, 8, 0, 4, 0];

const EXIT_INTRO: Section = Section {
    label: "intro",
    pad: EXIT_PAD,
    pad_chord: EXIT_PAD_CHORDS,
    pad_vel: &[7],
    arp: &[
        REST, REST, REST, REST, REST, REST, REST, REST, 14, REST, 16, REST, 18, REST, 16, REST,
    ],
    arp_vel: &[5],
    perc: &[
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Hat,
        Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat,
        Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat,
        Silent, Hat, Hat,
    ],
    perc_vel: &[4, 0, 3, 0],
    ..Section::EMPTY
};
const EXIT_VERSE: Section = Section {
    label: "verse",
    bass: EXIT_BASS,
    lead: &[
        REST, REST, REST, REST, 11, HOLD, HOLD, HOLD, 12, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD,
        9, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST,
        REST, REST, REST, REST, REST, 9, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD, 12, HOLD, HOLD,
        HOLD, 14, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 13, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD,
    ],
    lead_vel: &[7],
    pad: EXIT_PAD,
    pad_chord: EXIT_PAD_CHORDS,
    drums: EXIT_DRUMS,
    perc: EXIT_PERC,
    perc_vel: EXIT_PERC_VEL,
    ..Section::EMPTY
};
const EXIT_REFRAIN: Section = Section {
    label: "refrain",
    bass: EXIT_BASS,
    lead: &[
        14, HOLD, HOLD, HOLD, HOLD, HOLD, 16, HOLD, 14, HOLD, HOLD, HOLD, 12, HOLD, HOLD, HOLD, 11,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 12, HOLD, HOLD, HOLD, 9,
        HOLD, HOLD, HOLD, HOLD, HOLD, 11, HOLD, 12, HOLD, HOLD, HOLD, 14, HOLD, HOLD, HOLD, 13,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD,
    ],
    lead_vel: &[9, 9, 9, 9, 9, 9, 7, 9, 8, 9, 9, 9, 7, 9, 9, 9],
    pad: EXIT_PAD,
    pad_chord: EXIT_PAD_CHORDS,
    arp: &[
        14, 16, 18, 21, 16, 18, 21, 23, 12, 14, 16, 19, 14, 16, 19, 21,
    ],
    arp_vel: &[6, 3, 4, 3, 5, 3, 4, 3],
    drums: EXIT_DRUMS,
    perc: EXIT_PERC,
    perc_vel: EXIT_PERC_VEL,
    ..Section::EMPTY
};
const EXIT_OUTRO: Section = Section {
    label: "outro",
    bass: &[
        0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD,
    ],
    lead: &[
        REST, REST, REST, REST, REST, REST, REST, REST, 11, HOLD, HOLD, HOLD, 9, HOLD, HOLD, HOLD,
        7, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD,
    ],
    lead_vel: &[6],
    pad: &[
        0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD,
    ],
    pad_chord: &[Chord::Add9],
    perc: &[
        Hat, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Silent,
        Silent, Hat, Silent,
    ],
    perc_vel: &[3, 0, 2, 0, 0, 0, 2, 0, 3, 0, 2, 0, 0, 0, 2, 0],
    ..Section::EMPTY
};

const LAST_EXIT: SongSpec = SongSpec {
    name: "Last Exit",
    root: 110.0, // A2
    scale: MINOR,
    bpm: 91.0,
    steps_per_beat: 4,
    voices: [
        // bass: a soft triangle over a big sine sub, sliding between its two
        // notes a bar
        Voice::mono(Wave::Triangle)
            .with_env(0.02, 1.5)
            .with_filter(120.0, 500.0, 0.0, 0.4, 1.0)
            .with_sub(0.7)
            .with_glide(0.12),
        // lead: a doubled square, a slow late vibrato, a long dotted echo
        // in the hall, sliding into each tied note
        Voice::wide(Wave::Square, 0.15, 5.0, 0.35)
            .with_env(0.03, 1.5)
            .with_filter(900.0, 2400.0, 0.0, 0.35, 1.2)
            .with_vibrato(4.8, 10.0, 0.5)
            .with_glide(0.09)
            .with_echo(0.45)
            .with_reverb(0.4),
        // pad: seven saws, very wide, blooming over 1.8 s deep in the hall
        Voice::stack(Wave::Sawtooth, 0.0, 10.0, 0.95, 7)
            .with_filter(400.0, 1900.0, 1.8, 0.0, 0.9)
            .with_reverb(0.6),
        // arp: a quiet sine sparkle, echoing to the right
        Voice::panned(Wave::Sine, -0.4)
            .with_echo(0.55)
            .with_reverb(0.3),
        Voice::mono(Wave::Square), // keys: unused
    ],
    sections: &[
        EXIT_INTRO,
        EXIT_VERSE,
        EXIT_REFRAIN,
        EXIT_VERSE,
        EXIT_REFRAIN,
        EXIT_OUTRO,
        EXIT_REFRAIN,
    ],
    intensity: 0.6,
    swing: 0.0,
    sidechain: Sidechain::new(0.3, 1.2),
    echo: Echo::new(3.0, 0.45, 2600.0),
    humanize: 0.006,
    sweep: 0.2,
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
    pad_chord: SODIUM_PAD_CHORDS,
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
/// One voicing per bar under [`SODIUM_PAD`]: i7 – VI – III(add9) – VII.
const SODIUM_PAD_CHORDS: &[Chord] = &[
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Seventh,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Add9,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
    Chord::Triad,
];

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
    pad_chord: SODIUM_PAD_CHORDS,
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
    pad_chord: SODIUM_PAD_CHORDS,
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
    pad_chord: SODIUM_PAD_CHORDS,
    // The riser: silent for two bars, then one 32-step noise swell.
    keys: &[
        REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST, REST,
        REST, REST, 0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD, HOLD, HOLD,
    ],
    keys_vel: &[7],
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
        // bass: a centred saw with a fast resonant pluck and a sine sub under it
        Voice::mono(Wave::Sawtooth)
            .with_filter(230.0, 1100.0, 0.0, 0.12, 3.0)
            .with_sub(0.5),
        // lead: a doubled square, a little right, late vibrato, a soft wow,
        // dotted-eighth echoes trailing into the hall
        Voice::wide(Wave::Square, 0.25, 6.0, 0.3)
            .with_vibrato(5.5, 12.0, 0.25)
            .with_filter(1800.0, 5200.0, 0.0, 0.18, 1.6)
            .with_echo(0.35)
            .with_reverb(0.25),
        // pad: a five-saw supersaw that blooms open over a second, deep in
        // the hall
        Voice::stack(Wave::Sawtooth, 0.0, 12.0, 0.85, 5)
            .with_filter(700.0, 2600.0, 1.1, 0.0, 1.1)
            .with_reverb(0.45),
        // arp: a triangle answering from the left, echoing to the right
        Voice::panned(Wave::Triangle, -0.35).with_echo(0.5),
        // keys: the noise RISER out of the break — a filter opening over
        // five seconds under a slow swell, deep in the hall
        Voice::mono(Wave::Noise)
            .with_env(2.5, 1.0)
            .with_filter(300.0, 7000.0, 4.8, 0.0, 1.8)
            .with_reverb(0.4),
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
    echo: Echo::new(3.0, 0.42, 2800.0),
    humanize: 0.004,
    sweep: 0.35,
};

// ---------------------------------------------------------------------------
// SONG 10 — "Blood Engine" (AGGRESSIVE / darksynth): the chase. E harmonic
// minor, i–VI–iv–V in power chords, 126 bpm. A driven saw bass hammering
// sixteenths with octave jumps, KEYS power-chord stabs with a resonant wow,
// a three-square lead wailing over a five-saw fifths pad, octave arps in
// dotted-eighth echoes, double-kick refrains, a tom fill into the drop.
// ---------------------------------------------------------------------------

/// The bass per bar: root / octave hammer in sixteenths, one bar per chord
/// (E, C, A, B — the C, A and B below the root).
const ENGINE_BASS: &[i32] = &[
    0, 0, 7, 0, 0, 0, 7, 0, 0, 7, 0, 0, 0, 0, 7, 7, -2, -2, 5, -2, -2, -2, 5, -2, -2, 5, -2, -2,
    -2, -2, 5, 5, -4, -4, 3, -4, -4, -4, 3, -4, -4, 3, -4, -4, -4, -4, 3, 3, -3, -3, 4, -3, -3, -3,
    4, -3, -3, 4, -3, -3, -3, -3, 4, 4,
];
const ENGINE_BASS_VEL: &[u8] = &[9, 6, 7, 6, 9, 6, 7, 6, 9, 7, 6, 6, 9, 6, 8, 8];
/// Off-beat power-chord stabs on the chord roots (E3, C3, A3, B3).
const ENGINE_KEYS: &[i32] = &[
    REST, REST, 7, REST, REST, 7, REST, REST, 7, REST, REST, 7, REST, REST, 7, REST, REST, REST, 5,
    REST, REST, 5, REST, REST, 5, REST, REST, 5, REST, REST, 5, REST, REST, REST, 10, REST, REST,
    10, REST, REST, 10, REST, REST, 10, REST, REST, 10, REST, REST, REST, 11, REST, REST, 11, REST,
    REST, 11, REST, REST, 11, REST, REST, 11, REST,
];
const ENGINE_KEYS_VEL: &[u8] = &[0, 0, 9, 0, 0, 7, 0, 0, 8, 0, 0, 7, 0, 0, 9, 0];
/// A bar of fifths per chord, struck once and held twelve steps.
const ENGINE_PAD: &[i32] = &[
    7, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST, 5,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST, 10,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST, 11,
    HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, REST, REST, REST, REST,
];
const ENGINE_PAD_CHORDS: &[Chord] = &[Chord::Power];
/// Root / octave / fifth / octave arps on each chord (E4, C4, A4, B4).
const ENGINE_ARP: &[i32] = &[
    14, 21, 18, 21, 14, 21, 18, 21, 14, 21, 18, 21, 14, 21, 18, 21, 12, 19, 16, 19, 12, 19, 16, 19,
    12, 19, 16, 19, 12, 19, 16, 19, 17, 24, 21, 24, 17, 24, 21, 24, 17, 24, 21, 24, 17, 24, 21, 24,
    18, 25, 22, 25, 18, 25, 22, 25, 18, 25, 22, 25, 18, 25, 22, 25,
];
const ENGINE_ARP_VEL: &[u8] = &[9, 5, 7, 5];
const ENGINE_DRUMS: &[Drum] = &[
    Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Kick, Silent, Snare,
    Silent, Silent, Silent,
];
const ENGINE_DRUMS_DOUBLE: &[Drum] = &[
    Kick, Silent, Silent, Kick, Snare, Silent, Silent, Silent, Kick, Kick, Silent, Silent, Snare,
    Silent, Silent, Kick,
];
/// Sixteenth hats with claps under the snares and an open hat into the bar.
const ENGINE_PERC: &[Drum] = &[
    Hat, Hat, Hat, Hat, Clap, Hat, Hat, Hat, Hat, Hat, Hat, Hat, Clap, Hat, OpenHat, Hat,
];
const ENGINE_PERC_VEL: &[u8] = &[7, 3, 5, 3, 9, 3, 5, 3, 7, 3, 5, 3, 9, 3, 7, 3];

const ENGINE_INTRO: Section = Section {
    label: "intro",
    pad: ENGINE_PAD,
    pad_chord: ENGINE_PAD_CHORDS,
    arp: ENGINE_ARP,
    arp_vel: &[7, 3, 5, 3],
    perc: &[
        Crash, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat, Silent, Hat,
        Hat, Hat, Hat, Hat, Hat, Hat, Hat,
    ],
    perc_vel: &[
        8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 4, 0, 5, 0, 5, 0, 5, 4, 6, 5,
        7, 6, 8, 8,
    ],
    ..Section::EMPTY
};
const ENGINE_VERSE: Section = Section {
    label: "verse",
    bass: ENGINE_BASS,
    bass_vel: ENGINE_BASS_VEL,
    keys: ENGINE_KEYS,
    keys_vel: ENGINE_KEYS_VEL,
    keys_chord: &[Chord::Power],
    pad: ENGINE_PAD,
    pad_chord: ENGINE_PAD_CHORDS,
    drums: ENGINE_DRUMS,
    perc: ENGINE_PERC,
    perc_vel: ENGINE_PERC_VEL,
    ..Section::EMPTY
};
const ENGINE_REFRAIN: Section = Section {
    label: "refrain",
    bass: ENGINE_BASS,
    bass_vel: ENGINE_BASS_VEL,
    lead: &[
        14, HOLD, HOLD, HOLD, HOLD, HOLD, 13, HOLD, 12, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD, 12,
        HOLD, HOLD, HOLD, HOLD, HOLD, 11, HOLD, 9, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 11,
        HOLD, HOLD, HOLD, 12, HOLD, HOLD, HOLD, 13, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 14,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 16, HOLD, HOLD, HOLD, 13, HOLD, HOLD, HOLD,
    ],
    lead_vel: &[9, 9, 9, 9, 9, 9, 7, 9, 8, 9, 9, 9, 7, 9, 9, 9],
    keys: ENGINE_KEYS,
    keys_vel: ENGINE_KEYS_VEL,
    keys_chord: &[Chord::Power],
    pad: ENGINE_PAD,
    pad_chord: ENGINE_PAD_CHORDS,
    arp: ENGINE_ARP,
    arp_vel: ENGINE_ARP_VEL,
    drums: ENGINE_DRUMS_DOUBLE,
    perc: ENGINE_PERC,
    perc_vel: ENGINE_PERC_VEL,
    ..Section::EMPTY
};
const ENGINE_BREAK: Section = Section {
    label: "break",
    bass: &[
        0, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, -2, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD,
    ],
    lead: &[
        REST, REST, REST, REST, 7, HOLD, HOLD, HOLD, 9, HOLD, HOLD, HOLD, 11, HOLD, HOLD, HOLD, 12,
        HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, 13, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
    ],
    lead_vel: &[7],
    pad: &[
        7, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, 5, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD, HOLD,
        HOLD, HOLD,
    ],
    pad_chord: &[Chord::Sus2],
    drums: &[
        Kick, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Kick, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Kick, Silent, Silent, Silent, Tom, Silent, Tom, Silent,
        Tom, Silent, Tom, Tom, Snare, Snare, Snare, Snare,
    ],
    drums_vel: &[
        8, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 7, 0, 7, 0, 8, 0, 8, 8, 6, 7,
        8, 9,
    ],
    perc: &[
        Crash, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
        Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent, Silent,
    ],
    perc_vel: &[7],
    ..Section::EMPTY
};

const BLOOD_ENGINE: SongSpec = SongSpec {
    name: "Blood Engine",
    root: 82.41, // E2
    scale: HARMONIC_MINOR,
    bpm: 126.0,
    steps_per_beat: 4,
    voices: [
        // bass: a driven saw with a fast resonant snap and a sub under it
        Voice::mono(Wave::Sawtooth)
            .with_filter(200.0, 1500.0, 0.0, 0.07, 4.5)
            .with_drive(0.55)
            .with_sub(0.4),
        // lead: three squares, wide vibrato, a resonant wow, drive, echo,
        // sliding between the tied phrase notes
        Voice::stack(Wave::Square, 0.2, 9.0, 0.5, 3)
            .with_filter(1300.0, 6000.0, 0.0, 0.16, 2.8)
            .with_glide(0.06)
            .with_vibrato(6.0, 18.0, 0.2)
            .with_echo(0.3)
            .with_reverb(0.2)
            .with_drive(0.35),
        // pad: five saws in fifths, blooming over 0.6 s, back in the hall
        Voice::stack(Wave::Sawtooth, 0.0, 14.0, 0.9, 5)
            .with_filter(450.0, 2200.0, 0.6, 0.0, 1.3)
            .with_reverb(0.5),
        // arp: a square in dotted-eighth echoes, left
        Voice::panned(Wave::Square, -0.35).with_echo(0.45),
        // keys: short power-chord stabs — three saws, a big wow, crushed
        Voice::stack(Wave::Sawtooth, 0.15, 10.0, 0.6, 3)
            .with_env(0.003, 0.5)
            .with_filter(500.0, 3500.0, 0.0, 0.15, 2.5)
            .with_drive(0.5)
            .with_echo(0.15),
    ],
    sections: &[
        ENGINE_INTRO,
        ENGINE_VERSE,
        ENGINE_REFRAIN,
        ENGINE_VERSE,
        ENGINE_REFRAIN,
        ENGINE_BREAK,
        ENGINE_REFRAIN,
        ENGINE_REFRAIN,
    ],
    intensity: 1.05,
    swing: 0.0,
    sidechain: Sidechain::new(0.45, 0.7),
    echo: Echo::new(3.0, 0.3, 2400.0),
    humanize: 0.003,
    sweep: 0.5,
};

/// All songs, in ascending darkness (intro first). Index into this with
/// `play_song`, or map a floor number through `song_for_floor`.
pub const SONGS: &[SongSpec] = &[
    INSERT_COIN,
    NEON_LOUNGE,
    LAST_EXIT,
    SODIUM_LIGHTS,
    CHROME_VEINS,
    DESCENT,
    BLOOD_RUSH,
    DEEP_STATIC,
    BLOOD_ENGINE,
    STATIC_PRAYER,
    MASK_OF_DREAD,
];

/// Pick a song for a given floor, escalating darkness as you descend. Kept as a
/// plain mapping so the integrator can call it per level.
pub fn song_for_floor(level: usize) -> SongSpec {
    match level {
        0 => NEON_LOUNGE,
        1 => LAST_EXIT,
        2..=3 => SODIUM_LIGHTS,
        4..=5 => CHROME_VEINS,
        6..=7 => DESCENT,
        8..=9 => BLOOD_RUSH,
        10 => DEEP_STATIC,
        11..=12 => BLOOD_ENGINE,
        13 => STATIC_PRAYER,
        _ => MASK_OF_DREAD,
    }
}

// --- reading the format ------------------------------------------------------

/// The name of a known mode, for the tracker's info line (`"CUSTOM"` for
/// any other set of offsets).
pub fn scale_name(scale: Scale) -> &'static str {
    match scale {
        s if s == MINOR => "MINOR",
        s if s == DORIAN => "DORIAN",
        s if s == HARMONIC_MINOR => "HARMONIC MINOR",
        s if s == PHRYGIAN => "PHRYGIAN",
        s if s == PHRYGIAN_DOMINANT => "PHRYGIAN DOMINANT",
        s if s == LOCRIAN => "LOCRIAN",
        _ => "CUSTOM",
    }
}

/// The nearest note name + octave of a frequency (`55.0` → `"A1"`,
/// `73.42` → `"D2"`), scientific pitch, A4 = 440 Hz.
pub fn note_name(hz: f64) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    if hz <= 0.0 || !hz.is_finite() {
        return "?".to_string();
    }
    // Semitones above C0 (16.352 Hz).
    let n = (12.0 * (hz / 16.351_6).log2()).round() as i64;
    let name = NAMES[n.rem_euclid(12) as usize];
    format!("{}{}", name, n.div_euclid(12))
}

/// One-line summary of a voice for the tracker (`"SAW ×5 ±12C · BLOOM ·
/// HALL 45%"`): shape and stack, then which features are on.
pub fn voice_summary(v: &Voice) -> String {
    let wave = match v.wave {
        Wave::Sine => "SIN",
        Wave::Triangle => "TRI",
        Wave::Square => "SQR",
        Wave::Sawtooth => "SAW",
        Wave::Noise => "NOISE",
    };
    let mut parts = Vec::new();
    let n = v.oscillators();
    if n > 1 {
        parts.push(format!("{wave} ×{n} ±{:.0}C", v.detune));
    } else {
        parts.push(wave.to_string());
    }
    if v.sub > 0.0 {
        parts.push(format!("SUB {:.0}%", v.sub * 100.0));
    }
    if v.glide > 0.0 {
        parts.push(format!("GLIDE {:.0}MS", v.glide * 1000.0));
    }
    if let Some(f) = v.filter {
        if f.attack > 0.0 {
            parts.push("BLOOM".to_string());
        }
        if f.decay > 0.0 || f.attack == 0.0 {
            parts.push(format!("WOW Q{:.0}", f.q));
        }
    }
    if let Some(vb) = v.vibrato {
        parts.push(format!("VIB {:.0}C", vb.depth));
    }
    if let Some(e) = v.env {
        parts.push(format!("GATE {:.1}", e.gate));
    }
    if v.drive > 0.0 {
        parts.push(format!("DRV {:.0}%", v.drive * 100.0));
    }
    if v.echo > 0.0 {
        parts.push(format!("ECHO {:.0}%", v.echo * 100.0));
    }
    if v.reverb > 0.0 {
        parts.push(format!("HALL {:.0}%", v.reverb * 100.0));
    }
    parts.join(" · ")
}

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
    /// The degree of the lane's previous note when it runs right into this
    /// one (its tail ends where this starts, no rest between) and differs —
    /// what a legato glide comes from. `None` after a rest, at the loop's
    /// first note of an otherwise empty lane, or for a repeated pitch.
    pub from: Option<i32>,
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
    // Legato: walk back over the previous note's HOLDs to its start; a
    // REST anywhere on the way (or nothing but HOLDs) means no glide.
    let mut from = None;
    for back in 1..n {
        match pattern[(step + n - back) % n] {
            HOLD => continue,
            REST => break,
            d => {
                if d != degree {
                    from = Some(d);
                }
                break;
            }
        }
    }
    Some(NoteOn {
        degree,
        len: len.min(u16::MAX as usize) as u16,
        from,
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

/// Read a chord lane at `step` (loops): the lane's default voicing for an
/// empty lane.
pub fn chord_at(lane: usize, chords: &[Chord], step: usize) -> Chord {
    if chords.is_empty() {
        return Chord::default_for(lane);
    }
    chords[step % chords.len()]
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
        .max(sec.keys.len())
        .max(sec.drums.len())
        .max(sec.perc.len())
        .max(1)
}

/// A lane's built-in note shape: `(gate, level, attack)` — the decay tail
/// in STEPS an untied note rings, its mix level, and its attack in seconds.
/// (The pad level is per chord: its default triad lands each partial at
/// 0.78/√3 = 0.45 after the 1/√n split.)
pub fn lane_shape(lane: usize) -> (f64, f64, f64) {
    match lane {
        BASS => (1.9, 1.3, 0.005),
        LEAD => (0.9, 1.0, 0.005),
        PAD => (4.0, 0.78, 0.06),
        KEYS => (1.2, 0.8, 0.005),
        _ => (0.7, 0.7, 0.005),
    }
}

/// The lane's shape with the song voice's [`Env`] override applied.
pub fn voice_shape(song: &SongSpec, lane: usize) -> (f64, f64, f64) {
    let (gate, level, attack) = lane_shape(lane);
    match song.voices.get(lane).and_then(|v| v.env) {
        Some(e) => (e.gate.max(0.05), level, e.attack.max(0.0)),
        None => (gate, level, attack),
    }
}

/// One sequencer step of `song`, in seconds.
pub fn step_seconds(song: &SongSpec) -> f64 {
    60.0 / song.bpm.max(1.0) / f64::from(song.steps_per_beat.max(1))
}

/// Seconds of signal one baked voice needs: a note's attack + its tied
/// hold + the lane's decay tail (plus the builders' 30 ms stop margin), or
/// the longest layer of a kit piece.
pub fn key_seconds(song: &SongSpec, key: MusicKey) -> f64 {
    match key {
        MusicKey::Note { lane, len, .. } => {
            let (gate, _, attack) = voice_shape(song, lane);
            attack + step_seconds(song) * (gate + f64::from(len.max(1) - 1)) + 0.03
        }
        MusicKey::Drum(d) => match d {
            Drum::Kick => 0.21,
            Drum::Hat => 0.06,
            Drum::Snare => 0.16,
            Drum::Clap => 0.20,
            Drum::OpenHat => 0.31,
            Drum::Tom => 0.31,
            Drum::Rim => 0.06,
            Drum::Crash => 1.0,
            Drum::Silent => 0.03,
        },
    }
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
    /// A melodic lane ([`MELODIC`]) note at this scale degree, this many
    /// steps long (1 = untied), voiced as `chord`, gliding in from `from`
    /// (only ever `Some` on a lane whose voice has a `glide`).
    Note {
        lane: usize,
        degree: i32,
        len: u16,
        chord: Chord,
        from: Option<i32>,
    },
    /// One kit piece (never `Silent`) — shared by both percussion lanes.
    Drum(Drum),
}

/// Enumerate the exact, finite voice set `song` can ever schedule: the
/// distinct (degree, length, voicing) triples of each melodic lane across
/// every section, plus the kit pieces its percussion lanes use — in
/// bake-priority order (drums first — the densest lanes — then the melodic
/// lanes per [`MELODIC`]). Typically 30–50 keys per song.
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
    for lane in MELODIC {
        for sec in song.sections {
            let pattern = sec.lane(lane);
            for step in 0..pattern.len() {
                if let Some(n) = note_at(pattern, step) {
                    add(&mut keys, note_key(song, sec, lane, step, &n));
                }
            }
        }
    }
    keys
}

/// The bake key of the note `n` starting at `step` of `lane` in `sec`: its
/// voicing from the chord lane, its glide origin only if the lane's voice
/// glides (so a non-gliding lane never multiplies its keys by context).
pub fn note_key(song: &SongSpec, sec: &Section, lane: usize, step: usize, n: &NoteOn) -> MusicKey {
    let glides = song.voices.get(lane).is_some_and(|v| v.glide > 0.0);
    MusicKey::Note {
        lane,
        degree: n.degree,
        len: n.len,
        chord: chord_at(lane, sec.chord_lane(lane), step),
        from: if glides { n.from } else { None },
    }
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
        let on = |degree, len, from| Some(NoteOn { degree, len, from });
        let lane = [0, HOLD, HOLD, HOLD, 3, REST, HOLD, 5];
        // The lane loops, so the 0 runs straight on from the 5 at its end.
        assert_eq!(note_at(&lane, 0), on(0, 4, Some(5)));
        assert_eq!(note_at(&lane, 1), None, "a HOLD is not a note start");
        // 3 starts right where the held 0 ends: legato from 0.
        assert_eq!(note_at(&lane, 4), on(3, 1, Some(0)));
        assert_eq!(note_at(&lane, 5), None);
        assert_eq!(note_at(&lane, 6), None, "a HOLD after a REST is silent");
        // Counting wraps around the lane but stops at the next note start
        // (step 0 here), so the last note is one step long.
        assert_eq!(note_at(&lane, 7), on(5, 1, None));
        // Wrapping tie: a note at the end sustains into the lane's repeat —
        // and it runs straight on from the 2 before it (legato).
        let wrap = [HOLD, HOLD, 2, 4];
        assert_eq!(note_at(&wrap, 3), on(4, 3, Some(2)));
        // Steps beyond the lane length loop.
        assert_eq!(note_at(&wrap, 7), note_at(&wrap, 3));
        assert_eq!(note_at(&[], 0), None);
        // An all-HOLD lane never sounds (and never loops forever counting).
        assert_eq!(note_at(&[HOLD, HOLD], 0), None);
    }

    #[test]
    fn legato_origin_needs_touching_different_notes() {
        let touching = [0, 3, 3, REST, 5, HOLD, 7];
        assert_eq!(note_at(&touching, 1).and_then(|n| n.from), Some(0));
        assert_eq!(
            note_at(&touching, 2).and_then(|n| n.from),
            None,
            "same pitch"
        );
        assert_eq!(
            note_at(&touching, 4).and_then(|n| n.from),
            None,
            "after a rest"
        );
        assert_eq!(
            note_at(&touching, 6).and_then(|n| n.from),
            Some(5),
            "after a tie"
        );
        // The origin only enters the bake key on a gliding voice.
        const SEC: Section = Section {
            lead: &[0, 3, 0, 3],
            ..Section::EMPTY
        };
        let plain = SongSpec {
            sections: &[SEC],
            ..SONGS[0]
        };
        assert_eq!(music_keys(&plain).len(), 2);
        let mut voices = SONGS[0].voices;
        voices[LEAD] = voices[LEAD].with_glide(0.1);
        let gliding = SongSpec { voices, ..plain };
        // 0←3 and 3←0 (each is reached from the other around the loop).
        assert_eq!(music_keys(&gliding).len(), 2);
        assert!(music_keys(&gliding)
            .iter()
            .all(|k| matches!(k, MusicKey::Note { from: Some(_), .. })));
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
    fn voice_builders_compose() {
        let v = Voice::stack(Wave::Sawtooth, 0.1, 10.0, 0.5, 5)
            .with_filter(500.0, 2000.0, 0.0, 0.2, 2.0)
            .with_vibrato(5.0, 10.0, 0.3)
            .with_env(0.01, 2.0)
            .with_drive(0.4);
        assert_eq!(v.oscillators(), 5);
        assert!(v.is_wide());
        assert_eq!(v.filter.map(|f| f.peak), Some(2000.0));
        assert_eq!(v.vibrato.map(|vb| vb.rate), Some(5.0));
        assert_eq!(v.env.map(|e| e.gate), Some(2.0));
        assert_eq!(v.drive, 0.4);
        assert_eq!(v.pan, 0.1);
        let w = v.with_echo(0.3).with_reverb(0.2);
        assert_eq!((w.echo, w.reverb), (0.3, 0.2));
        assert_eq!(Voice::mono(Wave::Sine).echo, 0.0);
        // No detune = no stack, whatever the count; no width = not wide.
        assert_eq!(Voice::stack(Wave::Sine, 0.0, 0.0, 1.0, 7).oscillators(), 1);
        assert!(!Voice::wide(Wave::Sine, 0.0, 5.0, 0.0).is_wide());
        assert_eq!(Voice::wide(Wave::Sine, 0.0, 5.0, 0.0).oscillators(), 2);
        assert_eq!(Voice::mono(Wave::Sine).oscillators(), 1);
        assert_eq!(MELODIC.len(), NUM_VOICES);
        assert_eq!(CHANNEL_NAMES[KEYS], "KEYS");
    }

    #[test]
    fn chords_default_per_lane_and_stay_in_key() {
        assert_eq!(chord_at(PAD, &[], 5), Chord::Triad);
        assert_eq!(chord_at(LEAD, &[], 5), Chord::Single);
        assert_eq!(
            chord_at(LEAD, &[Chord::Power, Chord::Octave], 3),
            Chord::Octave
        );
        // Every voicing starts at or above the written note's octave and is
        // spelled lowest-first (the synth relies on the order for levels).
        let mut n = 0;
        for c in [
            Chord::Single,
            Chord::Octave,
            Chord::Power,
            Chord::Triad,
            Chord::Sus2,
            Chord::Sus4,
            Chord::Seventh,
            Chord::Add9,
            Chord::Inv1,
            Chord::Inv2,
            Chord::Open,
        ] {
            let d = c.degrees();
            assert!(!d.is_empty());
            assert!(d.windows(2).all(|w| w[0] < w[1]), "{c:?} not ascending");
            assert!(d[0] >= 0, "{c:?} below the root");
            n += 1;
        }
        assert_eq!(n, 11);
        // A Triad in A minor on the root is A C E (0, 3, 7 semitones).
        let f: Vec<f64> = Chord::Triad
            .degrees()
            .iter()
            .map(|&d| degree_freq(55.0, MINOR, d))
            .collect();
        assert!((f[1] / f[0] - 2f64.powf(3.0 / 12.0)).abs() < 1e-9);
        assert!((f[2] / f[0] - 2f64.powf(7.0 / 12.0)).abs() < 1e-9);
        // A chord lane changes the bake key.
        const SEC: Section = Section {
            lead: &[0, 0],
            lead_chord: &[Chord::Single, Chord::Power],
            ..Section::EMPTY
        };
        let keys = music_keys(&SongSpec {
            sections: &[SEC],
            ..SONGS[0]
        });
        assert_eq!(keys.len(), 2);
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
            assert!(
                song.echo.steps > 0.0 && song.echo.tone > 0.0,
                "{}: echo",
                song.name
            );
            assert!(
                (0.0..0.95).contains(&song.echo.feedback),
                "{}: echo fb",
                song.name
            );
            assert!((0.0..=1.0).contains(&song.sweep), "{}: sweep", song.name);
            assert!(
                (0.0..=0.02).contains(&song.humanize),
                "{}: humanize",
                song.name
            );
            for v in song.voices {
                assert!((-1.0..=1.0).contains(&v.pan), "{}: pan", song.name);
                assert!((0.0..=1.0).contains(&v.width), "{}: width", song.name);
                assert!(v.detune >= 0.0, "{}: detune", song.name);
                assert!((1..=7).contains(&v.unison), "{}: unison", song.name);
                assert!((0.0..=1.0).contains(&v.drive), "{}: drive", song.name);
                assert!((0.0..=1.0).contains(&v.echo), "{}: echo", song.name);
                assert!((0.0..=1.0).contains(&v.reverb), "{}: reverb", song.name);
                assert!((0.0..=1.0).contains(&v.sub), "{}: sub", song.name);
                assert!(v.glide >= 0.0, "{}: glide", song.name);
                if let Some(e) = v.env {
                    assert!(e.attack >= 0.0 && e.gate > 0.0, "{}: env", song.name);
                }
                if let Some(f) = v.filter {
                    assert!(
                        f.cutoff >= 20.0 && f.peak >= f.cutoff,
                        "{}: filter",
                        song.name
                    );
                    assert!(
                        f.attack >= 0.0 && f.decay >= 0.0 && f.q > 0.0,
                        "{}",
                        song.name
                    );
                }
                if let Some(vb) = v.vibrato {
                    assert!(
                        vb.rate > 0.0 && vb.depth >= 0.0 && vb.delay >= 0.0,
                        "{}",
                        song.name
                    );
                }
            }
            for sec in song.sections {
                for lane in MELODIC {
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
                for lane in MELODIC {
                    let p = sec.lane(lane);
                    for step in 0..p.len() {
                        if let Some(n) = note_at(p, step) {
                            let key = note_key(song, sec, lane, step, &n);
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
                "{:14} {:2} voices (drums {} bass {:2} lead {:2} arp {:2} keys {:2} pad {:2})",
                song.name,
                keys.len(),
                count(|k| matches!(k, MusicKey::Drum(_))),
                count(|k| matches!(k, MusicKey::Note { lane: BASS, .. })),
                count(|k| matches!(k, MusicKey::Note { lane: LEAD, .. })),
                count(|k| matches!(k, MusicKey::Note { lane: ARP, .. })),
                count(|k| matches!(k, MusicKey::Note { lane: KEYS, .. })),
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
    fn info_line_helpers() {
        assert_eq!(scale_name(MINOR), "MINOR");
        assert_eq!(scale_name(LOCRIAN), "LOCRIAN");
        assert_eq!(scale_name(&[0, 5]), "CUSTOM");
        assert_eq!(note_name(55.0), "A1");
        assert_eq!(note_name(73.42), "D2");
        assert_eq!(note_name(440.0), "A4");
        assert_eq!(note_name(32.70), "C1");
        assert_eq!(note_name(0.0), "?");
        assert_eq!(voice_summary(&Voice::mono(Wave::Sine)), "SIN");
        let b = Voice::mono(Wave::Sawtooth).with_sub(0.5).with_glide(0.08);
        assert_eq!(voice_summary(&b), "SAW · SUB 50% · GLIDE 80MS");
        assert_eq!(voice_summary(&Voice::mono(Wave::Noise)), "NOISE");
        assert_eq!(
            Voice::stack(Wave::Noise, 0.0, 10.0, 1.0, 5).oscillators(),
            1
        );
        let v = Voice::stack(Wave::Sawtooth, 0.0, 12.0, 0.9, 5)
            .with_filter(500.0, 2000.0, 1.0, 0.0, 1.1)
            .with_reverb(0.45);
        assert_eq!(voice_summary(&v), "SAW ×5 ±12C · BLOOM · HALL 45%");
        let w = Voice::mono(Wave::Square)
            .with_filter(500.0, 2000.0, 0.0, 0.1, 3.0)
            .with_vibrato(5.0, 10.0, 0.2)
            .with_drive(0.5)
            .with_echo(0.3);
        assert_eq!(
            voice_summary(&w),
            "SQR · WOW Q3 · VIB 10C · DRV 50% · ECHO 30%"
        );
    }

    #[test]
    fn bake_lengths_cover_the_note() {
        let song = SONGS[0]; // 84 bpm, 16ths: a step is ~0.1786 s
        let sd = step_seconds(&song);
        assert!((sd - 60.0 / 84.0 / 4.0).abs() < 1e-9);
        let one = MusicKey::Note {
            lane: LEAD,
            degree: 0,
            len: 1,
            chord: Chord::Single,
            from: None,
        };
        let held = MusicKey::Note {
            lane: LEAD,
            degree: 0,
            len: 8,
            chord: Chord::Single,
            from: None,
        };
        // A held note bakes its 7 extra steps on top of the untied length.
        assert!((key_seconds(&song, held) - key_seconds(&song, one) - 7.0 * sd).abs() < 1e-9);
        // An env override with a long attack is baked in full.
        let mut voices = song.voices;
        voices[KEYS] = Voice::mono(Wave::Noise).with_env(2.5, 1.0);
        let riser = SongSpec { voices, ..song };
        let key = MusicKey::Note {
            lane: KEYS,
            degree: 0,
            len: 1,
            chord: Chord::Single,
            from: None,
        };
        assert!(key_seconds(&riser, key) > 2.5 + sd);
        assert_eq!(voice_shape(&riser, KEYS).0, 1.0);
        assert_eq!(voice_shape(&riser, LEAD), lane_shape(LEAD));
        // Every kit piece is baked at least as long as its layers.
        for d in Drum::KIT {
            assert!(key_seconds(&song, MusicKey::Drum(d)) >= 0.05, "{d:?}");
        }
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
