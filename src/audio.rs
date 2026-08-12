//! Procedural audio engine for Open Miami // Rogue Purge.
//!
//! Everything here is synthesized at runtime with the Web Audio API (via
//! `web-sys`): oscillators for tones, a white-noise buffer for hits and
//! whooshes, all shaped by gain envelopes for a punchy, glitchy synthwave feel.
//! No audio files, no extra dependencies.
//!
//! Robustness first: if the `AudioContext` (or any node) fails to build we
//! silently degrade to silence. Nothing in here ever panics or unwraps a
//! fallible Web Audio call — every `Result` is swallowed so the game runs fine
//! even when audio is unavailable or blocked by the browser.

use web_sys::{AudioBuffer, AudioContext, AudioDestinationNode, BiquadFilterType, OscillatorType};

/// Look-ahead window (seconds) for the music scheduler: we queue notes this far
/// in advance of the audio clock so playback never gaps between frames.
const LOOKAHEAD: f64 = 0.15;

/// Master music level — kept low so the looping backing never buries the SFX.
const MUSIC_GAIN: f64 = 0.07;

/// Sentinel used inside a pattern to mean "rest" (no note this step).
const REST: i32 = i32::MIN;

// --- data-driven song format ----------------------------------------------
//
// A song is *plain, `const`-able data*: a key (root frequency + scale), a tempo,
// and three step-sequenced patterns (bass, lead, drums). Patterns are read one
// step at a time by the same look-ahead scheduler that used to drive the single
// hardcoded loop, so adding a song is just adding another `SongSpec` const.
//
// Melodic patterns are written as *scale degrees* (see `degree_freq`): `0` is
// the root, `1` the next scale note up, `7` an octave up (for a 7-note scale),
// negative degrees drop below the root. `REST` means silence for that step.
// This keeps a song readable and in-key no matter which root/scale it uses.

/// Scale = semitone offsets from the root, one octave's worth. Darker modes
/// (flat 2nd, tritone) read as more menacing — we escalate them across floors.
type Scale = &'static [i32];

/// Aeolian / natural minor — the classic neon-noir minor key.
const MINOR: Scale = &[0, 2, 3, 5, 7, 8, 10];
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

/// A whole song as copyable data. Author one, drop it in `SONGS`, done.
#[derive(Clone, Copy)]
pub struct SongSpec {
    /// Human-readable name (shown in the `?viz` "Musics" tab).
    pub name: &'static str,
    /// Root/tonic frequency in Hz (e.g. `55.0` = A1). Lower == darker/deeper.
    pub root: f64,
    /// The key/mode: semitone offsets from `root`.
    pub scale: Scale,
    /// Tempo in beats per minute.
    pub bpm: f64,
    /// Sequencer resolution: steps per beat (`4` = sixteenth notes).
    pub steps_per_beat: u32,
    /// Bass lane, one scale-degree (or `REST`) per step. Loops.
    pub bass: &'static [i32],
    /// Oscillator shape for the bass voice.
    pub bass_wave: OscillatorType,
    /// Lead/arp lane, one scale-degree (or `REST`) per step. Loops.
    pub lead: &'static [i32],
    /// Oscillator shape for the lead voice.
    pub lead_wave: OscillatorType,
    /// Percussion lane, one `Drum` per step. Loops.
    pub drums: &'static [Drum],
    /// Overall punch/loudness feel (~0.5 lounge .. ~1.2 boss).
    pub intensity: f64,
}

/// Cool, loungey opening groove. A-minor, laid-back tempo, sparse triangle
/// bass and a mellow syncopated lead over light hats — neon at dusk.
const NEON_LOUNGE: SongSpec = SongSpec {
    name: "Neon Lounge",
    root: 55.0, // A1
    scale: MINOR,
    bpm: 108.0,
    steps_per_beat: 4,
    bass: &[
        0, REST, REST, REST, 0, REST, 4, REST, 3, REST, REST, REST, 2, REST, 2, REST,
    ],
    bass_wave: OscillatorType::Triangle,
    lead: &[
        7, REST, 9, REST, REST, 11, REST, 7, REST, REST, 9, REST, 10, REST, REST, REST,
    ],
    lead_wave: OscillatorType::Triangle,
    drums: &[
        Kick, Silent, Hat, Silent, Silent, Silent, Hat, Silent, Kick, Silent, Hat, Silent, Silent,
        Silent, Hat, Snare,
    ],
    intensity: 0.55,
};

/// Tense mid-descent. D Phrygian (flat 2nd), driving square bass hammering the
/// root, a restless sawtooth arp, four-on-the-floor kicks with a backbeat.
const DESCENT: SongSpec = SongSpec {
    name: "Descent",
    root: 36.71, // D1
    scale: PHRYGIAN,
    bpm: 132.0,
    steps_per_beat: 4,
    bass: &[
        0, REST, 0, REST, 0, REST, 0, REST, 0, REST, 0, REST, 5, REST, 4, REST,
    ],
    bass_wave: OscillatorType::Square,
    lead: &[7, 8, 10, 8, 7, 10, 8, 10, 12, 11, 10, 8, 7, 8, 7, REST],
    lead_wave: OscillatorType::Sawtooth,
    drums: &[
        Kick, Hat, Hat, Hat, Snare, Hat, Hat, Hat, Kick, Hat, Kick, Hat, Snare, Hat, Hat, Hat,
    ],
    intensity: 0.85,
};

