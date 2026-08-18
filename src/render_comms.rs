//! In-game scenario overlays: elevators (world space), and the intercepted
//! comms feed + objective line (screen space, drawn after the camera reset).
//!
//! Everything here is drawn with the primitive `Graphics` API (rects, lines,
//! circles, arcs, text) so it needs no assets.

use crate::components::{Elevator, Zone};
use crate::ecs::World;
use crate::graphics::Graphics;
use crate::math::{Color, Vec2};
use crate::scenario::{speaker_rgb, ElevatorKind, ScenarioState};
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
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// The shaft side of the car `(x, y, w, h)`: the side a wall sits behind
/// (`in_wall` tests a world point), else the long side. A gate / doorway may
/// sit in a GAP of the perimeter wall: then no wall is behind its centre, but
/// both corners outside that edge touch one — probed second.
pub fn car_back_side(x: f32, y: f32, w: f32, h: f32, in_wall: impl Fn(Vec2) -> bool) -> Side {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let corners = |a: Vec2, b: Vec2| in_wall(a) && in_wall(b);
    if in_wall(Vec2::new(cx, y - 6.0)) {
        Side::Top
    } else if in_wall(Vec2::new(cx, y + h + 6.0)) {
        Side::Bottom
    } else if in_wall(Vec2::new(x - 6.0, cy)) {
        Side::Left
    } else if in_wall(Vec2::new(x + w + 6.0, cy)) {
        Side::Right
    } else if corners(Vec2::new(x - 6.0, y - 6.0), Vec2::new(x + w + 6.0, y - 6.0)) {
        Side::Top
    } else if corners(
        Vec2::new(x - 6.0, y + h + 6.0),
        Vec2::new(x + w + 6.0, y + h + 6.0),
    ) {
        Side::Bottom
    } else if corners(Vec2::new(x - 6.0, y - 6.0), Vec2::new(x - 6.0, y + h + 6.0)) {
        Side::Left
    } else if corners(
        Vec2::new(x + w + 6.0, y - 6.0),
        Vec2::new(x + w + 6.0, y + h + 6.0),
    ) {
        Side::Right
    } else if w >= h {
        Side::Top
    } else {
        Side::Left
    }
}

fn back_side(world: &World, e: &Elevator) -> Side {
    car_back_side(e.x, e.y, e.w, e.h, |p| {
        world
            .walls()
            .iter()
            .any(|w| p.x >= w.x && p.x <= w.x + w.width && p.y >= w.y && p.y <= w.y + w.height)
    })
}

/// What [`draw_elevator_car`] needs to know about a car — the `Elevator`
/// component's look-relevant fields, borrowed (so the level editor can draw
/// its owned, editable cars the same way).
#[derive(Clone, Copy)]
pub struct CarView<'a> {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: &'a str,
    pub is_exit: bool,
    pub open: bool,
    /// Seconds the player has stood inside (the extraction bar); 0 = none.
    pub dwell: f32,
    /// Lift car / sliding doorway / open gateway (which frame to draw).
    pub kind: ElevatorKind,
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
        let car = CarView {
            x: e.x,
            y: e.y,
            w: e.w,
            h: e.h,
            label: e.label,
            is_exit: e.is_exit,
            open: e.open,
            dwell: e.dwell,
            kind: e.kind,
        };
        draw_elevator_car(graphics, &car, side, accent, now);
    }
}

