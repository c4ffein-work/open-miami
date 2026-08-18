//! The visual-novel DIALOGUE panel (`talk` scenario actions): a slanted
//! cyberpunk panel sliding in from the RIGHT with the speaking bot's bust
//! (live robot / shoggoth render, or a primitive glyph), the speaker's name
//! in their fixed colour, the current line (typewriter + wrap) and a blinking
//! advance prompt. Screen space — drawn with the HUD, after `camera.reset`,
//! outside the world pixel group.
//!
//! The panel's left edge is a DIAGONAL cut (wider at the top): the fill is
//! built from thin horizontal slices (exact, no clipping needed) and the edge
//! is overdrawn with thick accent lines.

use crate::graphics::Graphics;
use crate::math::{Color, Vec2};
use crate::render_comms::{draw_portrait, wrap_text};
use crate::scenario::{speaker_rgb, DialogueView};

/// Approximate VT323 advance as a fraction of the font size (same heuristic
/// as `render_comms`).
const CHAR_W: f32 = 0.42;

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

/// Draw the dialogue panel for this frame's [`DialogueView`]. `now` is
/// elapsed seconds (drives the portrait animation and the blink).
pub fn render_dialogue(graphics: &Graphics, view: &DialogueView, accent: (u8, u8, u8), now: f32) {
    if view.slide <= 0.0 {
        return;
    }
    let w = graphics.width();
    let h = graphics.height();
    let panel_w = (w * 0.32).clamp(270.0, 400.0);
    let slant = 80.0; // the diagonal cut: this much wider at the top
    let who_col = speaker_rgb(view.who);

    // Slide in/out: the whole panel translates from beyond the right edge.
    let off = (1.0 - view.slide) * (panel_w + slant + 40.0);
    graphics.save();
    graphics.translate(off, 0.0);

    let edge_top = w - panel_w - slant; // x of the cut at y = 0
    let edge_bot = w - panel_w; // x of the cut at y = h

    // Diagonal-edged fill from thin horizontal slices.
    let bg = Color::new(0.05, 0.03, 0.09, 0.96);
    let step = 8.0;
    let mut y = 0.0;
    while y < h {
        let t = (y + step * 0.5) / h;
        let ex = edge_top + (edge_bot - edge_top) * t;
        graphics.draw_rectangle(Vec2::new(ex, y), w - ex + off + 4.0, step.min(h - y), bg);
        y += step;
    }
    // The cut edge: a bright accent line plus a parallel speaker-colour line.
    graphics.draw_line(
        Vec2::new(edge_top, -6.0),
        Vec2::new(edge_bot, h + 6.0),
        3.0,
        rgb(accent, 0.9),
    );
    graphics.draw_line(
        Vec2::new(edge_top + 10.0, -6.0),
        Vec2::new(edge_bot + 10.0, h + 6.0),
        1.5,
        rgb(who_col, 0.5),
    );

    // Content column, right of the cut at its widest (the bottom).
    let cx0 = edge_bot + 26.0;
    let cw = w - 18.0 - cx0;

    // Header.
    graphics.draw_text(
        "DIRECT CHANNEL // ON-SITE",
        Vec2::new(cx0, 30.0),
        12.0,
        rgb(accent, 0.7),
    );
    graphics.draw_line(
        Vec2::new(cx0, 37.0),
        Vec2::new(cx0 + cw, 37.0),
        1.0,
        rgb(accent, 0.3),
    );

    // Portrait box.
    let box_y = 52.0;
    let box_h = (h * 0.28).clamp(150.0, 210.0);
    graphics.draw_rectangle(
        Vec2::new(cx0, box_y),
        cw,
        box_h,
        Color::new(0.02, 0.015, 0.045, 0.95),
    );
    graphics.draw_rectangle_lines(Vec2::new(cx0, box_y), cw, box_h, 1.5, rgb(who_col, 0.55));
    let center = Vec2::new(cx0 + cw / 2.0, box_y + box_h / 2.0);
    draw_bust(graphics, view.who, center, box_h, now);

    // Name plate.
    let name = if view.who == "CL4-UD3" {
        "CL4-UD3 // you"
    } else {
        view.who
    };
    let name_fs = 22.0;
    let name_y = box_y + box_h + 28.0;
    graphics.draw_text(name, Vec2::new(cx0, name_y), name_fs, rgb(who_col, 1.0));
    graphics.draw_line(
        Vec2::new(cx0, name_y + 6.0),
        Vec2::new(cx0 + cw, name_y + 6.0),
        1.0,
        rgb(who_col, 0.4),
    );

    // The line, typewriter-revealed and wrapped.
    let text_fs = 18.0;
    let row_h = 21.0;
    let max_chars = ((cw / (text_fs * CHAR_W)) as usize).max(10);
    let shown: String = view.text.chars().take(view.chars_shown).collect();
    let cursor_on = ((now * 6.0) as u32).is_multiple_of(2);
    let text_col = if view.who == "CL4-UD3" {
        Color::new(1.0, 0.93, 0.9, 1.0)
    } else {
        Color::new(0.88, 0.85, 0.97, 1.0)
    };
    let mut ty = name_y + 30.0;
    let wrapped = wrap_text(&shown, max_chars);
    for (i, seg) in wrapped.iter().enumerate() {
        let last = i + 1 == wrapped.len();
        let line = if last && !view.fully_typed && cursor_on {
            format!("{seg}_")
        } else {
            seg.clone()
        };
        graphics.draw_text(&line, Vec2::new(cx0, ty), text_fs, text_col);
        ty += row_h;
    }

    // Advance prompt: a blinking pixel triangle once the line is fully shown.
    if view.fully_typed {
        let blink = 0.45 + 0.45 * (now * 4.0).sin();
        let tx = cx0 + cw / 2.0;
        let tyy = h - 52.0;
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
        let hw = hint.chars().count() as f32 * 11.0 * CHAR_W;
        graphics.draw_text(
            hint,
            Vec2::new(tx - hw / 2.0, h - 24.0),
            11.0,
            rgb(accent, 0.35 + 0.25 * blink),
        );
    }

    graphics.restore();
}

