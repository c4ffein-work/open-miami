//! The DRIVE — the glitchy synthwave ride home under the credits.
//!
//! A first-person OutRun-style scene: a banded dusk sky, a big cut-band sun
//! on the horizon, a dark road converging on the vanishing point with dashed
//! centre lines rushing at the camera, and palm silhouettes streaming past
//! on both sides, rasterized chunky on an art-pixel grid. The simulation
//! TEARS: horizontal slices displace sideways, colour channels split into
//! red/cyan ghosts, palms stutter on frozen time buckets, neon debris blocks
//! flash, and a faint digital rain shimmers in the sky. Everything loops
//! forever off `t` alone — all randomness is hashed time buckets, no RNG
//! state — so the scene is deterministic and replayable frame by frame.
//!
//! RENDERING IS ONE SHADER PASS (opcode 20 = DRIVE): every pixel of the
//! scene is computed in renderer.js's drive fragment shader,
//! shadertoy-style — this module only computes the schedules below each
//! frame and emits the opcode (`Graphics::drive`). It used to be built from
//! 2D primitives through pixel-art groups; that cost several full-screen
//! blended layers per frame and drowned fill-rate-limited GPUs.
//!
//! The emission needs the canvas so it is `cfg(target_arch = "wasm32")`; the
//! projection and glitch schedules below are plain math, unit-tested natively.

// ---------------------------------------------------------------------------
// Tunables — the scene geometry constants are MIRRORED in renderer.js's
// DRIVE shader (DRIVE_FS + its palm-placement JS); editing one side alone
// desyncs the native tests from the picture. `BANDS` and the schedule
// functions below are the live Rust side of the split.
// ---------------------------------------------------------------------------

/// Horizon height as a fraction of the screen height.
pub const HORIZON_FRAC: f32 = 0.44;
/// Camera speed, world units per second.
pub const SPEED: f32 = 13.0;
/// World length of one light/dark road stripe.
pub const STRIPE: f32 = 2.4;
/// World length of one centre-line dash cycle (dash + gap).
pub const DASH: f32 = 1.4;
/// Half the road width, world units.
pub const ROAD_HALF: f32 = 3.0;
/// Palms stand this far from the road centre, world units.
pub const PALM_X: f32 = 4.6;
/// World spacing between consecutive palms on one side.
pub const PALM_SPACING: f32 = 6.5;
/// Palm height, world units.
pub const PALM_H: f32 = 3.4;
/// The far clip: everything fogs out toward the horizon by here.
pub const Z_FAR: f32 = 36.0;
/// Pixels per world unit at z = 1, as a fraction of the screen width.
pub const PPU_FRAC: f32 = 0.14;
/// The screen is cut into this many horizontal tear slices.
pub const BANDS: usize = 9;

// ---------------------------------------------------------------------------
// Deterministic helpers (native-testable)
// ---------------------------------------------------------------------------

/// Stateless hash -> 0..1. All the scene's randomness derives from hashed
/// (bucket, salt) pairs, so equal `t` always renders the exact same frame.
pub fn hash01(a: u32, b: u32) -> f32 {
    let mut x = a
        .wrapping_mul(374_761_393)
        .wrapping_add(b.wrapping_mul(668_265_263));
    x = (x ^ (x >> 13)).wrapping_mul(1_274_126_177);
    ((x ^ (x >> 16)) & 0xff_ffff) as f32 / 0xff_ffff as f32
}

/// Screen y of world depth `z` (`z` >= 1; z = 1 is the bottom edge).
pub fn project_y(horizon: f32, h: f32, z: f32) -> f32 {
    horizon + (h - horizon) / z
}

/// World depth at screen row `y` (`y` strictly below the horizon).
pub fn z_at(horizon: f32, h: f32, y: f32) -> f32 {
    (h - horizon) / (y - horizon)
}

/// Sideways displacement of tear slice `band`, as a fraction of the screen
/// width: 0 almost always, bursts on ~90 ms buckets, plus rare violent
/// tears on ~50 ms buckets. (The violent tears were single-frame ~16 ms
/// buckets when the game ran at 20 fps — each shown frame then held one for
/// ~50 ms; this keeps that read at 60 fps.) Zero everywhere at `glitch` = 0.
pub fn band_offset_frac(t: f32, band: u32, glitch: f32) -> f32 {
    if glitch <= 0.0 {
        return 0.0;
    }
    let mut dx = 0.0;
    let b = (t / 0.09).floor() as u32;
    if hash01(b, 700 + band) < glitch * 0.22 {
        dx += (hash01(b, 900 + band) - 0.5) * 0.09 * (0.4 + glitch);
    }
    let f = (t / 0.05).floor() as u32;
    if hash01(f, 1300 + band) < glitch * 0.05 {
        dx += (hash01(f, 1500 + band) - 0.5) * 0.30;
    }
    dx
}

