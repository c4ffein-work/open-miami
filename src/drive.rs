//! The DRIVE — the glitchy synthwave ride home under the credits.
//!
//! A first-person OutRun-style scene built entirely from the existing 2D
//! primitives: a banded dusk sky, a big cut-band sun on the horizon, a dark
//! road converging on the vanishing point with dashed centre lines rushing at
//! the camera, and palm silhouettes streaming past on both sides. The whole
//! frame is rasterized chunky through pixel-art groups (opcodes 15/16).
//!
//! Then the simulation TEARS: horizontal slices of the scene displace
//! sideways (each slice is its own pixel group, offset at `pixel_end`),
//! colour channels split into red/cyan ghost passes, palms stutter on frozen
//! time buckets, neon debris blocks flash, and a faint digital rain shimmers
//! in the sky. Everything loops forever off `t` alone — all randomness is
//! hashed time buckets, no RNG state — so the scene is deterministic and
//! replayable frame by frame.
//!
//! Drawing needs the canvas so it is `cfg(target_arch = "wasm32")`; the
//! projection and glitch schedules below are plain math, unit-tested natively.

// ---------------------------------------------------------------------------
// Tunables
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
/// single-frame tears on ~16 ms buckets. Zero everywhere at `glitch` = 0.
pub fn band_offset_frac(t: f32, band: u32, glitch: f32) -> f32 {
    if glitch <= 0.0 {
        return 0.0;
    }
    let mut dx = 0.0;
    let b = (t / 0.09).floor() as u32;
    if hash01(b, 700 + band) < glitch * 0.22 {
        dx += (hash01(b, 900 + band) - 0.5) * 0.09 * (0.4 + glitch);
    }
    let f = (t / 0.016).floor() as u32;
    if hash01(f, 1300 + band) < glitch * 0.02 {
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
    use crate::math::{Color, Vec2};
    use std::f32::consts::PI;

    /// Screen rows the ground/road is rasterized in.
    const ROWS: usize = 48;

    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    fn mix(a: Color, b: Color, t: f32) -> Color {
        Color::new(
            lerp(a.r, b.r, t),
            lerp(a.g, b.g, t),
            lerp(a.b, b.b, t),
            lerp(a.a, b.a, t),
        )
    }

    fn with_a(c: Color, a: f32) -> Color {
        Color::new(c.r, c.g, c.b, a)
    }

    /// Render the drive into the rect `(0, 0)..(w, h)` of the CURRENT
    /// transform (translate first to draw it elsewhere, e.g. a preview card
    /// — the pixel groups clip the scene to that rect for free). `t` is the
    /// loop clock in seconds, `glitch` 0..1 the tear intensity.
    pub fn render_drive(g: &Graphics, w: f32, h: f32, t: f32, glitch: f32) {
        // Chunky art pixels; keep every group under the 1024-texel cap.
        let px = 3.0f32.max((w / 1000.0).ceil()).max((h / 1000.0).ceil());
        // Backing plate: the gap a displaced slice leaves shows this void.
        g.draw_rectangle(Vec2::new(0.0, 0.0), w, h, Color::new(0.01, 0.0, 0.03, 1.0));
        let split = channel_split_frac(t, glitch) * w;
        let mut offs = [0.0f32; BANDS];
        for (i, o) in offs.iter_mut().enumerate() {
            *o = (band_offset_frac(t, i as u32, glitch) * w).clamp(-0.12 * w, 0.12 * w);
        }
        if offs.iter().all(|d| d.abs() < 1.0) {
            // Calm frame: one full-screen pixel group.
            g.pixel_begin(px, w, h);
            scene_passes(g, w, h, t, glitch, split);
            g.pixel_end(0.0, 0.0);
        } else {
            // Torn frame: each horizontal slice is its own pixel group — the
            // full scene is drawn into each (the group's texel region clips
            // it), then the slice is placed with its sideways offset.
            let bh = h / BANDS as f32;
            for (i, &dx) in offs.iter().enumerate() {
                let y0 = (i as f32 * bh).floor();
                g.pixel_begin(px, w, bh + px);
                g.save();
                g.translate(0.0, -y0);
                scene_passes(g, w, h, t, glitch, split);
                g.restore();
                g.pixel_end(dx, y0);
            }
        }
    }

    /// The scene, plus — during a channel-split burst — red and cyan ghost
    /// passes offset sideways, and the neon debris blocks on top.
    fn scene_passes(g: &Graphics, w: f32, h: f32, t: f32, glitch: f32, split: f32) {
        scene(g, w, h, t, glitch, None);
        if split.abs() >= 0.75 {
            g.save();
            g.translate(-split, 0.0);
            scene(
                g,
                w,
                h,
                t,
                glitch,
                Some((Color::new(1.0, 0.12, 0.25, 1.0), 0.26)),
            );
            g.restore();
            g.save();
            g.translate(split, 0.0);
            scene(
                g,
                w,
                h,
                t,
                glitch,
                Some((Color::new(0.10, 0.90, 1.0, 1.0), 0.26)),
            );
            g.restore();
        }
        debris(g, w, h, t, glitch);
    }

    /// One pass of the scene. `ghost` = None for the real pass; a ghost pass
    /// multiplies every colour by the mask and alpha (cheap channel split)
    /// and skips the detail layers (stars, rain).
    fn scene(g: &Graphics, w: f32, h: f32, t: f32, glitch: f32, ghost: Option<(Color, f32)>) {
        let detail = ghost.is_none();
        let cc = |c: Color| match ghost {
            None => c,
            Some((m, a)) => Color::new(c.r * m.r, c.g * m.g, c.b * m.b, c.a * a),
        };
        let horizon = h * HORIZON_FRAC;
        let ppu = w * PPU_FRAC;

        // --- Sky: banded dusk gradient ---
        let sky_top = Color::new(0.06, 0.02, 0.13, 1.0);
        let sky_mid = Color::new(0.30, 0.06, 0.34, 1.0);
        let sky_low = Color::new(0.86, 0.24, 0.33, 1.0);
        let bands = 22;
        for i in 0..bands {
            let f0 = i as f32 / bands as f32;
            let f1 = (i + 1) as f32 / bands as f32;
            let c = if f0 < 0.55 {
                mix(sky_top, sky_mid, f0 / 0.55)
            } else {
                mix(sky_mid, sky_low, (f0 - 0.55) / 0.45)
            };
            g.draw_rectangle(
                Vec2::new(0.0, f0 * horizon),
                w,
                (f1 - f0) * horizon + 1.0,
                cc(c),
            );
        }

        // --- Sun: warm glow + banded disc with growing cuts, on the horizon ---
        let sr = h * 0.21;
        let sc = Vec2::new(w * 0.5, horizon - sr * 0.28);
        if detail {
            g.draw_circle(sc, sr * 1.9, cc(Color::new(1.0, 0.45, 0.35, 0.10)));
            g.draw_circle(sc, sr * 1.4, cc(Color::new(1.0, 0.55, 0.35, 0.12)));
        }
        // Stars + digital rain live behind the sun disc but over the glow.
        if detail {
            for i in 0..60u32 {
                let sx = hash01(i, 11) * w;
                let sy = hash01(i, 12) * horizon * 0.8;
                let tw = 0.5 + 0.5 * (t * (0.8 + hash01(i, 13) * 2.2) + i as f32).sin();
                g.draw_circle(
                    Vec2::new(sx, sy),
                    0.7 + hash01(i, 14) * 1.1,
                    Color::new(1.0, 0.95, 1.0, 0.5 * tw * (1.0 - sy / horizon)),
                );
            }
            // A faint digital-rain shimmer — the sky is being rendered.
            let ra = 0.05 + 0.18 * glitch;
            for i in 0..22u32 {
                let cx = hash01(i, 31) * w;
                let sp = 26.0 + hash01(i, 32) * 70.0;
                let head = (t * sp + hash01(i, 33) * 600.0) % (horizon + 40.0) - 20.0;
                for j in 0..4u32 {
                    let yy = head - j as f32 * 7.0;
                    if yy > 0.0 && yy < horizon {
                        g.draw_rectangle(
                            Vec2::new(cx, yy),
                            2.0,
                            5.0,
                            Color::new(0.35, 1.0, 0.65, ra * (1.0 - j as f32 * 0.22)),
                        );
                    }
                }
            }
        }
        let sun_top = Color::new(1.0, 0.88, 0.28, 1.0);
        let sun_bot = Color::new(1.0, 0.22, 0.52, 1.0);
        // Occasionally the sun's bands slide sideways — the sky is data too.
        let sb = (t / 0.12).floor() as u32;
        let sun_glitch = hash01(sb, 399) < glitch * 0.3;
        let slices = 26;
        for i in 0..slices {
            let f0 = i as f32 / slices as f32;
            let f1 = (i + 1) as f32 / slices as f32;
            let y0 = sc.y - sr + f0 * 2.0 * sr;
            let y1 = sc.y - sr + f1 * 2.0 * sr;
            if y0 >= horizon {
                break;
            }
            let ym = (y0 + y1) / 2.0;
            let dy = (ym - sc.y).abs();
            if dy >= sr {
                continue;
            }
            let half = (sr * sr - dy * dy).sqrt();
            let c = mix(sun_top, sun_bot, f0);
            let cut = if f0 > 0.45 {
                (f0 - 0.45) / 0.55 * 0.55
            } else {
                0.0
            };
            let hh = (y1.min(horizon) - y0) * (1.0 - cut);
            let dxs = if sun_glitch {
                (hash01(sb, 400 + i) - 0.5) * 14.0 * glitch
            } else {
                0.0
            };
            g.draw_rectangle(
                Vec2::new(sc.x - half + dxs, y0),
                half * 2.0,
                hh.max(1.0),
                cc(c),
            );
        }
        // Horizon glow line.
        g.draw_rectangle(
            Vec2::new(0.0, horizon - 1.0),
            w,
            2.0,
            cc(Color::new(1.0, 0.42, 0.70, 0.9)),
        );

        // --- Ground + road, rasterized in screen rows ---
        for i in 0..ROWS {
            let y0 = horizon + (h - horizon) * i as f32 / ROWS as f32;
            let y1 = horizon + (h - horizon) * (i + 1) as f32 / ROWS as f32;
            let bh = y1 - y0 + 0.5;
            let z = z_at(horizon, h, (y0 + y1) * 0.5).min(400.0);
            let fog = (z / Z_FAR).clamp(0.0, 1.0).powf(1.2);
            let p = z + t * SPEED;
            let alt = (p / STRIPE).floor() as i64 % 2 == 0;
            let gnd = if alt {
                Color::new(0.060, 0.025, 0.100, 1.0)
            } else {
                Color::new(0.045, 0.015, 0.085, 1.0)
            };
            let gc = mix(gnd, Color::new(0.10, 0.035, 0.13, 1.0), fog);
            g.draw_rectangle(Vec2::new(0.0, y0), w, bh, cc(gc));
            let half = ROAD_HALF * ppu / z;
            if half > 1.0 {
                let rc0 = if alt {
                    Color::new(0.130, 0.075, 0.190, 1.0)
                } else {
                    Color::new(0.100, 0.055, 0.155, 1.0)
                };
                let rc = mix(rc0, Color::new(0.11, 0.045, 0.145, 1.0), fog);
                g.draw_rectangle(Vec2::new(w * 0.5 - half, y0), half * 2.0, bh, cc(rc));
                // Edge lines, alternating hot pink / pale.
                let ew = (half * 0.055).max(1.2);
                let ec0 = if alt {
                    Color::new(1.0, 0.32, 0.62, 1.0)
                } else {
                    Color::new(0.95, 0.90, 0.95, 1.0)
                };
                let ec = with_a(
                    mix(ec0, Color::new(0.5, 0.2, 0.4, 1.0), fog),
                    1.0 - fog * 0.6,
                );
                g.draw_rectangle(Vec2::new(w * 0.5 - half, y0), ew, bh, cc(ec));
                g.draw_rectangle(Vec2::new(w * 0.5 + half - ew, y0), ew, bh, cc(ec));
                // Centre dashes rushing at the camera.
                if (p / DASH).floor() as i64 % 2 == 0 {
                    let cw = (half * 0.045).max(1.0);
                    g.draw_rectangle(
                        Vec2::new(w * 0.5 - cw * 0.5, y0),
                        cw,
                        bh,
                        cc(Color::new(0.98, 0.92, 0.72, 0.9 - fog * 0.7)),
                    );
                }
            }
        }

        // --- Palms streaming past on both sides, far to near ---
        let slots = 12;
        for i in (0..slots).rev() {
            for side in [-1.0f32, 1.0] {
                // The right rank is offset half a spacing so palms alternate.
                let phase = if side > 0.0 { 0.5 } else { 0.0 };
                // Stable physical id: which palm currently occupies slot `i`.
                let travelled = t * SPEED / PALM_SPACING + phase;
                let pid = (travelled.floor() as i64 + i as i64) as u32 * 2
                    + if side > 0.0 { 1 } else { 0 };
                // Stutter: on hashed ~130 ms buckets a palm freezes on the
                // bucket's start time, then snaps forward — it skips frames.
                let bkt = (t / 0.13).floor();
                let te = if hash01(pid, 505 + bkt as u32) < glitch * 0.4 {
                    bkt * 0.13
                } else {
                    t
                };
                let off = (te * SPEED / PALM_SPACING + phase).fract();
                let z = (i as f32 + 1.0 - off) * PALM_SPACING;
                if !(1.05..=Z_FAR).contains(&z) {
                    continue;
                }
                let s = 1.0 / z;
                let yb = project_y(horizon, h, z);
                let xb = w * 0.5 + side * PALM_X * ppu * s * (1.0 + 0.12 * hash01(pid, 61));
                let ht = PALM_H * ppu * s * (0.8 + 0.4 * hash01(pid, 62));
                if ht < 3.0 {
                    continue;
                }
                let fog = (z / Z_FAR).powf(1.3);
                let col = cc(mix(
                    Color::new(0.050, 0.015, 0.090, 1.0),
                    Color::new(0.55, 0.16, 0.30, 1.0),
                    fog * 0.8,
                ));
                let lean = -side * 0.10 + (hash01(pid, 63) - 0.5) * 0.24;
                // Trunk: three tapering segments curving into the lean.
                let (mut px0, mut py0) = (xb, yb);
                let (mut tx, mut ty) = (xb, yb);
                for seg in 1..=3 {
                    let f = seg as f32 / 3.0;
                    tx = xb + lean * ht * f.powf(1.6);
                    ty = yb - ht * f;
                    let th = (ht * 0.050 * (1.0 - 0.5 * f)).max(1.0);
                    g.draw_line(Vec2::new(px0, py0), Vec2::new(tx, ty), th, col);
                    (px0, py0) = (tx, ty);
                }
                // Crown: drooping fronds fanned across the top.
                let sway = (t * 1.1 + pid as f32).sin() * 0.05;
                let nf = 7;
                for k in 0..nf {
                    let a = -PI * (0.12 + 0.76 * k as f32 / (nf - 1) as f32)
                        + sway
                        + (hash01(pid, 70 + k) - 0.5) * 0.12;
                    let len = ht * (0.38 + 0.10 * hash01(pid, 80 + k));
                    let (mx, my) = (tx + a.cos() * len * 0.6, ty + a.sin() * len * 0.6);
                    let a2 = if a.cos() >= 0.0 { a + 0.7 } else { a - 0.7 };
                    let (ex, ey) = (mx + a2.cos() * len * 0.5, my + a2.sin() * len * 0.5);
                    let th = (ht * 0.022).max(1.0);
                    g.draw_line(Vec2::new(tx, ty), Vec2::new(mx, my), th, col);
                    g.draw_line(
                        Vec2::new(mx, my),
                        Vec2::new(ex, ey),
                        (th * 0.8).max(1.0),
                        col,
                    );
                }
                g.draw_circle(Vec2::new(tx, ty), (ht * 0.045).max(1.5), col);
            }
        }
    }

    /// Neon debris: on hashed ~100 ms buckets, a handful of cyan / magenta /
    /// white blocks flash at random positions — corrupted tiles.
    fn debris(g: &Graphics, w: f32, h: f32, t: f32, glitch: f32) {
        if glitch <= 0.0 {
            return;
        }
        let b = (t / 0.10).floor() as u32;
        if hash01(b, 611) >= glitch * 0.5 {
            return;
        }
        let n = 2 + (hash01(b, 612) * 5.0) as u32;
        for i in 0..n {
            let c = match (hash01(b, 780 + i) * 3.0) as u32 {
                0 => Color::new(0.2, 0.95, 1.0, 1.0),
                1 => Color::new(1.0, 0.25, 0.85, 1.0),
                _ => Color::new(0.95, 0.95, 1.0, 1.0),
            };
            g.draw_rectangle(
                Vec2::new(hash01(b, 700 + i) * w, hash01(b, 720 + i) * h),
                4.0 + hash01(b, 740 + i) * 50.0,
                2.0 + hash01(b, 760 + i) * 8.0,
                with_a(c, 0.25 + 0.35 * hash01(b, 790 + i)),
            );
        }
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
