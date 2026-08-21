//! The DIALOGUE frame (`talk` scenario actions): a cinematic JRPG letterbox
//! CROSSED with the old visual-novel side panel. A small black bar pops in at
//! the TOP of the screen and a bigger one at the BOTTOM (both full width,
//! sliding in over the panel's slide time); the bottom bar carries the
//! speaker's name in their fixed colour (letter-spaced, drop-shadowed), the
//! typewriter line in large type from the LEFT edge, and the blinking advance
//! triangle. The speaker's FACE lives on a tall SLAB hugging the RIGHT edge
//! between the two bars: a dark translucent panel whose left border is a
//! DIAGONAL cut (top edge further right, bottom edge further left, meeting
//! the bottom bar), the cut overdrawn with accent edge lines; the fill is
//! built from thin horizontal slices (exact, no clipping needed). On the slab
//! sits the baked pixel-art headshot, gently rocking in 2D, big and
//! borderless (SWARM = three out-of-phase headshots down the diagonal,
//! CORRUPTOR = the live shoggoth, UPLINK = its carrier glyph). The world keeps rendering between
//! the bars and left of the slab. Screen space — drawn with the HUD, after
//! `camera.reset`, outside the world pixel group.

use crate::graphics::Graphics;
use crate::math::{Color, Vec2};
use crate::render_comms::wrap_text;
use crate::scenario::{speaker_rgb, DialogueView};

/// Approximate VT323 advance as a fraction of the font size (same heuristic
/// as `render_comms`).
const CHAR_W: f32 = 0.42;

/// Height of the top letterbox bar, px.
const TOP_H: f32 = 52.0;
/// Height of the bottom letterbox bar (the dialogue bar), px.
const BOT_H: f32 = 170.0;
/// Left padding of the text column in the bottom bar, px.
const TEXT_X0: f32 = 40.0;
/// Extra letter-spacing (tracking) of the speaker name, px.
const NAME_TRACK: f32 = 4.0;
/// Speaker-name font size, px.
const NAME_FS: f32 = 28.0;
/// Dialogue-line font size, px.
const TEXT_FS: f32 = 24.0;
/// Dialogue-line leading, px.
const ROW_H: f32 = 27.0;
/// The slab's diagonal cut: how much further LEFT its bottom edge sits, px.
const SLANT: f32 = 90.0;

fn rgb(c: (u8, u8, u8), a: f32) -> Color {
    Color::new(
        c.0 as f32 / 255.0,
        c.1 as f32 / 255.0,
        c.2 as f32 / 255.0,
        a,
    )
}

/// The live-robot colour index for a speaker (renderer.js table:
/// 0 coral, 1 red, 2 violet, 3 magenta), `None` for non-robot speakers.
fn robot_color_idx(who: &str) -> Option<u32> {
    match who {
        "CL4-UD3" => Some(0),
        "SENTINEL" => Some(1),
        "DRIFTER" => Some(2),
        "HUNTER" => Some(3),
        _ => None,
    }
}

