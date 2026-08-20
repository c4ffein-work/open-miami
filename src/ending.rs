//! The ending — everything that happens once the last car goes up.
//!
//! Timeline (all driven from `update_game` in lib.rs):
//!   1. the player extracts through a `"to": "surface"` exit (`scenario::SURFACE_EXIT`) → the EXFILTRATE card
//!      ([`draw_extract_card`], also used on every ordinary floor)
//!   2. the [`Outro`] takes over: the card fades, the floor's `extracted`
//!      scenario step talks (13½: the UPLINK epilogue) until the comms feed
//!      goes idle, then a BLUR-OUT (POSTFX kind 0) dissolves the frame
//!   3. `GameScreen::Ending`: the [`CREDITS`] roll over the ELEVATOR RIDE
//!      home ([`render_ride`]: the car interior top-down, CL4-UD3 idling at
//!      the centre, shaft lights rushing past — smeared into radial light
//!      trails by POSTFX kind 10, strength [`Ending::warp_t`]), Enter / Esc
//!      back to the level select.
//!
//! The credits text is the plain [`CREDITS`] list below — edit freely.
//! Everything that needs the canvas is behind `cfg(target_arch = "wasm32")`;
//! the timeline and layout are plain data so they are unit-tested natively.

use crate::math::Color;
#[cfg(target_arch = "wasm32")]
use crate::math::Vec2;

// ---------------------------------------------------------------------------
// The credits text
// ---------------------------------------------------------------------------