/// Menacing deep-floor pressure. E Phrygian-dominant (flat 2nd + major 3rd),
/// relentless sawtooth sub-bass in 16ths, dissonant stabs, pounding drums.
const DEEP_STATIC: SongSpec = SongSpec {
    name: "Deep Static",
    root: 41.20, // E1
    scale: PHRYGIAN_DOMINANT,
    bpm: 140.0,
    steps_per_beat: 4,
    bass: &[
        0, 0, 0, REST, 1, REST, 0, REST, 0, 0, 0, REST, 4, REST, 1, 0,
    ],
    bass_wave: OscillatorType::Sawtooth,
    lead: &[
        REST, REST, 7, REST, 8, REST, REST, 7, REST, 11, REST, REST, 8, REST, 7, REST,
    ],
    lead_wave: OscillatorType::Sawtooth,
    drums: &[
        Kick, Silent, Kick, Silent, Snare, Silent, Kick, Kick, Kick, Silent, Kick, Silent, Snare,
        Hat, Kick, Snare,
    ],
    intensity: 1.0,
};

/// Dread-filled BOSS theme. C Locrian (flat 2nd + tritone), slow and heavy;
/// sustained sawtooth bass lurching to the tritone, sparse high square wails,
/// enormous slow kicks and crashing snares. The mask is watching.
const MASK_OF_DREAD: SongSpec = SongSpec {
    name: "Mask of Dread",
    root: 32.70, // C1
    scale: LOCRIAN,
    bpm: 100.0,
    steps_per_beat: 4,
    bass: &[
        0, REST, REST, REST, 0, REST, 4, REST, 0, REST, REST, REST, 4, REST, 3, REST,
    ],
    bass_wave: OscillatorType::Sawtooth,
    lead: &[
        7, REST, REST, REST, REST, REST, REST, REST, 8, REST, REST, REST, REST, REST, 11, REST,
    ],
    lead_wave: OscillatorType::Square,
    drums: &[
        Kick, Silent, Silent, Silent, Snare, Silent, Silent, Silent, Kick, Silent, Silent, Kick,
        Snare, Silent, Snare, Silent,
    ],
    intensity: 1.15,
};

/// All songs, in ascending darkness. Index into this with `play_song`, or map a
/// floor number through `song_for_floor`.
pub const SONGS: &[SongSpec] = &[NEON_LOUNGE, DESCENT, DEEP_STATIC, MASK_OF_DREAD];

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

/// Pick a song for a given floor, escalating darkness as you descend. Kept as a
/// plain mapping so the integrator can call it per level.
pub fn song_for_floor(level: usize) -> SongSpec {
    let idx = match level {
        0..=3 => 0,  // cool opening groove
        4..=7 => 1,  // tense descent
        8..=11 => 2, // deep-floor menace
        _ => 3,      // boss dread
    };
    SONGS[idx]
}

/// The self-contained audio engine. Construct once, hand it around, drive
/// `update()` from the game loop.
pub struct AudioEngine {
    /// `None` if the browser refused to give us an audio context.
    ctx: Option<AudioContext>,
    /// Pre-rendered white noise, reused (via cheap buffer-source nodes) for
    /// every percussive/whoosh sound.
    noise: Option<AudioBuffer>,
    music_playing: bool,
    /// Absolute audio-clock time of the next music step to schedule.
    next_note_time: f64,
    /// Current index into the currently-playing song's step loop.
    step: usize,
    /// The song currently driving the scheduler.
    song: SongSpec,
}