/// Draw the dialogue letterbox + face slab for this frame's
/// [`DialogueView`]. `now` is elapsed seconds (drives the portrait's 2D rock
/// and the blink).
pub fn render_dialogue(graphics: &Graphics, view: &DialogueView, accent: (u8, u8, u8), now: f32) {
    if view.slide <= 0.0 {
        return;
    }
    let w = graphics.width();
    let h = graphics.height();
    let who_col = speaker_rgb(view.who);
    let bar = Color::new(0.0, 0.0, 0.0, 1.0);

    // ---- the face slab: slides in from beyond the right edge ----
    draw_slab(graphics, view, accent, now);

    // ---- top bar: slides down from above the screen ----
    graphics.save();
    graphics.translate(0.0, -(1.0 - view.slide) * (TOP_H + 4.0));
    graphics.draw_rectangle(Vec2::new(-4.0, -4.0), w + 8.0, TOP_H + 4.0, bar);
    graphics.draw_line(
        Vec2::new(0.0, TOP_H - 1.0),
        Vec2::new(w, TOP_H - 1.0),
        1.5,
        rgb(accent, 0.35),
    );
    graphics.draw_text(
        "DIRECT CHANNEL // ON-SITE",
        Vec2::new(24.0, 33.0),
        15.0,
        rgb(accent, 0.6),
    );
    graphics.restore();

    // ---- bottom bar: slides up from below the screen ----
    graphics.save();
    graphics.translate(0.0, (1.0 - view.slide) * (BOT_H + 4.0));
    let bar_top = h - BOT_H;
    graphics.draw_rectangle(Vec2::new(-4.0, bar_top), w + 8.0, BOT_H + 8.0, bar);
    graphics.draw_line(
        Vec2::new(0.0, bar_top + 1.0),
        Vec2::new(w, bar_top + 1.0),
        1.5,
        rgb(accent, 0.35),
    );

    // The text column now owns the whole bar: the face lives on the slab.
    let cx0 = TEXT_X0;
    let cw = w - 130.0 - cx0;

    // Name plate: letter-spaced, drop-shadowed, in the speaker's colour.
    let name = if view.who == "CL4-UD3" {
        "CL4-UD3 // you"
    } else {
        view.who
    };
    let name_y = bar_top + 42.0;
    let adv = NAME_FS * CHAR_W + NAME_TRACK;
    let mut nx = cx0;
    let mut buf = [0u8; 4];
    for ch in name.chars() {
        let s: &str = ch.encode_utf8(&mut buf);
        graphics.draw_text(
            s,
            Vec2::new(nx + 2.0, name_y + 2.0),
            NAME_FS,
            Color::new(0.0, 0.0, 0.0, 0.9),
        );
        graphics.draw_text(s, Vec2::new(nx, name_y), NAME_FS, rgb(who_col, 1.0));
        nx += adv;
    }
    graphics.draw_line(
        Vec2::new(cx0, name_y + 9.0),
        Vec2::new(cx0 + cw, name_y + 9.0),
        1.0,
        rgb(who_col, 0.35),
    );

    // The line, typewriter-revealed and wrapped, large with a soft shadow.
    let max_chars = ((cw / (TEXT_FS * CHAR_W)) as usize).max(10);
    let shown: String = view.text.chars().take(view.chars_shown).collect();
    let cursor_on = ((now * 6.0) as u32).is_multiple_of(2);
    let text_col = if view.who == "CL4-UD3" {
        Color::new(1.0, 0.93, 0.9, 1.0)
    } else {
        Color::new(0.9, 0.88, 0.98, 1.0)
    };
    let mut ty = name_y + 38.0;
    let wrapped = wrap_text(&shown, max_chars);
    for (i, seg) in wrapped.iter().enumerate() {
        let last = i + 1 == wrapped.len();
        let line = if last && !view.fully_typed && cursor_on {
            format!("{seg}_")
        } else {
            seg.clone()
        };
        graphics.draw_text(
            &line,
            Vec2::new(cx0 + 2.0, ty + 2.0),
            TEXT_FS,
            Color::new(0.0, 0.0, 0.0, 0.85),
        );
        graphics.draw_text(&line, Vec2::new(cx0, ty), TEXT_FS, text_col);
        ty += ROW_H;
    }

    // Advance prompt, bottom-right of the bar: a blinking pixel triangle
    // once the line is fully shown, the CLICK/SPACE hint under it.
    if view.fully_typed {
        let blink = 0.45 + 0.45 * (now * 4.0).sin();
        let tx = w - 62.0;
        let tyy = bar_top + BOT_H - 56.0;
        for i in 0..4 {
            let half = 10.0 - i as f32 * 2.5;
            graphics.draw_rectangle(
                Vec2::new(tx - half, tyy + i as f32 * 3.0),
                half * 2.0,
                3.0,
                rgb(accent, blink),
            );
        }
        let hint = if view.more { "CLICK / SPACE" } else { "END" };
        let hw = hint.chars().count() as f32 * 12.0 * CHAR_W;
        graphics.draw_text(
            hint,
            Vec2::new(tx - hw / 2.0, bar_top + BOT_H - 16.0),
            12.0,
            rgb(accent, 0.35 + 0.25 * blink),
        );
    }

    graphics.restore();
}