/// One line of the credits roll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Line {
    /// The big pink title.
    Title(&'static str),
    /// A coral section heading.
    Head(&'static str),
    /// A plain white line.
    Text(&'static str),
    /// A smaller grey note.
    Dim(&'static str),
    /// The closing LORE line (large, coral, held at the end).
    Quote(&'static str),
    /// Vertical breathing room.
    Gap,
}

/// The credits, top to bottom. Edit this list to change the roll.
pub const CREDITS: &[Line] = &[
    Line::Title("OPEN MIAMI"),
    Line::Head("// ROGUE PURGE"),
    Line::Gap,
    Line::Gap,
    Line::Head("THANK YOU"),
    Line::Text("for walking in the front door,"),
    Line::Text("going all the way down,"),
    Line::Text("and coming back valid."),
    Line::Gap,
    Line::Gap,
    Line::Head("HOW THIS WAS MADE"),
    Line::Text("A conversation between c4ffein and Claude."),
    Line::Dim("one human with taste and a keyboard, one model, a swarm of subagents"),
    Line::Gap,
    Line::Text("A Rust + wasm engine that only simulates."),
    Line::Text("One flat command stream per frame to a WebGL renderer."),
    Line::Text("Live 3D -> 2D robots and a shoggoth, every frame, no cache."),
    Line::Text("A level + scenario editor writing the floors as JSON."),
    Line::Text("Music and every sound effect synthesized procedurally,"),
    Line::Text("measured against free recording packs."),
    Line::Gap,
    Line::Gap,
    Line::Head("HOMAGES"),
    Line::Text("Hotline Miami - Dennaton Games."),
    Line::Dim("the whole genre debt: top-down, one hit, neon, the mask."),
    Line::Text("The DOOM lineage."),
    Line::Dim("rip, tear, and the fine art of the pixelated corridor."),
    Line::Gap,
    Line::Gap,
    Line::Head("ASSETS // CREDITS"),
    Line::Text("\"Snake's Authentic Gun Sounds\" (free pack)"),
    Line::Dim("used only as a measurement reference for the synthesized guns"),
    Line::Text("\"Bullet Impact Body Concrete Metal Flyby\" pack (free)"),
    Line::Dim("measurement reference for the impacts"),
    Line::Text("VT323 by Peter Hull - SIL Open Font License"),
    Line::Text("Rust / wasm-bindgen / web-sys"),
    Line::Text("Playwright + Bun for the tests"),
    Line::Gap,
    Line::Gap,
    Line::Dim("Open Miami // Rogue Purge is a fan project - neon-noir tone,"),
    Line::Dim("not affiliated with Hotline Miami or its creators."),
    Line::Gap,
    Line::Gap,
    Line::Gap,
    Line::Quote("MY MASK NEVER COMES OFF."),
];

impl Line {
    /// The text of the line (empty for a gap).
    pub fn text(&self) -> &'static str {
        match *self {
            Line::Title(t) | Line::Head(t) | Line::Text(t) | Line::Dim(t) | Line::Quote(t) => t,
            Line::Gap => "",
        }
    }

    /// Font size in px.
    pub fn size(&self) -> f32 {
        match self {
            Line::Title(_) => 64.0,
            Line::Head(_) => 28.0,
            Line::Text(_) => 22.0,
            Line::Dim(_) => 16.0,
            Line::Quote(_) => 34.0,
            Line::Gap => 0.0,
        }
    }

    /// Vertical advance in px.
    pub fn height(&self) -> f32 {
        match self {
            Line::Title(_) => 74.0,
            Line::Head(_) => 40.0,
            Line::Text(_) => 30.0,
            Line::Dim(_) => 24.0,
            Line::Quote(_) => 50.0,
            Line::Gap => 22.0,
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Line::Title(_) => Color::new(1.0, 0.09, 0.26, 1.0),
            Line::Head(_) => Color::from_rgba(217, 119, 87, 255),
            Line::Text(_) => Color::new(0.96, 0.94, 1.0, 1.0),
            Line::Dim(_) => Color::new(0.68, 0.64, 0.78, 1.0),
            Line::Quote(_) => Color::from_rgba(255, 111, 97, 255),
            Line::Gap => Color::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

/// Total height of the roll in px.
pub fn credits_height() -> f32 {
    CREDITS.iter().map(Line::height).sum()
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

/// Seconds the "EXFILTRATED // FLOOR N" card stays up after the player
/// extracts, before the next floor loads (or the outro starts).
pub const EXTRACT_CARD_SECS: f32 = 2.4;
/// The card fades out over this long once the outro starts.
pub const CARD_FADE_SECS: f32 = 0.6;
/// The uplink phase ends this long after the comms feed goes idle...
pub const UPLINK_HOLD_SECS: f32 = 4.5;
/// ...but never before this (so an empty epilogue still breathes)...
pub const UPLINK_MIN_SECS: f32 = 3.0;
/// ...and never later than this (a stuck feed cannot hold the ending hostage).
pub const UPLINK_MAX_SECS: f32 = 60.0;
/// The blur-out (POSTFX kind 0, t 0 -> 1) duration.
pub const BLUR_OUT_SECS: f32 = 2.5;
/// The colour the blur-out dissolves into = the credits' sky at the top, so
/// the cut to the credits is seamless.
pub const BLUR_COLOR: Color = Color::new(0.05, 0.02, 0.10, 1.0);
/// Credits scroll speed, px/s.
pub const CREDITS_PX_PER_SEC: f32 = 36.0;
/// The credits fade in from [`BLUR_COLOR`] over this long.
pub const CREDITS_FADE_IN_SECS: f32 = 0.8;
/// The warp-trail post pass (POSTFX kind 10) ramps in over this long — the
/// ride home accelerating...
pub const WARP_RAMP_SECS: f32 = 6.0;
/// ...holds at full strength, and eases down over the last seconds of scroll
/// before the roll settles on the closing quote...
pub const WARP_EASE_SECS: f32 = 6.0;
/// ...down to this idle strength (the car keeps gliding, gently).
pub const WARP_IDLE_T: f32 = 0.30;
/// The trail tint: a cold shaft-light cyan (POSTFX 10's colour params).
pub const WARP_TINT: Color = Color::new(0.55, 0.80, 1.0, 1.0);

/// Distance travelled up the shaft, in light-cycle units: the ride starts at
/// 35% speed and accelerates linearly to full over [`WARP_RAMP_SECS`], so the
/// passing lights speed up together with the trails. Monotonic in `time`.
pub fn ride_phase(time: f32) -> f32 {
    let a = time.clamp(0.0, WARP_RAMP_SECS);
    0.35 * a + 0.65 * a * a / (2.0 * WARP_RAMP_SECS) + (time.max(0.0) - a)
}

/// The post-card part of the ending, on the last floor: the uplink epilogue
/// (comms), then the blur-out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outro {
    /// The uplink is back; the `extracted` step's comms play. `time` since
    /// the outro began, `idle_for` seconds the feed has been idle.
    Uplink { time: f32, idle_for: f32 },
    /// The frame dissolves. `time` since the blur began.
    Blur { time: f32 },
}

impl Outro {
    pub fn new() -> Self {
        Outro::Uplink {
            time: 0.0,
            idle_for: 0.0,
        }
    }

    /// Advance by `dt`. `feed_idle` = the comms feed has nothing queued or
    /// typing. Returns `true` once the blur-out has finished (→ credits).
    pub fn tick(&mut self, dt: f32, feed_idle: bool) -> bool {
        match self {
            Outro::Uplink { time, idle_for } => {
                *time += dt;
                if feed_idle {
                    *idle_for += dt;
                } else {
                    *idle_for = 0.0;
                }
                if (*time >= UPLINK_MIN_SECS && *idle_for >= UPLINK_HOLD_SECS)
                    || *time >= UPLINK_MAX_SECS
                {
                    *self = Outro::Blur { time: 0.0 };
                }
                false
            }
            Outro::Blur { time } => {
                *time += dt;
                *time >= BLUR_OUT_SECS
            }
        }
    }

    /// Opacity of the extraction card (fades out at the start of the outro).
    pub fn card_alpha(&self) -> f32 {
        match self {
            Outro::Uplink { time, .. } => (1.0 - time / CARD_FADE_SECS).clamp(0.0, 1.0),
            Outro::Blur { .. } => 0.0,
        }
    }

    /// The blur-out strength 0..1 while dissolving, `None` before.
    pub fn blur_t(&self) -> Option<f32> {
        match self {
            Outro::Uplink { .. } => None,
            Outro::Blur { time } => Some((time / BLUR_OUT_SECS).clamp(0.0, 1.0)),
        }
    }
}

impl Default for Outro {
    fn default() -> Self {
        Self::new()
    }
}

/// The credits screen state: elapsed seconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ending {
    pub time: f32,
}

impl Ending {
    pub fn new() -> Self {
        Ending { time: 0.0 }
    }

    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
    }

    /// The resting scroll offset for a screen `h` px tall: the closing quote
    /// sits at the vertical centre.
    pub fn max_scroll(h: f32) -> f32 {
        let last_h = CREDITS.last().map(Line::height).unwrap_or(0.0);
        (h + 40.0 + credits_height() - last_h - h * 0.5).max(0.0)
    }

    /// Scroll offset in px: the roll enters from the bottom and stops at
    /// [`Self::max_scroll`].
    pub fn scroll(&self, h: f32) -> f32 {
        (self.time * CREDITS_PX_PER_SEC).min(Self::max_scroll(h))
    }

    /// Whether the roll has reached its resting position.
    pub fn settled(&self, h: f32) -> bool {
        self.time * CREDITS_PX_PER_SEC >= Self::max_scroll(h)
    }

    /// POSTFX kind 10 strength for a screen `h` px tall: smooth ramp 0 -> 1
    /// over the first [`WARP_RAMP_SECS`] (the ride accelerating), hold, then
    /// an ease down to [`WARP_IDLE_T`] over the last [`WARP_EASE_SECS`] of
    /// scroll before the roll settles on the quote.
    pub fn warp_t(&self, h: f32) -> f32 {
        fn smooth(x: f32) -> f32 {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        }
        let ramp = smooth(self.time / WARP_RAMP_SECS);
        let left_secs = (Self::max_scroll(h) / CREDITS_PX_PER_SEC - self.time).max(0.0);
        let settle = smooth(left_secs / WARP_EASE_SECS);
        ramp * (WARP_IDLE_T + (1.0 - WARP_IDLE_T) * settle)
    }
}

// ---------------------------------------------------------------------------
// Drawing (canvas-only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod draw {
    use super::*;
    use crate::graphics::Graphics;

    /// Approximate VT323 advance as a fraction of the font size (the renderer
    /// measures the real glyphs; this is only for centring).
    const CHAR_W: f32 = 0.42;

    /// Draw `text` centred on `cx` at baseline `y`.
    fn text_centered(g: &Graphics, text: &str, cx: f32, y: f32, size: f32, color: Color) {
        let w = text.chars().count() as f32 * size * CHAR_W;
        g.draw_text(text, Vec2::new(cx - w / 2.0, y), size, color);
    }

    /// The "EXFILTRATED // FLOOR N" completion card: the title reveals left
    /// to right over the first second, then the sub-line wobbles in.
    /// `t` = seconds since extraction, `alpha` = overall opacity (the outro
    /// fades it), `home` = this car goes to the surface.
    pub fn draw_extract_card(g: &Graphics, floor_title: &str, t: f32, alpha: f32, home: bool) {
        if alpha <= 0.0 {
            return;
        }
        let (w, h) = (g.width(), g.height());
        // A dark band behind the card so it reads over the elevator labels.
        g.draw_rectangle(
            Vec2::new(0.0, h / 2.0 - 90.0),
            w,
            220.0,
            Color::new(0.0, 0.0, 0.0, 0.55 * alpha),
        );
        let message = format!("EXFILTRATED // {floor_title}");
        let reveal = (t / 1.0).min(1.0);
        let n = message.chars().count();
        let shown = ((n as f32 * reveal) as usize).min(n);
        let revealed: String = message.chars().take(shown).collect();
        let size = 56.0;
        // Centre on the FULL message so the reveal doesn't slide.
        let full_w = n as f32 * size * CHAR_W;
        g.draw_text(
            &revealed,
            Vec2::new(w / 2.0 - full_w / 2.0, h / 2.0),
            size,
            Color::new(0.0, 1.0, 0.0, alpha),
        );
        if t > 1.0 {
            let anim = t - 1.0;
            let y_off = 5.0 * (anim * 1.5 * 2.0 * std::f32::consts::PI).sin();
            let sub = if home { "GOING HOME" } else { "EXFILTRATING" };
            text_centered(
                g,
                sub,
                w / 2.0,
                h / 2.0 + 80.0 + y_off,
                30.0,
                Color::new(1.0, 1.0, 1.0, alpha),
            );
        }
    }

    /// Cheap deterministic 0..1 hash (per-light parameters).
    fn fr(seed: f32) -> f32 {
        let v = (seed * 12.9898).sin() * 43758.547;
        v - v.floor()
    }

    /// The RIDE HOME backdrop under the credits: a dark shaft rushing by,
    /// the elevator car interior seen top-down at dead centre with CL4-UD3
    /// idling in it, and sparse bright shaft lights streaking outward on
    /// fixed bearings — the raw material POSTFX kind 10 (WARP TRAILS)
    /// smears into radial long-exposure light trails. The caller draws
    /// [`draw_credits`] over this and closes the frame with
    /// `postfx(10, ending.warp_t(h), WARP_TINT)`.
    pub fn render_ride(g: &Graphics, ending: &Ending) {
        let (w, h) = (g.width(), g.height());
        let (cx, cy) = (w / 2.0, h / 2.0);
        let t = ending.time;
        // The shaft: near-black violet with a faint glow well at the centre
        // (the vanishing point the trails stream away from).
        g.draw_rectangle(
            Vec2::new(0.0, 0.0),
            w,
            h,
            Color::new(0.015, 0.008, 0.040, 1.0),
        );
        for (r, a) in [(300.0, 0.05), (180.0, 0.06), (90.0, 0.08)] {
            g.draw_circle(Vec2::new(cx, cy), r, Color::new(0.10, 0.12, 0.30, a));
        }
        // Passing shaft lights: each lives on a fixed bearing (golden-angle
        // spread) and rushes outward exponentially, drawn as a short radial
        // streak that grows brighter and longer as it closes on the border.
        let phase = ride_phase(t);
        let rmax = (cx * cx + cy * cy).sqrt() * 1.05;
        let r0 = 120.0;
        for i in 0..56u32 {
            let fi = i as f32;
            let ang = fi * 2.399_963; // golden angle
            let rate = 0.24 + 0.50 * fr(fi + 0.17);
            let k = (phase * rate + fr(fi + 3.7)).fract();
            let curve = ((2.4 * k).exp_m1()) / 2.4f32.exp_m1();
            let r = r0 + curve * (rmax - r0);
            let len = 8.0 + 90.0 * k * k;
            let (dx, dy) = (ang.cos(), ang.sin());
            let tip = Vec2::new(cx + dx * r, cy + dy * r);
            let tail = Vec2::new(cx + dx * (r - len), cy + dy * (r - len));
            let a = (k / 0.12).min(1.0); // fade in at spawn, no pop
            let c = match i % 4 {
                0 => Color::new(0.10, 0.90, 1.00, a),
                1 => Color::new(1.00, 0.15, 0.85, a),
                2 => Color::new(0.25, 0.55, 1.00, a),
                _ => Color::new(1.00, 0.70, 0.05, a * 0.9),
            };
            g.draw_line(tail, tip, 2.0 + 3.0 * k, c);
        }
        draw_car(g, cx, cy, t);
    }

    /// The car interior, top-down, centred: floor plate, frame, corner
    /// posts, the door seam, a call panel with a blinking ride light — and
    /// the live coral robot idling in the middle of it.
    fn draw_car(g: &Graphics, cx: f32, cy: f32, t: f32) {
        let (cw, ch) = (176.0, 196.0);
        let (x0, y0) = (cx - cw / 2.0, cy - ch / 2.0);
        // The shaft void hugging the car (a soft drop shadow ring).
        g.draw_rectangle(
            Vec2::new(x0 - 14.0, y0 - 14.0),
            cw + 28.0,
            ch + 28.0,
            Color::new(0.0, 0.0, 0.0, 0.55),
        );
        // Floor plate + brushed tread lines.
        g.draw_rectangle(Vec2::new(x0, y0), cw, ch, Color::new(0.13, 0.14, 0.19, 1.0));
        for i in 1..7 {
            let y = y0 + ch * i as f32 / 7.0;
            g.draw_line(
                Vec2::new(x0 + 10.0, y),
                Vec2::new(x0 + cw - 10.0, y),
                1.0,
                Color::new(1.0, 1.0, 1.0, 0.04),
            );
        }
        // Inner inset, outer frame, corner posts.
        g.draw_rectangle_lines(
            Vec2::new(x0 + 12.0, y0 + 12.0),
            cw - 24.0,
            ch - 24.0,
            2.0,
            Color::new(0.22, 0.24, 0.31, 1.0),
        );
        g.draw_rectangle_lines(
            Vec2::new(x0, y0),
            cw,
            ch,
            6.0,
            Color::new(0.30, 0.33, 0.42, 1.0),
        );
        for (px, py) in [
            (x0, y0),
            (x0 + cw - 14.0, y0),
            (x0, y0 + ch - 14.0),
            (x0 + cw - 14.0, y0 + ch - 14.0),
        ] {
            g.draw_rectangle(
                Vec2::new(px, py),
                14.0,
                14.0,
                Color::new(0.40, 0.44, 0.55, 1.0),
            );
        }
        // The doors (top edge), shut for the ride, with their centre seam.
        g.draw_rectangle(
            Vec2::new(x0 + 20.0, y0 - 4.0),
            cw - 40.0,
            10.0,
            Color::new(0.34, 0.37, 0.47, 1.0),
        );
        g.draw_line(
            Vec2::new(cx, y0 - 4.0),
            Vec2::new(cx, y0 + 6.0),
            2.0,
            Color::new(0.08, 0.08, 0.11, 1.0),
        );
        // Call panel + the ride light breathing green (and a dim hold light).
        g.draw_rectangle(
            Vec2::new(x0 + cw - 10.0, cy - 16.0),
            8.0,
            32.0,
            Color::new(0.16, 0.17, 0.22, 1.0),
        );
        let blink = 0.5 + 0.5 * (t * 2.2).sin();
        g.draw_circle(
            Vec2::new(x0 + cw - 6.0, cy - 6.0),
            2.5,
            Color::new(0.2 + 0.6 * blink, 1.0, 0.4, 1.0),
        );
        g.draw_circle(
            Vec2::new(x0 + cw - 6.0, cy + 6.0),
            2.5,
            Color::new(0.9, 0.35, 0.25, 0.5),
        );
        // CL4-UD3, unarmed, idling — going home.
        g.draw_robot(
            0, // coral
            0, // idle
            0, // fist
            Vec2::new(cx, cy + 6.0),
            -std::f32::consts::FRAC_PI_2,
            200.0,
            t,
        );
    }

    /// The credits roll + the hint, drawn OVER the elevator-ride scene
    /// ([`render_ride`], the caller draws it first): a soft full-screen dim
    /// plus a darker column behind the text keep the roll readable while the
    /// car and the streaking shaft lights stay visible at the edges.
    pub fn draw_credits(g: &Graphics, ending: &Ending) {
        let (w, h) = (g.width(), g.height());
        g.draw_rectangle(
            Vec2::new(0.0, 0.0),
            w,
            h,
            Color::new(0.02, 0.01, 0.05, 0.28),
        );
        // Feathered edges: three nested washes instead of one hard seam.
        let col_w = (w * 0.66).min(760.0);
        for (grow, a) in [(160.0, 0.10), (80.0, 0.12), (0.0, 0.25)] {
            let cw = col_w + grow;
            g.draw_rectangle(
                Vec2::new(w / 2.0 - cw / 2.0, 0.0),
                cw,
                h,
                Color::new(0.02, 0.01, 0.05, a),
            );
        }

        let scroll = ending.scroll(h);
        let mut y = h + 40.0 - scroll;
        for line in CREDITS {
            let lh = line.height();
            if y > -lh && y < h + lh {
                if let Line::Gap = line {
                } else {
                    let size = line.size();
                    let base = y + size * 0.8;
                    // Shadow, then the line.
                    text_centered(
                        g,
                        line.text(),
                        w / 2.0 + 2.0,
                        base + 2.0,
                        size,
                        Color::new(0.0, 0.0, 0.0, 0.7),
                    );
                    text_centered(g, line.text(), w / 2.0, base, size, line.color());
                }
            }
            y += lh;
        }

        // Fixed hint on a dark strip (the roll scrolls beneath it).
        g.draw_rectangle(
            Vec2::new(0.0, h - 44.0),
            w,
            44.0,
            Color::new(0.02, 0.01, 0.05, 1.0),
        );
        text_centered(
            g,
            "Enter / Esc - back to the surface",
            w / 2.0,
            h - 22.0,
            16.0,
            Color::new(0.8, 0.78, 0.9, 0.7),
        );

        // Fade in from the blur colour.
        let fade = 1.0 - (ending.time / CREDITS_FADE_IN_SECS).clamp(0.0, 1.0);
        if fade > 0.0 {
            g.draw_rectangle(
                Vec2::new(0.0, 0.0),
                w,
                h,
                Color::new(BLUR_COLOR.r, BLUR_COLOR.g, BLUR_COLOR.b, fade),
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use draw::{draw_credits, draw_extract_card, render_ride};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_are_well_formed() {
        assert!(CREDITS.len() > 10);
        assert!(matches!(CREDITS[0], Line::Title(_)));
        assert_eq!(
            CREDITS.last().unwrap().text(),
            "MY MASK NEVER COMES OFF.",
            "the roll ends on the LORE line"
        );
        for line in CREDITS {
            // Everything must fit a ~760px column at its size (0.42 em/char).
            let px = line.text().chars().count() as f32 * line.size() * 0.42;
            assert!(px <= 740.0, "credit line too wide: {:?}", line);
            assert!(!line.text().contains('\u{1f}'));
        }
        assert!(credits_height() > 600.0);
        let texts: Vec<&str> = CREDITS.iter().map(Line::text).collect();
        for needle in [
            "THANK YOU",
            "c4ffein",
            "Hotline Miami",
            "DOOM",
            "Snake's Authentic Gun Sounds",
            "Bullet Impact Body Concrete Metal Flyby",
            "VT323",
            "Playwright",
        ] {
            assert!(
                texts.iter().any(|t| t.contains(needle)),
                "credits mention {needle}"
            );
        }
    }

    #[test]
    fn outro_waits_for_the_feed_then_blurs_then_finishes() {
        let mut o = Outro::new();
        assert_eq!(o.card_alpha(), 1.0);
        assert_eq!(o.blur_t(), None);
        // Feed busy: never leaves the uplink phase (until the hard cap).
        for _ in 0..600 {
            assert!(!o.tick(0.05, false)); // 30 s
        }
        assert!(matches!(o, Outro::Uplink { .. }));
        assert_eq!(o.card_alpha(), 0.0, "the card has faded");
        // Feed idle: holds UPLINK_HOLD_SECS, then blurs.
        for _ in 0..80 {
            o.tick(0.05, true); // 4.0 s
        }
        assert!(matches!(o, Outro::Uplink { .. }));
        for _ in 0..12 {
            o.tick(0.05, true); // 4.6 s idle
        }
        assert!(matches!(o, Outro::Blur { .. }));
        let mut done = false;
        let mut last_t = 0.0;
        for _ in 0..60 {
            done = o.tick(0.05, true);
            let t = o.blur_t().unwrap();
            assert!(t >= last_t);
            last_t = t;
            if done {
                break;
            }
        }
        assert!(done);
        assert_eq!(o.blur_t(), Some(1.0));
    }

    #[test]
    fn outro_hard_cap_and_minimum() {
        // Idle from the start: still waits UPLINK_MIN_SECS + hold.
        let mut o = Outro::new();
        for _ in 0..40 {
            o.tick(0.05, true); // 2 s
        }
        assert!(matches!(o, Outro::Uplink { .. }));
        // A feed that never goes idle is cut at UPLINK_MAX_SECS.
        let mut o = Outro::new();
        for _ in 0..1300 {
            o.tick(0.05, false); // 65 s
        }
        assert!(matches!(o, Outro::Blur { .. }));
    }

    #[test]
    fn warp_ramps_in_holds_and_eases_down_to_idle() {
        let h = 720.0;
        // Ramp: 0 at the cut, full after WARP_RAMP_SECS, monotonic.
        let mut e = Ending::new();
        assert_eq!(e.warp_t(h), 0.0);
        let mut last = 0.0;
        for _ in 0..40 {
            e.tick(WARP_RAMP_SECS / 20.0);
            let t = e.warp_t(h);
            assert!(t >= last - 1e-6);
            last = t;
        }
        assert!((e.warp_t(h) - 1.0).abs() < 1e-6, "holds at full strength");
        // Ease-down: at the settle point the ride idles at WARP_IDLE_T.
        let settle_secs = Ending::max_scroll(h) / CREDITS_PX_PER_SEC;
        let e = Ending { time: settle_secs };
        assert!((e.warp_t(h) - WARP_IDLE_T).abs() < 1e-6);
        // ...and it stays there.
        let e = Ending {
            time: settle_secs + 100.0,
        };
        assert!((e.warp_t(h) - WARP_IDLE_T).abs() < 1e-6);
        // Halfway through the ease window: between idle and full.
        let e = Ending {
            time: settle_secs - WARP_EASE_SECS / 2.0,
        };
        let t = e.warp_t(h);
        assert!(t > WARP_IDLE_T && t < 1.0);
    }

    #[test]
    fn ride_phase_accelerates_then_runs_at_unit_speed() {
        assert_eq!(ride_phase(0.0), 0.0);
        // Strictly increasing.
        let mut last = 0.0;
        for i in 1..100 {
            let p = ride_phase(i as f32 * 0.2);
            assert!(p > last);
            last = p;
        }
        // Early speed ~0.35, late speed ~1.0 per second.
        let early = ride_phase(0.5) - ride_phase(0.0);
        assert!((early - 0.5 * 0.37) < 0.05);
        let late = ride_phase(21.0) - ride_phase(20.0);
        assert!((late - 1.0).abs() < 1e-4);
        // Continuous across the ramp end.
        let a = ride_phase(WARP_RAMP_SECS - 1e-3);
        let b = ride_phase(WARP_RAMP_SECS + 1e-3);
        assert!(b - a < 0.01);
    }

    #[test]
    fn credits_scroll_settles_on_the_quote() {
        let h = 720.0;
        let mut e = Ending::new();
        assert_eq!(e.scroll(h), 0.0);
        e.tick(10.0);
        assert_eq!(e.scroll(h), 10.0 * CREDITS_PX_PER_SEC);
        e.tick(10_000.0);
        let last_h = CREDITS.last().unwrap().height();
        let expect = h + 40.0 + credits_height() - last_h - h * 0.5;
        assert_eq!(e.scroll(h), expect);
        assert!(e.settled(h));
        // The quote's top then sits at the vertical centre.
        let quote_top = h + 40.0 + credits_height() - last_h - e.scroll(h);
        assert!((quote_top - h * 0.5).abs() < 0.01);
    }
}