/// The speaker's bust inside the portrait box. The live in-game robot render
/// (opcode ROBOT) is a strict top-down camera, so at portrait size a robot is
/// just its head plate seen from above — not a readable bust. Robot speakers
/// therefore use the scaled-up stylized primitive heads (the per-faction
/// visor designs of [`render_comms::draw_portrait`], the same language as the
/// comms feed), with a small LIVE robot standing beside the head as a colour
/// swatch. SWARM = a cluster of three small heads; CORRUPTOR = the LIVE
/// shoggoth (its top-down smiley mask reads perfectly); UPLINK = an abstract
/// carrier glyph from primitives.
fn draw_bust(graphics: &Graphics, who: &str, center: Vec2, box_h: f32, now: f32) {
    let col = speaker_rgb(who);
    if let Some(idx) = robot_color_idx(who) {
        let s = box_h * 0.86;
        draw_portrait(
            graphics,
            Vec2::new(center.x - s / 2.0, center.y - s / 2.0),
            s,
            who,
            1.0,
        );
        // The live robot, small, idling in the corner of the frame (top-down
        // — reads as the speaker's actual in-world sprite).
        graphics.draw_robot(
            idx,
            0,
            0,
            Vec2::new(center.x + box_h * 0.56, center.y + box_h * 0.30),
            0.0,
            box_h * 0.55,
            now,
        );
        return;
    }
    match who {
        "SWARM" => {
            // A chorus: three small heads, slightly overlapping.
            let s = box_h * 0.52;
            let offs = [(-0.30_f32, -0.14_f32), (0.30, -0.14), (0.0, 0.12)];
            for (dx, dy) in offs.iter() {
                draw_portrait(
                    graphics,
                    Vec2::new(
                        center.x + dx * box_h - s / 2.0,
                        center.y + dy * box_h - s / 2.0,
                    ),
                    s,
                    who,
                    1.0,
                );
            }
        }
        "CORRUPTOR" => {
            // The shoggoth, small, its mask barely holding.
            graphics.draw_shoggoth_live(
                Vec2::new(center.x, center.y + 2.0),
                box_h * 0.95,
                std::f32::consts::FRAC_PI_2,
                0.18,
                now,
            );
        }
        _ => {
            // UPLINK (and any unknown voice): an abstract carrier — a core,
            // radiating arcs and a breathing waveform.
            let r = box_h * 0.10;
            graphics.draw_circle(center, r, rgb(col, 0.95));
            graphics.draw_circle(center, r * 0.55, Color::new(0.02, 0.015, 0.045, 1.0));
            for k in 1..=3 {
                let rr = r + k as f32 * box_h * 0.09;
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
            let ww = box_h * 0.9;
            let wy = center.y + box_h * 0.32;
            for i in 0..n {
                let t0 = i as f32 / n as f32;
                let t1 = (i + 1) as f32 / n as f32;
                let x0 = center.x - ww / 2.0 + t0 * ww;
                let x1 = center.x - ww / 2.0 + t1 * ww;
                let y0 = wy + (t0 * 14.0 + now * 3.0).sin() * box_h * 0.05;
                let y1 = wy + (t1 * 14.0 + now * 3.0).sin() * box_h * 0.05;
                graphics.draw_line(Vec2::new(x0, y0), Vec2::new(x1, y1), 1.5, rgb(col, 0.8));
            }
        }
    }
}
