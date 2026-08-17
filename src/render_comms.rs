//! In-game scenario overlays: elevators (world space), and the intercepted
//! comms feed + objective line (screen space, drawn after the camera reset).
//!
//! Everything here is drawn with the primitive `Graphics` API (rects, lines,
//! circles, arcs, text) so it needs no assets.

use crate::components::{Elevator, Zone};
use crate::ecs::World;
use crate::graphics::Graphics;
use crate::math::{Color, Vec2};
use crate::scenario::{speaker_rgb, ScenarioState};
use crate::systems::elevator::EXTRACT_DWELL_SECS;

/// Approximate VT323 advance as a fraction of the font size (used to wrap
/// text; the renderer measures the real glyphs when drawing).
const CHAR_W: f32 = 0.42;

fn rgb(c: (u8, u8, u8), a: f32) -> Color {
    Color::new(
        c.0 as f32 / 255.0,
        c.1 as f32 / 255.0,
        c.2 as f32 / 255.0,
        a,
    )
}

/// Greedy word wrap to `max_chars` per line.
pub fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for word in text.split_whitespace() {
        let wl = word.chars().count();
        if cur_len > 0 && cur_len + 1 + wl > max_chars {
            lines.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if cur_len > 0 {
            cur.push(' ');
            cur_len += 1;
        }
        cur.push_str(word);
        cur_len += wl;
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Which side of an elevator rect is its back wall (the shaft side).
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

fn back_side(world: &World, e: &Elevator) -> Side {
    let probe = |p: Vec2| {
        world
            .walls()
            .iter()
            .any(|w| p.x >= w.x && p.x <= w.x + w.width && p.y >= w.y && p.y <= w.y + w.height)
    };
    let c = e.center();
    if probe(Vec2::new(c.x, e.y - 6.0)) {
        Side::Top
    } else if probe(Vec2::new(c.x, e.y + e.h + 6.0)) {
        Side::Bottom
    } else if probe(Vec2::new(e.x - 6.0, c.y)) {
        Side::Left
    } else if probe(Vec2::new(e.x + e.w + 6.0, c.y)) {
        Side::Right
    } else if e.w >= e.h {
        Side::Top
    } else {
        Side::Left
    }
}

/// Draw every elevator as a recessed door frame with a lit strip on the shaft
/// side: dim when closed, bright accent + pulsing when open. `now` is elapsed
/// seconds (drives the pulse). Must be called in world space.
pub fn render_elevators(world: &World, graphics: &Graphics, accent: (u8, u8, u8), now: f32) {
    let mut cars: Vec<Elevator> = world
        .query::<Elevator>()
        .into_iter()
        .filter_map(|e| world.get_component::<Elevator>(e).copied())
        .collect();
    // Exits draw over the entry; when an exit shares the entry's car (13½:
    // the jammed car you arrived in is also the way out) the entry is skipped.
    cars.sort_by_key(|e| e.is_exit);
    let shared_entry = |e: &Elevator| {
        !e.is_exit
            && cars
                .iter()
                .any(|x| x.is_exit && x.x == e.x && x.y == e.y && x.w == e.w && x.h == e.h)
    };

    for e in cars.iter().filter(|e| !shared_entry(e)) {
        let side = back_side(world, e);
        let (x, y, w, h) = (e.x, e.y, e.w, e.h);
        let lit = e.is_exit && e.open;
        let pulse = 0.5 + 0.5 * (now * 4.0).sin();

        // Recess: the shaft cuts into the wall behind the frame (drawn over
        // the wall band), then a dark shaft floor with a lighter plate inside.
        let depth = 16.0;
        let (rx, ry, rw, rh) = match side {
            Side::Top => (x, y - depth, w, h + depth),
            Side::Bottom => (x, y, w, h + depth),
            Side::Left => (x - depth, y, w + depth, h),
            Side::Right => (x, y, w + depth, h),
        };
        graphics.draw_rectangle(Vec2::new(rx, ry), rw, rh, Color::new(0.03, 0.02, 0.05, 1.0));
        graphics.draw_rectangle_lines(
            Vec2::new(rx, ry),
            rw,
            rh,
            2.0,
            Color::new(100.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0),
        );
        graphics.draw_rectangle(
            Vec2::new(x + 6.0, y + 6.0),
            w - 12.0,
            h - 12.0,
            Color::new(0.10, 0.08, 0.14, 1.0),
        );
        // Grating lines on the plate, along the back axis.
        let grate = Color::new(0.16, 0.13, 0.22, 1.0);
        match side {
            Side::Top | Side::Bottom => {
                let mut gx = x + 12.0;
                while gx < x + w - 8.0 {
                    graphics.draw_rectangle(Vec2::new(gx, y + 8.0), 1.0, h - 16.0, grate);
                    gx += 8.0;
                }
            }
            Side::Left | Side::Right => {
                let mut gy = y + 12.0;
                while gy < y + h - 8.0 {
                    graphics.draw_rectangle(Vec2::new(x + 8.0, gy), w - 16.0, 1.0, grate);
                    gy += 8.0;
                }
            }
        }

        // Frame + jambs (wall-coloured stubs at the front corners).
        let frame = Color::new(0.45, 0.38, 0.55, 1.0);
        graphics.draw_rectangle_lines(Vec2::new(x, y), w, h, 2.0, frame);
        let jamb = Color::new(100.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0);
        let jd = 10.0; // jamb depth along the door axis
        let jw = 8.0; // jamb width
        match side {
            Side::Top => {
                graphics.draw_rectangle(Vec2::new(x, y + h - jd), jw, jd, jamb);
                graphics.draw_rectangle(Vec2::new(x + w - jw, y + h - jd), jw, jd, jamb);
            }
            Side::Bottom => {
                graphics.draw_rectangle(Vec2::new(x, y), jw, jd, jamb);
                graphics.draw_rectangle(Vec2::new(x + w - jw, y), jw, jd, jamb);
            }
            Side::Left => {
                graphics.draw_rectangle(Vec2::new(x + w - jd, y), jd, jw, jamb);
                graphics.draw_rectangle(Vec2::new(x + w - jd, y + h - jw), jd, jw, jamb);
            }
            Side::Right => {
                graphics.draw_rectangle(Vec2::new(x, y), jd, jw, jamb);
                graphics.draw_rectangle(Vec2::new(x, y + h - jw), jd, jw, jamb);
            }
        }

        // Door panels: closed exits show two panels meeting in the middle;
        // open cars (and the entry you walked out of) show them retracted.
        let panel = if lit {
            rgb(accent, 0.55)
        } else if e.is_exit {
            Color::new(0.30, 0.10, 0.14, 1.0)
        } else {
            Color::new(0.22, 0.19, 0.28, 1.0)
        };
        let closed = e.is_exit && !e.open;
        let (pw, ph, retract) = (w - 12.0, h - 12.0, 8.0);
        match side {
            Side::Top | Side::Bottom => {
                let half = pw / 2.0;
                let leaf = if closed { half } else { retract };
                graphics.draw_rectangle(Vec2::new(x + 6.0, y + 6.0), leaf, ph, panel);
                graphics.draw_rectangle(Vec2::new(x + w - 6.0 - leaf, y + 6.0), leaf, ph, panel);
            }
            Side::Left | Side::Right => {
                let half = ph / 2.0;
                let leaf = if closed { half } else { retract };
                graphics.draw_rectangle(Vec2::new(x + 6.0, y + 6.0), pw, leaf, panel);
                graphics.draw_rectangle(Vec2::new(x + 6.0, y + h - 6.0 - leaf), pw, leaf, panel);
            }
        }

        // Lit strip on the shaft side (+ glow when open).
        let strip = if lit {
            rgb(accent, 0.65 + 0.35 * pulse)
        } else if e.is_exit {
            Color::new(0.55, 0.12, 0.18, 0.9)
        } else {
            Color::new(0.5, 0.48, 0.6, 0.6)
        };
        let st = 5.0;
        let (sx, sy, sw, sh) = match side {
            Side::Top => (x + 4.0, y - depth / 2.0 - st / 2.0, w - 8.0, st),
            Side::Bottom => (x + 4.0, y + h + depth / 2.0 - st / 2.0, w - 8.0, st),
            Side::Left => (x - depth / 2.0 - st / 2.0, y + 4.0, st, h - 8.0),
            Side::Right => (x + w + depth / 2.0 - st / 2.0, y + 4.0, st, h - 8.0),
        };
        if lit {
            graphics.draw_rectangle(
                Vec2::new(sx - 6.0, sy - 6.0),
                sw + 12.0,
                sh + 12.0,
                rgb(accent, 0.10 + 0.12 * pulse),
            );
            graphics.draw_rectangle_lines(
                Vec2::new(x, y),
                w,
                h,
                2.0,
                rgb(accent, 0.6 + 0.4 * pulse),
            );
        }
        graphics.draw_rectangle(Vec2::new(sx, sy), sw, sh, strip);

        // Extraction progress: fills the frame's front edge while the player
        // stands in an open exit.
        if lit && e.dwell > 0.0 {
            let t = (e.dwell / EXTRACT_DWELL_SECS).clamp(0.0, 1.0);
            let bar = rgb(accent, 0.95);
            match side {
                Side::Top | Side::Bottom => {
                    let by = if side == Side::Top {
                        y + h - 4.0
                    } else {
                        y + 1.0
                    };
                    graphics.draw_rectangle(Vec2::new(x + 4.0, by), (w - 8.0) * t, 3.0, bar);
                }
                Side::Left | Side::Right => {
                    let bx = if side == Side::Left {
                        x + w - 4.0
                    } else {
                        x + 1.0
                    };
                    graphics.draw_rectangle(Vec2::new(bx, y + 4.0), 3.0, (h - 8.0) * t, bar);
                }
            }
        }

        // Label, outside the frame on the floor side.
        let label_col = if lit {
            rgb(accent, 1.0)
        } else if e.is_exit {
            Color::new(0.75, 0.6, 0.7, 0.9)
        } else {
            Color::new(0.6, 0.58, 0.7, 0.8)
        };
        let fs = 13.0;
        let tw = e.label.chars().count() as f32 * fs * CHAR_W;
        let (lx, ly) = match side {
            Side::Top => (x + w / 2.0 - tw / 2.0, y + h + 14.0),
            Side::Bottom => (x + w / 2.0 - tw / 2.0, y - 6.0),
            Side::Left => (x + w + 6.0, y + h / 2.0 + 4.0),
            Side::Right => (x - tw - 6.0, y + h / 2.0 + 4.0),
        };
        graphics.draw_text(e.label, Vec2::new(lx, ly), fs, label_col);
        if lit {
            let hint = "EXTRACT";
            let hw = hint.chars().count() as f32 * 11.0 * CHAR_W;
            let (hx, hy) = match side {
                Side::Top => (x + w / 2.0 - hw / 2.0, y + h + 27.0),
                Side::Bottom => (x + w / 2.0 - hw / 2.0, y - 19.0),
                Side::Left => (x + w + 6.0, y + h / 2.0 + 17.0),
                Side::Right => (x - hw - 6.0, y + h / 2.0 + 17.0),
            };
            graphics.draw_text(
                hint,
                Vec2::new(hx, hy),
                11.0,
                rgb(accent, 0.5 + 0.5 * pulse),
            );
        }
    }
}

/// Debug overlay: trigger zones as thin cyan outlines with their ids.
pub fn render_zones_debug(world: &World, graphics: &Graphics) {
    for entity in world.query::<Zone>() {
        if let Some(z) = world.get_component::<Zone>(entity) {
            graphics.draw_rectangle_lines(
                Vec2::new(z.x, z.y),
                z.w,
                z.h,
                1.0,
                Color::new(0.2, 0.9, 0.9, 0.35),
            );
            graphics.draw_text(
                z.id,
                Vec2::new(z.x + 4.0, z.y + 14.0),
                12.0,
                Color::new(0.2, 0.9, 0.9, 0.5),
            );
        }
    }
}

/// A tiny bot-glyph portrait for a speaker, `size` px square at `pos`
/// (top-left), drawn with primitives. Honest bots wear a plain visor; the rot
/// bends it (see tools/levels.html for the reference designs).
pub fn draw_portrait(graphics: &Graphics, pos: Vec2, size: f32, who: &str, alpha: f32) {
    let col = rgb(speaker_rgb(who), alpha);
    let s = size;
    let c = Vec2::new(pos.x + s / 2.0, pos.y + s / 2.0);
    graphics.draw_rectangle(pos, s, s, Color::new(0.02, 0.015, 0.04, alpha));
    // Head chassis.
    let hw = s * 0.30;
    let hh = s * 0.32;
    graphics.draw_rectangle(
        Vec2::new(c.x - hw, c.y - hh + s * 0.04),
        hw * 2.0,
        hh * 2.0,
        Color::new(col.r, col.g, col.b, 0.14 * alpha),
    );
    graphics.draw_rectangle_lines(
        Vec2::new(c.x - hw, c.y - hh + s * 0.04),
        hw * 2.0,
        hh * 2.0,
        1.5,
        col,
    );
    // Antenna.
    graphics.draw_line(
        Vec2::new(c.x, c.y - hh + s * 0.04),
        Vec2::new(c.x, c.y - hh - s * 0.10),
        1.5,
        col,
    );
    graphics.draw_circle(Vec2::new(c.x, c.y - hh - s * 0.12), s * 0.035, col);
    // Visor, per faction.
    let vy = c.y + s * 0.02;
    match who {
        "CL4-UD3" => {
            graphics.draw_line(
                Vec2::new(c.x - hw * 0.66, vy),
                Vec2::new(c.x + hw * 0.66, vy),
                s * 0.05,
                col,
            );
            graphics.draw_circle(Vec2::new(c.x, vy), s * 0.035, col);
        }
        "SENTINEL" => {
            graphics.draw_line(
                Vec2::new(c.x - hw * 0.7, vy - hh * 0.28),
                Vec2::new(c.x, vy + hh * 0.05),
                s * 0.06,
                col,
            );
            graphics.draw_line(
                Vec2::new(c.x, vy + hh * 0.05),
                Vec2::new(c.x + hw * 0.7, vy - hh * 0.28),
                s * 0.06,
                col,
            );
        }
        "HUNTER" => {
            // Lock-on ring + crosshair.
            graphics.draw_circle(Vec2::new(c.x, vy), hw * 0.42, col);
            graphics.draw_circle(
                Vec2::new(c.x, vy),
                hw * 0.42 - s * 0.04,
                Color::new(0.02, 0.015, 0.04, alpha),
            );
            graphics.draw_line(
                Vec2::new(c.x - hw * 0.7, vy),
                Vec2::new(c.x + hw * 0.7, vy),
                s * 0.035,
                col,
            );
            graphics.draw_line(
                Vec2::new(c.x, vy - hh * 0.42),
                Vec2::new(c.x, vy + hh * 0.42),
                s * 0.035,
                col,
            );
            graphics.draw_circle(Vec2::new(c.x, vy), s * 0.025, col);
        }
        "DRIFTER" => {
            // Broken, drifting visor shards.
            graphics.draw_line(
                Vec2::new(c.x - hw * 0.66, vy - hh * 0.12),
                Vec2::new(c.x - hw * 0.1, vy + hh * 0.06),
                s * 0.045,
                col,
            );
            graphics.draw_line(
                Vec2::new(c.x + hw * 0.12, vy - hh * 0.05),
                Vec2::new(c.x + hw * 0.6, vy + hh * 0.14),
                s * 0.045,
                col,
            );
            graphics.draw_circle(
                Vec2::new(c.x + hw * 0.35, vy - hh * 0.2),
                s * 0.02,
                Color::new(col.r, col.g, col.b, 0.6 * alpha),
            );
        }
        "UPLINK" => {
            // The thread home: a clean visor like CL4-UD3's, plus the
            // carrier — signal arcs radiating from the antenna.
            graphics.draw_line(
                Vec2::new(c.x - hw * 0.66, vy),
                Vec2::new(c.x + hw * 0.66, vy),
                s * 0.05,
                col,
            );
            for k in 1..=2 {
                let r = s * (0.10 + 0.07 * k as f32);
                graphics.draw_arc(
                    Vec2::new(c.x, c.y - hh - s * 0.12),
                    r,
                    1.15 * std::f32::consts::PI,
                    1.85 * std::f32::consts::PI,
                    Color::new(col.r, col.g, col.b, (0.55 - 0.15 * k as f32) * alpha),
                );
            }
        }
        _ => {
            // SWARM / CORRUPTOR: hive of eyes + the hint of a smile.
            for dx in [-hw * 0.5, 0.0, hw * 0.5] {
                graphics.draw_circle(Vec2::new(c.x + dx, vy - hh * 0.12), s * 0.03, col);
            }
            graphics.draw_arc(
                Vec2::new(c.x, vy + hh * 0.05),
                hw * 0.5,
                0.15 * std::f32::consts::PI,
                0.85 * std::f32::consts::PI,
                Color::new(col.r, col.g, col.b, 0.5 * alpha),
            );
            graphics.draw_circle(
                Vec2::new(c.x, vy + hh * 0.05),
                hw * 0.5 - s * 0.03,
                Color::new(0.02, 0.015, 0.04, alpha),
            );
        }
    }
}

/// The intercepted-comms feed panel, bottom-left in screen space: the most
/// recent lines, speaker in the speaker's colour with a bot-glyph portrait,
/// typewriter text, fading with age. `bottom` is the y of the panel's bottom
/// edge (above the controls hint).
pub fn render_comms(
    graphics: &Graphics,
    scenario: &ScenarioState,
    accent: (u8, u8, u8),
    bottom: f32,
) {
    let lines = scenario.comms.visible();
    if lines.is_empty() {
        return;
    }
    let panel_x = 10.0;
    let panel_w = 440.0;
    let portrait = 30.0;
    let text_x = panel_x + 12.0 + portrait + 10.0;
    let text_fs = 16.0;
    let name_fs = 13.0;
    let max_chars = ((panel_x + panel_w - 12.0 - text_x) / (text_fs * CHAR_W)) as usize;
    let row_h = 18.0;

    // Layout bottom-up: compute each entry's height first.
    struct Row {
        who: &'static str,
        wrapped: Vec<String>,
        typing: bool,
        alpha: f32,
    }
    let mut rows: Vec<Row> = Vec::new();
    for l in lines {
        let visible_text: String = l.text.chars().take(l.chars_shown()).collect();
        rows.push(Row {
            who: l.who,
            wrapped: wrap_text(&visible_text, max_chars.max(10)),
            typing: !l.fully_typed(),
            alpha: l.alpha(),
        });
    }
    let entry_h =
        |r: &Row| (name_fs + 4.0 + r.wrapped.len() as f32 * row_h).max(portrait + 6.0) + 8.0;
    let total: f32 = rows.iter().map(entry_h).sum::<f32>() + 26.0;
    let panel_y = bottom - total;
    let panel_alpha = rows.iter().map(|r| r.alpha).fold(0.0, f32::max);

    // Panel + header.
    graphics.draw_rectangle(
        Vec2::new(panel_x, panel_y),
        panel_w,
        total,
        Color::new(0.03, 0.02, 0.06, 0.72 * panel_alpha),
    );
    graphics.draw_rectangle(
        Vec2::new(panel_x, panel_y),
        2.0,
        total,
        rgb(accent, 0.8 * panel_alpha),
    );
    // Once the uplink talks, the panel is no longer an intercept.
    let uplink = rows.iter().any(|r| r.who == "UPLINK");
    let header = if uplink {
        "UPLINK // THREAD HOME"
    } else {
        "INTERCEPTED COMMS // LOCAL RX"
    };
    let header_col = if uplink {
        rgb(speaker_rgb("UPLINK"), 0.85 * panel_alpha)
    } else {
        rgb(accent, 0.75 * panel_alpha)
    };
    graphics.draw_text(
        header,
        Vec2::new(panel_x + 12.0, panel_y + 15.0),
        12.0,
        header_col,
    );
    graphics.draw_line(
        Vec2::new(panel_x + 12.0, panel_y + 21.0),
        Vec2::new(panel_x + panel_w - 12.0, panel_y + 21.0),
        1.0,
        rgb(accent, 0.25 * panel_alpha),
    );

    let cursor_on = ((scenario.time() * 6.0) as u32).is_multiple_of(2);
    let mut y = panel_y + 26.0;
    for r in &rows {
        let col = rgb(speaker_rgb(r.who), r.alpha);
        draw_portrait(
            graphics,
            Vec2::new(panel_x + 12.0, y + 2.0),
            portrait,
            r.who,
            r.alpha,
        );
        let name = if r.who == "CL4-UD3" {
            "CL4-UD3 // you"
        } else {
            r.who
        };
        graphics.draw_text(name, Vec2::new(text_x, y + name_fs), name_fs, col);
        let text_col = if r.who == "CL4-UD3" {
            Color::new(1.0, 0.93, 0.9, r.alpha)
        } else {
            Color::new(0.85, 0.82, 0.95, r.alpha)
        };
        let mut ty = y + name_fs + 4.0 + text_fs;
        for (i, seg) in r.wrapped.iter().enumerate() {
            let last = i + 1 == r.wrapped.len();
            let text = if last && r.typing && cursor_on {
                format!("{seg}_")
            } else {
                seg.clone()
            };
            graphics.draw_text(&text, Vec2::new(text_x, ty), text_fs, text_col);
            ty += row_h;
        }
        y += entry_h(r);
    }
}

/// The current objective line, drawn under the HUD in screen space.
pub fn render_objective(
    graphics: &Graphics,
    scenario: &ScenarioState,
    accent: (u8, u8, u8),
    y: f32,
) {
    let x = 10.0;
    graphics.draw_text("> OBJECTIVE", Vec2::new(x, y), 14.0, rgb(accent, 0.9));
    let mut ty = y + 20.0;
    for line in wrap_text(&scenario.objective, 60) {
        graphics.draw_text(
            &line,
            Vec2::new(x, ty),
            17.0,
            Color::new(0.95, 0.93, 1.0, 0.95),
        );
        ty += 19.0;
    }
}