/// Draw one elevator car (see [`render_elevators`]) whose shaft is on `side`.
pub fn draw_elevator_car(
    graphics: &Graphics,
    e: &CarView,
    side: Side,
    accent: (u8, u8, u8),
    now: f32,
) {
    {
        let (x, y, w, h) = (e.x, e.y, e.w, e.h);
        let lit = e.is_exit && e.open;
        let pulse = 0.5 + 0.5 * (now * 4.0).sin();
        let depth = 16.0;

        // Doorways and gates have no cabin: their own frame, then the shared
        // strip / progress / label below.
        match e.kind {
            ElevatorKind::Door => draw_doorway(graphics, e, side, lit, accent, depth),
            ElevatorKind::Gate => draw_gateway(graphics, e, side, accent, depth, pulse),
            ElevatorKind::Lift => {}
        }
        if e.kind == ElevatorKind::Lift {
            // Recess: the shaft cuts into the wall behind the frame (drawn over
            // the wall band), then a dark shaft floor with a lighter plate inside.
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
                    graphics.draw_rectangle(
                        Vec2::new(x + w - 6.0 - leaf, y + 6.0),
                        leaf,
                        ph,
                        panel,
                    );
                }
                Side::Left | Side::Right => {
                    let half = ph / 2.0;
                    let leaf = if closed { half } else { retract };
                    graphics.draw_rectangle(Vec2::new(x + 6.0, y + 6.0), pw, leaf, panel);
                    graphics.draw_rectangle(
                        Vec2::new(x + 6.0, y + h - 6.0 - leaf),
                        pw,
                        leaf,
                        panel,
                    );
                }
            }
        } // end of the lift cabin

        // Lit strip on the shaft side (+ glow when open); for a doorway /
        // gate it is the header light beyond the opening.
        let strip = if lit {
            rgb(accent, 0.65 + 0.35 * pulse)
        } else if e.is_exit {
            Color::new(0.55, 0.12, 0.18, 0.9)
        } else {
            Color::new(0.5, 0.48, 0.6, 0.6)
        };
        let st = 5.0;
        let (sx, sy, sw, sh) = if e.kind == ElevatorKind::Lift {
            match side {
                Side::Top => (x + 4.0, y - depth / 2.0 - st / 2.0, w - 8.0, st),
                Side::Bottom => (x + 4.0, y + h + depth / 2.0 - st / 2.0, w - 8.0, st),
                Side::Left => (x - depth / 2.0 - st / 2.0, y + 4.0, st, h - 8.0),
                Side::Right => (x + w + depth / 2.0 - st / 2.0, y + 4.0, st, h - 8.0),
            }
        } else {
            match side {
                Side::Top => (x + 4.0, y - depth - st, w - 8.0, st),
                Side::Bottom => (x + 4.0, y + h + depth, w - 8.0, st),
                Side::Left => (x - depth - st, y + 4.0, st, h - 8.0),
                Side::Right => (x + w + depth, y + 4.0, st, h - 8.0),
            }
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

/// The rect just beyond the back edge of a portal (cut into the wall band),
/// `depth` deep.
fn back_recess(e: &CarView, side: Side, depth: f32) -> (f32, f32, f32, f32) {
    match side {
        Side::Top => (e.x, e.y - depth, e.w, depth),
        Side::Bottom => (e.x, e.y + e.h, e.w, depth),
        Side::Left => (e.x - depth, e.y, depth, e.h),
        Side::Right => (e.x + e.w, e.y, depth, e.h),
    }
}

/// The two end points of a portal's back edge (where jambs / posts stand).
fn back_edge_ends(e: &CarView, side: Side) -> (Vec2, Vec2) {
    match side {
        Side::Top => (Vec2::new(e.x, e.y), Vec2::new(e.x + e.w, e.y)),
        Side::Bottom => (Vec2::new(e.x, e.y + e.h), Vec2::new(e.x + e.w, e.y + e.h)),
        Side::Left => (Vec2::new(e.x, e.y), Vec2::new(e.x, e.y + e.h)),
        Side::Right => (Vec2::new(e.x + e.w, e.y), Vec2::new(e.x + e.w, e.y + e.h)),
    }
}

/// `ElevatorKind::Door`: a doorway in the wall behind the rect — an opening
/// cut into the wall band, two leaves that slide apart into the jambs when
/// open (meeting in the middle when closed), a threshold mat on the floor.
/// No cabin.
fn draw_doorway(
    graphics: &Graphics,
    e: &CarView,
    side: Side,
    lit: bool,
    accent: (u8, u8, u8),
    depth: f32,
) {
    let (rx, ry, rw, rh) = back_recess(e, side, depth);
    // The opening: dark interior beyond the wall.
    graphics.draw_rectangle(Vec2::new(rx, ry), rw, rh, Color::new(0.04, 0.03, 0.06, 1.0));
    // Threshold mat on the floor side (the trigger rect), faint.
    let mat = if lit {
        rgb(accent, 0.10)
    } else {
        Color::new(0.5, 0.45, 0.6, 0.08)
    };
    graphics.draw_rectangle(Vec2::new(e.x + 3.0, e.y + 3.0), e.w - 6.0, e.h - 6.0, mat);
    graphics.draw_rectangle_lines(
        Vec2::new(e.x, e.y),
        e.w,
        e.h,
        1.0,
        Color::new(0.45, 0.38, 0.55, 0.5),
    );
    // Leaves: thin panels inside the opening, sliding along the door axis.
    let closed = e.is_exit && !e.open;
    let panel = if lit {
        rgb(accent, 0.55)
    } else if e.is_exit {
        Color::new(0.30, 0.10, 0.14, 1.0)
    } else {
        Color::new(0.22, 0.19, 0.28, 1.0)
    };
    let t = 8.0; // leaf thickness (across the opening)
    let stub = 8.0; // how much of a leaf still shows when retracted
    match side {
        Side::Top | Side::Bottom => {
            let leaf = if closed { rw / 2.0 } else { stub };
            let ly = ry + (rh - t) / 2.0;
            graphics.draw_rectangle(Vec2::new(rx, ly), leaf, t, panel);
            graphics.draw_rectangle(Vec2::new(rx + rw - leaf, ly), leaf, t, panel);
        }
        Side::Left | Side::Right => {
            let leaf = if closed { rh / 2.0 } else { stub };
            let lx = rx + (rw - t) / 2.0;
            graphics.draw_rectangle(Vec2::new(lx, ry), t, leaf, panel);
            graphics.draw_rectangle(Vec2::new(lx, ry + rh - leaf), t, leaf, panel);
        }
    }
    // Jambs at both ends of the opening.
    let jamb = Color::new(0.45, 0.38, 0.55, 1.0);
    let (a, b) = back_edge_ends(e, side);
    let jw = 6.0;
    for p in [a, b] {
        let (jx, jy, jwid, jh) = match side {
            Side::Top => (p.x - jw / 2.0, ry, jw, rh + 4.0),
            Side::Bottom => (p.x - jw / 2.0, ry - 4.0, jw, rh + 4.0),
            Side::Left => (rx, p.y - jw / 2.0, rw + 4.0, jw),
            Side::Right => (rx - 4.0, p.y - jw / 2.0, rw + 4.0, jw),
        };
        graphics.draw_rectangle(Vec2::new(jx, jy), jwid, jh, jamb);
    }
}

/// `ElevatorKind::Gate`: an open gateway — the opening in the perimeter
/// wall (the street beyond), two scanner posts with a light each, and a
/// faint scan beam across the threshold. Nothing ever closes.
fn draw_gateway(
    graphics: &Graphics,
    e: &CarView,
    side: Side,
    accent: (u8, u8, u8),
    depth: f32,
    pulse: f32,
) {
    let deep = depth + 10.0;
    let (rx, ry, rw, rh) = back_recess(e, side, deep);
    // The opening: the outside (wet asphalt) beyond the wall band.
    graphics.draw_rectangle(
        Vec2::new(rx, ry),
        rw,
        rh,
        Color::new(0.06, 0.06, 0.085, 1.0),
    );
    // Stop line / threshold marking on the floor side.
    graphics.draw_rectangle_lines(
        Vec2::new(e.x, e.y),
        e.w,
        e.h,
        1.0,
        Color::new(0.75, 0.72, 0.6, 0.35),
    );
    // Scan beam across the threshold, breathing.
    let (a, b) = back_edge_ends(e, side);
    graphics.draw_line(a, b, 2.0, rgb(accent, 0.25 + 0.25 * pulse));
    // Posts (scanner pylons) at both ends, straddling the back edge.
    let post = Color::new(0.62, 0.60, 0.68, 1.0);
    let post_dark = Color::new(0.30, 0.28, 0.36, 1.0);
    let pw = 12.0;
    let pl = deep + 8.0; // post length along the depth axis
    for p in [a, b] {
        let (px, py, w, h) = match side {
            Side::Top => (p.x - pw / 2.0, ry - 2.0, pw, pl),
            Side::Bottom => (p.x - pw / 2.0, ry + rh - pl + 2.0, pw, pl),
            Side::Left => (rx - 2.0, p.y - pw / 2.0, pl, pw),
            Side::Right => (rx + rw - pl + 2.0, p.y - pw / 2.0, pl, pw),
        };
        graphics.draw_rectangle(Vec2::new(px, py), w, h, post_dark);
        graphics.draw_rectangle(Vec2::new(px + 2.0, py + 2.0), w - 4.0, h - 4.0, post);
        // Scanner light on the inner face.
        let c = Vec2::new(px + w / 2.0, py + h / 2.0);
        graphics.draw_circle(c, 3.0, rgb(accent, 0.7 + 0.3 * pulse));
        graphics.draw_circle(c, 6.0, rgb(accent, 0.12 + 0.12 * pulse));
    }
}

/// The dim centred caption of a running scenario `hold` ("SCANNING…"):
/// a soft dark band across the screen at ~38% height with the text in the
/// floor accent. `t` (seconds since the hold started) drives a slow blink of
/// the trailing dots. Screen space.
pub fn render_hold_caption(graphics: &Graphics, text: &str, accent: (u8, u8, u8), t: f32) {
    let w = graphics.width();
    let h = graphics.height();
    let fs = 28.0;
    let tw = text.chars().count() as f32 * fs * CHAR_W;
    let cy = h * 0.38;
    graphics.draw_rectangle(
        Vec2::new(0.0, cy - fs * 0.9),
        w,
        fs * 1.6,
        Color::new(0.0, 0.0, 0.0, 0.35),
    );
    let blink = 0.55 + 0.25 * (t * 3.0).sin();
    graphics.draw_text(
        text,
        Vec2::new(w / 2.0 - tw / 2.0, cy),
        fs,
        rgb(accent, blink),
    );
}

/// The tutorial gate prompt: a centred lower-third caption on a dim band
/// while the world is frozen on a `gate`. The input name (the part of the
/// text before the em-dash separator, e.g. "LEFT CLICK — PUNCH") pulses in
/// the floor's accent colour; the rest stays white.
pub fn render_gate_prompt(
    graphics: &Graphics,
    gate: &crate::scenario::GateDef,
    accent: (u8, u8, u8),
    t: f32,
) {
    let w = graphics.width();
    let h = graphics.height();
    let fs = 30.0;
    let cy = h * 0.72;
    graphics.draw_rectangle(
        Vec2::new(0.0, cy - fs * 0.95),
        w,
        fs * 1.8,
        Color::new(0.0, 0.0, 0.0, 0.55),
    );
    // Split "INPUT — action": the input half is highlighted.
    let (head, tail) = match gate.text.split_once(" — ") {
        Some((a, b)) => (a, Some(b)),
        None => (gate.text, None),
    };
    let head_w = head.chars().count() as f32 * fs * CHAR_W;
    let sep = " — ";
    let sep_w = if tail.is_some() {
        sep.chars().count() as f32 * fs * CHAR_W
    } else {
        0.0
    };
    let tail_w = tail
        .map(|s| s.chars().count() as f32 * fs * CHAR_W)
        .unwrap_or(0.0);
    let total = head_w + sep_w + tail_w;
    let x0 = w / 2.0 - total / 2.0;
    let pulse = 0.7 + 0.3 * (t * 4.0).sin();
    graphics.draw_text(head, Vec2::new(x0, cy), fs, rgb(accent, pulse));
    if let Some(tail) = tail {
        graphics.draw_text(
            sep,
            Vec2::new(x0 + head_w, cy),
            fs,
            Color::new(1.0, 1.0, 1.0, 0.45),
        );
        graphics.draw_text(
            tail,
            Vec2::new(x0 + head_w + sep_w, cy),
            fs,
            Color::new(1.0, 1.0, 1.0, 0.92),
        );
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