impl AudioEngine {
    /// Try to create the audio context. Never fails hard — on any error the
    /// engine simply stays silent.
    pub fn new() -> Self {
        let ctx = AudioContext::new().ok();
        let noise = ctx.as_ref().and_then(Self::make_noise);
        Self {
            ctx,
            noise,
            music_playing: false,
            next_note_time: 0.0,
            step: 0,
            song: SONGS[0],
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

    // --- one-shot SFX ------------------------------------------------------

    /// Tight high blip + noise click — a gunshot.
    pub fn play_shoot(&self) {
        let t = self.now();
        self.tone(1200.0, 300.0, t, 0.07, 0.22, OscillatorType::Square);
        self.noise(t, 0.05, 0.18, BiquadFilterType::Highpass, 2200.0, 2200.0);
    }

    /// Crunchy low noise thud — a bullet connects.
    pub fn play_hit(&self) {
        let t = self.now();
        self.noise(t, 0.12, 0.35, BiquadFilterType::Lowpass, 900.0, 120.0);
        self.tone(180.0, 60.0, t, 0.10, 0.25, OscillatorType::Square);
    }

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

    // --- music -------------------------------------------------------------

    /// Begin the looping backing track using the current song (idempotent).
    pub fn start_music(&mut self) {
        if self.music_playing {
            return;
        }
        self.music_playing = true;
        self.step = 0;
        self.next_note_time = self.now() + 0.1;
    }

    /// Stop the loop. Already-queued notes ring out; no new ones are scheduled.
    pub fn stop_music(&mut self) {
        self.music_playing = false;
    }

    /// Swap the active song. Takes effect from the next scheduled step, so a
    /// switch while playing is seamless (no gap, no restart of the audio clock).
    pub fn set_song(&mut self, spec: SongSpec) {
        self.song = spec;
        self.step = 0;
    }

    /// Select a song by index into [`SONGS`] (clamped) and start playing it.
    /// This is the primary entry point for the integrator: `play_song(floor)`
    /// via [`song_for_floor`], or a direct index from the `?viz` "Musics" tab.
    pub fn play_song(&mut self, index: usize) {
        let idx = index.min(SONGS.len().saturating_sub(1));
        self.set_song(SONGS[idx]);
        self.start_music();
    }

    /// Length of one sequencer step (seconds) for the current song's tempo.
    fn step_dur(&self) -> f64 {
        let spb = self.song.steps_per_beat.max(1) as f64;
        60.0 / self.song.bpm.max(1.0) / spb
    }

    /// Number of steps before the song's pattern loop repeats.
    fn loop_len(&self) -> usize {
        self.song
            .bass
            .len()
            .max(self.song.lead.len())
            .max(self.song.drums.len())
            .max(1)
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
        let loop_len = self.loop_len();
        while self.next_note_time < now + LOOKAHEAD {
            let step = self.step;
            let t = self.next_note_time;
            self.schedule_step(step, t);
            self.next_note_time += step_dur;
            self.step = (self.step + 1) % loop_len;
        }
    }

    /// Schedule one step of the current song (bass + lead + drums) at time `t`.
    fn schedule_step(&self, step: usize, t: f64) {
        let s = &self.song;
        let step_dur = self.step_dur();
        let gain = MUSIC_GAIN * s.intensity;

        if let Some(d) = degree_at(s.bass, step) {
            let f = degree_freq(s.root, s.scale, d);
            self.tone(f, f, t, step_dur * 1.9, gain * 1.3, s.bass_wave);
        }
        if let Some(d) = degree_at(s.lead, step) {
            let f = degree_freq(s.root, s.scale, d);
            self.tone(f, f, t, step_dur * 0.9, gain, s.lead_wave);
        }
        self.drum(drum_at(s.drums, step), t, gain);
    }

    /// Render one synthesized drum hit at absolute time `t`.
    fn drum(&self, hit: Drum, t: f64, gain: f64) {
        match hit {
            Silent => {}
            Kick => {
                self.tone(140.0, 45.0, t, 0.18, gain * 1.6, OscillatorType::Sine);
                self.noise(t, 0.05, gain * 0.4, BiquadFilterType::Lowpass, 400.0, 80.0);
            }
            Hat => {
                self.noise(
                    t,
                    0.03,
                    gain * 0.5,
                    BiquadFilterType::Highpass,
                    9000.0,
                    9000.0,
                );
            }
            Snare => {
                self.noise(
                    t,
                    0.13,
                    gain * 0.7,
                    BiquadFilterType::Highpass,
                    1800.0,
                    1400.0,
                );
                self.tone(220.0, 170.0, t, 0.10, gain * 0.5, OscillatorType::Triangle);
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

    /// A single enveloped oscillator tone. If `f0 != f1` the pitch glides
    /// (exponentially) from `f0` to `f1` over `dur` for glitchy dives/sweeps.
    /// Percussive attack (~5ms) then exponential decay to near-silence.
    fn tone(&self, f0: f64, f1: f64, start: f64, dur: f64, peak: f64, wave: OscillatorType) {
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
        let _ = g.exponential_ramp_to_value_at_time(peak.max(0.0002) as f32, start + 0.005);
        let _ = g.exponential_ramp_to_value_at_time(0.0001, start + dur);
        let _ = osc.connect_with_audio_node(&gain);
        if let Some(dest) = self.destination() {
            let _ = gain.connect_with_audio_node(&dest);
        }
        let sched: &web_sys::AudioScheduledSourceNode = osc.as_ref();
        let _ = sched.start_with_when(start);
        let _ = sched.stop_with_when(start + dur + 0.02);
    }

    /// A burst of the shared white-noise buffer through a sweeping biquad
    /// filter and a decaying gain envelope — used for hits, whooshes, cracks.
    fn noise(&self, start: f64, dur: f64, peak: f64, filter: BiquadFilterType, f0: f64, f1: f64) {
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
        if let Some(dest) = self.destination() {
            let _ = gain.connect_with_audio_node(&dest);
        }
        let sched: &web_sys::AudioScheduledSourceNode = src.as_ref();
        let _ = sched.start_with_when(start);
        let _ = sched.stop_with_when(start + dur + 0.02);
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
