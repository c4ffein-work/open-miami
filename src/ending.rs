//! The ending — everything that happens once the last car goes up.
//!
//! Timeline (all driven from `update_game` in lib.rs):
//!   1. the player extracts through a `to: 0` exit → the EXFILTRATE card
//!      ([`draw_extract_card`], also used on every ordinary floor)
//!   2. the [`Outro`] takes over: the card fades, the floor's `extracted`
//!      scenario step talks (13½: the UPLINK epilogue) until the comms feed
//!      goes idle, then a BLUR-OUT (POSTFX kind 0) dissolves the frame
//!   3. `GameScreen::Ending`: the [`CREDITS`] roll over a synthwave backdrop
//!      ([`draw_credits`]), Enter / Esc back to the level select.
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
    /// The synthwave horizon, as a fraction of the screen height.
    const HORIZON_FRAC: f32 = 0.62;

    fn hash01(a: u32, b: u32) -> f32 {
        let mut x = a
            .wrapping_mul(374_761_393)
            .wrapping_add(b.wrapping_mul(668_265_263));
        x = (x ^ (x >> 13)).wrapping_mul(1_274_126_177);
        ((x ^ (x >> 16)) & 0xff_ffff) as f32 / 0xff_ffff as f32
    }

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

    /// The synthwave backdrop: gradient sky, twinkling stars, a striped sun
    /// on the horizon, and a perspective grid rolling toward the viewer.
    /// `now` = seconds (drives the grid + twinkle).
    pub fn draw_synthwave_bg(g: &Graphics, now: f32) {
        let (w, h) = (g.width(), g.height());
        let horizon = h * HORIZON_FRAC;
        let sky_top = BLUR_COLOR;
        let sky_mid = Color::new(0.24, 0.04, 0.30, 1.0);
        let sky_low = Color::new(0.55, 0.10, 0.36, 1.0);
        // Sky: banded gradient.
        let bands = 28;
        for i in 0..bands {
            let f0 = i as f32 / bands as f32;
            let f1 = (i + 1) as f32 / bands as f32;
            let c = if f0 < 0.6 {
                mix(sky_top, sky_mid, f0 / 0.6)
            } else {
                mix(sky_mid, sky_low, (f0 - 0.6) / 0.4)
            };
            g.draw_rectangle(
                Vec2::new(0.0, f0 * horizon),
                w,
                (f1 - f0) * horizon + 1.0,
                c,
            );
        }
        // Stars.
        for i in 0..90u32 {
            let sx = hash01(i, 1) * w;
            let sy = hash01(i, 2) * horizon * 0.85;
            let tw =
                0.35 + 0.65 * (0.5 + 0.5 * (now * (1.0 + hash01(i, 3) * 2.0) + i as f32).sin());
            let r = 0.8 + hash01(i, 4) * 1.2;
            g.draw_circle(
                Vec2::new(sx, sy),
                r,
                Color::new(1.0, 0.95, 1.0, 0.75 * tw * (1.0 - sy / horizon)),
            );
        }
        // Sun: warm disc with horizontal cuts, sitting on the horizon.
        let sr = h * 0.17;
        let sc = Vec2::new(w / 2.0, horizon - sr * 0.55);
        let sun_top = Color::new(1.0, 0.92, 0.38, 1.0);
        let sun_bot = Color::new(1.0, 0.30, 0.62, 1.0);
        let slices = 24;
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
            // Lower half: growing dark cuts between the slices.
            let cut = if f0 > 0.5 {
                (f0 - 0.5) * 2.0 * 0.5
            } else {
                0.0
            };
            let hh = (y1.min(horizon) - y0) * (1.0 - cut);
            g.draw_rectangle(Vec2::new(sc.x - half, y0), half * 2.0, hh.max(1.0), c);
        }
        // Horizon glow.
        g.draw_rectangle(
            Vec2::new(0.0, horizon - 2.0),
            w,
            3.0,
            Color::new(1.0, 0.35, 0.75, 0.9),
        );
        // Ground.
        g.draw_rectangle(
            Vec2::new(0.0, horizon),
            w,
            h - horizon,
            Color::new(0.04, 0.01, 0.08, 1.0),
        );
        let grid = Color::new(1.0, 0.25, 0.75, 0.55);
        let grid_dim = Color::new(0.35, 0.85, 1.0, 0.25);
        // Horizontal lines: perspective spacing, rolling toward the viewer.
        let rows = 14;
        for i in 0..rows {
            let f = ((i as f32 + (now * 0.6).fract() * 1.0) / rows as f32).min(1.0);
            let y = horizon + (h - horizon) * f * f;
            let a = 0.15 + 0.85 * f;
            g.draw_line(
                Vec2::new(0.0, y),
                Vec2::new(w, y),
                1.0 + f,
                Color::new(grid.r, grid.g, grid.b, grid.a * a),
            );
        }
        // Vertical lines converging on the vanishing point.
        let cols = 18;
        for i in 0..=cols {
            let f = i as f32 / cols as f32 - 0.5;
            let x_bottom = w / 2.0 + f * w * 2.4;
            g.draw_line(
                Vec2::new(w / 2.0, horizon),
                Vec2::new(x_bottom, h),
                1.0,
                if i % 3 == 0 { grid } else { grid_dim },
            );
        }
    }

    /// The credits screen: backdrop + the scrolling roll + the hint. Meant to
    /// be followed by `postfx(1, ..)` for the CRT look. `now` = seconds.
    pub fn draw_credits(g: &Graphics, ending: &Ending, now: f32) {
        let (w, h) = (g.width(), g.height());
        draw_synthwave_bg(g, now);

        // A soft dark column over the ground so the text reads over the grid
        // (the sky is dark enough on its own, and the sun stays vivid).
        let col_w = (w * 0.66).min(760.0);
        let horizon = h * HORIZON_FRAC;
        g.draw_rectangle(
            Vec2::new(w / 2.0 - col_w / 2.0, horizon),
            col_w,
            h - horizon,
            Color::new(0.03, 0.01, 0.06, 0.45),
        );

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
pub use draw::{draw_credits, draw_extract_card, draw_synthwave_bg};

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