/// Red/cyan channel-split offset in fractions of the screen width (signed);
/// 0 outside bursts. Bursts live on ~110 ms buckets.
pub fn channel_split_frac(t: f32, glitch: f32) -> f32 {
    if glitch <= 0.0 {
        return 0.0;
    }
    let b = (t / 0.11).floor() as u32;
    if hash01(b, 41) < glitch * 0.30 {
        let sign = if hash01(b, 43) < 0.5 { -1.0 } else { 1.0 };
        sign * (0.004 + 0.010 * hash01(b, 47) * glitch)
    } else {
        0.0
    }
}

/// The credits' glitch ramp: the simulation stabilizes as CL4-UD3 gets away —
/// 0.8 at the start of the roll down to 0.15 after 20 s, then steady.
pub fn ending_glitch(credits_time: f32) -> f32 {
    0.8 - 0.65 * (credits_time / 20.0).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Drawing (canvas-only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod draw {
    use super::*;
    use crate::graphics::Graphics;

    /// Render the drive into the rect `(0, 0)..(w, h)` of the CURRENT
    /// transform (translate first to draw it elsewhere, e.g. a preview
    /// card). `t` is the loop clock in seconds, `glitch` 0..1 the tear
    /// intensity, `dim` 0..1 darkens the finished scene toward the menu's
    /// near-black inside the shader (the title passes 0.55; a preview 0).
    ///
    /// Since the shadertoy port this is ONE opcode: Rust computes the
    /// unit-tested glitch schedules (band tears, channel split) and the
    /// art-pixel size, and renderer.js's DRIVE fragment shader computes
    /// every pixel of the scene from them in a single opaque pass — no
    /// pixel groups, no stacked full-screen fills, so the cost on
    /// fill-rate-starved GPUs is one shaded write per pixel. The scene
    /// geometry (sky bands, cut-band sun, road rows, palms, debris) is
    /// mirrored from the tunables above into that shader.
    pub fn render_drive(g: &Graphics, w: f32, h: f32, t: f32, glitch: f32, dim: f32) {
        // Chunky art pixels, matched to the old pixel-group look.
        let px = 3.0f32.max((w / 1000.0).ceil()).max((h / 1000.0).ceil());
        let split = channel_split_frac(t, glitch) * w;
        let mut offs = [0.0f32; BANDS];
        for (i, o) in offs.iter_mut().enumerate() {
            *o = (band_offset_frac(t, i as u32, glitch) * w).clamp(-0.12 * w, 0.12 * w);
        }
        g.drive(w, h, t, glitch, split, px, dim, &offs);
    }
}

#[cfg(target_arch = "wasm32")]
pub use draw::render_drive;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_round_trips_and_is_ordered() {
        let (h, horizon) = (720.0, 720.0 * HORIZON_FRAC);
        // z = 1 is the bottom edge; deeper is higher on screen.
        assert!((project_y(horizon, h, 1.0) - h).abs() < 1e-3);
        let mut last = h + 1.0;
        for i in 1..40 {
            let z = i as f32;
            let y = project_y(horizon, h, z);
            assert!(y < last, "further must be higher on screen");
            assert!(y > horizon, "never above the horizon");
            assert!((z_at(horizon, h, y) - z).abs() < 1e-3, "round trip");
            last = y;
        }
    }

    #[test]
    fn glitch_schedules_are_deterministic_and_quiet_at_zero() {
        for i in 0..200 {
            let t = i as f32 * 0.037;
            for band in 0..BANDS as u32 {
                assert_eq!(band_offset_frac(t, band, 0.0), 0.0);
                // Same time, same band, same intensity -> the same frame.
                assert_eq!(
                    band_offset_frac(t, band, 0.7),
                    band_offset_frac(t, band, 0.7)
                );
                assert!(band_offset_frac(t, band, 1.0).abs() < 0.25);
            }
            assert_eq!(channel_split_frac(t, 0.0), 0.0);
            assert_eq!(channel_split_frac(t, 0.6), channel_split_frac(t, 0.6));
            assert!(channel_split_frac(t, 1.0).abs() < 0.02);
        }
    }

    #[test]
    fn glitches_actually_fire_at_high_intensity() {
        let mut tears = 0;
        let mut splits = 0;
        for i in 0..2000 {
            let t = i as f32 * 0.016;
            if (0..BANDS as u32).any(|b| band_offset_frac(t, b, 0.8) != 0.0) {
                tears += 1;
            }
            if channel_split_frac(t, 0.8) != 0.0 {
                splits += 1;
            }
        }
        assert!(tears > 50, "tears fire regularly: {tears}");
        assert!(splits > 20, "channel splits fire: {splits}");
    }

    #[test]
    fn ending_glitch_ramps_down_then_settles() {
        assert!((ending_glitch(0.0) - 0.8).abs() < 1e-6);
        assert!(ending_glitch(10.0) < ending_glitch(5.0));
        assert!((ending_glitch(20.0) - 0.15).abs() < 1e-6);
        assert!((ending_glitch(500.0) - 0.15).abs() < 1e-6);
    }

    #[test]
    fn hash01_stays_in_range() {
        for a in 0..300 {
            for b in 0..30 {
                let v = hash01(a, b);
                assert!((0.0..=1.0).contains(&v));
            }
        }
    }
}
