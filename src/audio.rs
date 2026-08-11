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

/// Tempo: 16th-note step length at ~130 BPM (60 / 130 / 4).
const STEP_DUR: f64 = 0.115_384;

/// Master music level — kept low so the looping backing never buries the SFX.
const MUSIC_GAIN: f64 = 0.07;

/// A-minor bassline, one note per 16th step (`0.0` = rest). Pulsing eighths
/// walking Am -> F -> G for that neon-noir drive.
const BASS: [f64; 16] = [
    55.00, 0.0, 55.00, 0.0, 55.00, 0.0, 55.00, 0.0, // Am (A1)
    43.65, 0.0, 43.65, 0.0, // F1
    49.00, 0.0, 49.00, 0.0, // G1
];

/// Fast arpeggio on top, one note per 16th step, tracing the same chords.
const ARP: [f64; 16] = [
    440.00, 523.25, 659.25, 523.25, // Am: A C E C
    440.00, 659.25, 523.25, 659.25, //
    349.23, 440.00, 523.25, 440.00, // F:  F A C A
    392.00, 493.88, 587.33, 493.88, // G:  G B D B
];

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
    /// Current index into the 16-step loop.
    step: usize,
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

    /// Begin the looping backing track (idempotent).
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
        while self.next_note_time < now + LOOKAHEAD {
            let step = self.step;
            let t = self.next_note_time;
            self.schedule_step(step, t);
            self.next_note_time += STEP_DUR;
            self.step = (self.step + 1) % 16;
        }
    }

    /// Schedule one 16th-note step of bass + arp at absolute time `t`.
    fn schedule_step(&self, step: usize, t: f64) {
        let bass = BASS[step];
        if bass > 0.0 {
            self.tone(
                bass,
                bass,
                t,
                STEP_DUR * 1.9,
                MUSIC_GAIN * 1.3,
                OscillatorType::Square,
            );
        }
        let arp = ARP[step];
        self.tone(
            arp,
            arp,
            t,
            STEP_DUR * 0.9,
            MUSIC_GAIN,
            OscillatorType::Triangle,
        );
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