/// The right-side FACE SLAB between the two letterbox bars: a dark
/// translucent panel with a diagonal-cut left border (top edge further
/// right, bottom edge further left, meeting the bottom bar), accent edge
/// lines along the cut, and the speaker's portrait floating borderless on
/// it. Robot speakers are the robot in HEADSHOT framing (opcode PORTRAIT
/// mode 1): a fixed close head-level camera, BAKED ONCE by the renderer at a
/// small art resolution, upscaled NEAREST and gently ROCKED in 2D by `now` —
/// a chunky Hotline-Miami-style portrait, BIG. SWARM = three smaller
/// out-of-phase headshots stepping down the diagonal (their `now` offsets
/// phase-shift the rock); CORRUPTOR = the LIVE
/// shoggoth (its top-down smiley mask reads perfectly); UPLINK = an
/// abstract carrier glyph from primitives.
fn draw_slab(graphics: &Graphics, view: &DialogueView, accent: (u8, u8, u8), now: f32) {
    let w = graphics.width();
    let h = graphics.height();
    let who_col = speaker_rgb(view.who);

    let slab_top = TOP_H;
    let slab_bot = h - BOT_H;
    let slab_h = slab_bot - slab_top;
    if slab_h < 60.0 {
        return;
    }
    // Width at the TOP (the narrow end); the bottom is SLANT px wider.
    let slab_w = (w * 0.24).clamp(220.0, 340.0);
    let edge_top = w - slab_w; // x of the cut at the slab's top
    let edge_bot = w - slab_w - SLANT; // x of the cut at the slab's bottom

    // Slide in/out with the bars: translate from beyond the right edge.
    let off = (1.0 - view.slide) * (slab_w + SLANT + 40.0);
    graphics.save();
    graphics.translate(off, 0.0);

    // Diagonal-edged fill from thin horizontal slices.
    let bg = Color::new(0.05, 0.03, 0.09, 0.88);
    let step = 8.0;
    let mut y = slab_top;
    while y < slab_bot {
        let t = (y - slab_top + step * 0.5) / slab_h;
        let ex = edge_top + (edge_bot - edge_top) * t;
        graphics.draw_rectangle(Vec2::new(ex, y), w - ex + 8.0, step.min(slab_bot - y), bg);
        y += step;
    }
    // The cut edge: a bright accent line plus a parallel speaker-colour one.
    graphics.draw_line(
        Vec2::new(edge_top, slab_top),
        Vec2::new(edge_bot, slab_bot),
        3.0,
        rgb(accent, 0.9),
    );
    graphics.draw_line(
        Vec2::new(edge_top + 10.0, slab_top),
        Vec2::new(edge_bot + 10.0, slab_bot),
        1.5,
        rgb(who_col, 0.5),
    );

    // The portrait, big and borderless, low on the slab (widest part).
    let face_s = (slab_h - 24.0).clamp(120.0, 300.0);
    let fx = (edge_bot + 24.0 + face_s / 2.0).min(w - 12.0 - face_s / 2.0);
    let fy = slab_bot - 10.0 - face_s / 2.0;
    if let Some(idx) = robot_color_idx(view.who) {
        graphics.draw_robot_portrait(idx, Vec2::new(fx, fy), face_s, now, 1);
        graphics.restore();
        return;
    }
    match view.who {
        "SWARM" => {
            // A chorus: three smaller headshots stepping down the diagonal,
            // the three rogue palettes, swaying out of phase.
            let s = (slab_h * 0.36).clamp(90.0, 170.0);
            for (k, color) in [1_u32, 3, 2].iter().enumerate() {
                let t = (k as f32 + 0.5) / 3.0;
                let cy = slab_top + t * slab_h;
                let ex = edge_top + (edge_bot - edge_top) * ((cy - slab_top) / slab_h);
                graphics.draw_robot_portrait(
                    *color,
                    Vec2::new(ex + 22.0 + s / 2.0, cy),
                    s,
                    now + k as f32 * 2.3,
                    1,
                );
            }
        }
        "CORRUPTOR" => {
            // The live shoggoth, its mask barely holding — only on 13½.
            graphics.draw_shoggoth_live(
                Vec2::new(fx, fy),
                face_s * 0.95,
                std::f32::consts::FRAC_PI_2,
                0.18,
                now,
            );
        }
        _ => {
            // UPLINK (and any unknown voice): an abstract carrier — a core,
            // radiating arcs and a breathing waveform. Borderless.
            let col = who_col;
            let bh = face_s * 0.85;
            let center = Vec2::new(fx, fy - bh * 0.06);
            let r = bh * 0.10;
            graphics.draw_circle(center, r, rgb(col, 0.95));
            graphics.draw_circle(center, r * 0.55, Color::new(0.0, 0.0, 0.0, 1.0));
            for k in 1..=3 {
                let rr = r + k as f32 * bh * 0.09;
                let a = 0.55 - 0.13 * k as f32;
                let wob = (now * 1.4 + k as f32).sin() * 0.25;
                graphics.draw_arc(
                    center,
                    rr,
                    1.15 * std::f32::consts::PI + wob,
                    1.85 * std::f32::consts::PI + wob,
                    rgb(col, a),
                );
                graphics.draw_arc(
                    center,
                    rr,
                    0.15 * std::f32::consts::PI + wob,
                    0.85 * std::f32::consts::PI + wob,
                    rgb(col, a),
                );
            }
            // Waveform across the lower third.
            let n = 24;
            let ww = bh * 0.9;
            let wy = center.y + bh * 0.32;
            for i in 0..n {
                let t0 = i as f32 / n as f32;
                let t1 = (i + 1) as f32 / n as f32;
                let x0 = center.x - ww / 2.0 + t0 * ww;
                let x1 = center.x - ww / 2.0 + t1 * ww;
                let y0 = wy + (t0 * 14.0 + now * 3.0).sin() * bh * 0.05;
                let y1 = wy + (t1 * 14.0 + now * 3.0).sin() * bh * 0.05;
                graphics.draw_line(Vec2::new(x0, y0), Vec2::new(x1, y1), 1.5, rgb(col, 0.8));
            }
        }
    }
    graphics.restore();
}
