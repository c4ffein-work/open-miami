//! The datacenter prop sprite library: top-down set dressing for the server
//! floors — racks, switching, cooling, power, storage and hazard furniture —
//! drawn entirely from the 2D command-stream primitives (no assets, no new
//! dependencies) and animated by the continuous clock (blinking LEDs,
//! spinning fans, rising bubbles, a patrolling tape-robot arm).
//!
//! Each prop is designed in a local 100x100 box centred on the origin
//! (y down) and drawn through the transform stack, so it can be placed at
//! any size. `draw_prop(g, idx, center, size_px, time)` is the whole API;
//! the `?viz` SPRITES tab's PROPS page is the gallery.

/// Display names, indexed by prop id (the order of the library).
pub const PROP_NAMES: [&str; 24] = [
    "RACK / CLOSED",
    "RACK / OPEN",
    "RACK / BURNT",
    "BLADE STACK",
    "CORE SWITCH",
    "PATCH BAY",
    "OPERATOR DESK",
    "MONITOR BANK",
    "HOLO TABLE",
    "CRAC COOLER",
    "FLOOR VENT",
    "EXHAUST FAN",
    "COOLANT TANK",
    "PIPE RUN",
    "UPS CABINET",
    "GENERATOR",
    "CABLE TRAY",
    "CABLE COIL",
    "TAPE LIBRARY",
    "SUPPLY CRATE",
    "SECURITY CAM",
    "FIRE SUPPRESSOR",
    "HAZARD PAD",
    "UPLINK OBELISK",
];

/// Number of props in the library.
pub const PROP_COUNT: usize = PROP_NAMES.len();

#[cfg(target_arch = "wasm32")]
pub use wasm::draw_prop;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::PROP_COUNT;
    use crate::graphics::Graphics;
    use crate::math::{Color, Vec2};
    use std::f32::consts::{FRAC_PI_2, PI, TAU};

    // ---- the shared datacenter palette --------------------------------------
    const STEEL_DARK: Color = Color::new(0.15, 0.14, 0.20, 1.0);
    const STEEL: Color = Color::new(0.24, 0.22, 0.30, 1.0);
    const TRIM: Color = Color::new(0.38, 0.34, 0.46, 1.0);
    const PANEL: Color = Color::new(0.08, 0.07, 0.12, 1.0);
    const LED_GREEN: Color = Color::new(0.30, 1.0, 0.55, 1.0);
    const LED_AMBER: Color = Color::new(1.0, 0.72, 0.20, 1.0);
    const LED_RED: Color = Color::new(1.0, 0.22, 0.28, 1.0);
    const GLOW_CYAN: Color = Color::new(0.30, 0.95, 1.0, 1.0);
    const GLOW_MAGENTA: Color = Color::new(0.95, 0.28, 0.78, 1.0);
    const CREAM: Color = Color::new(0.96, 0.93, 0.86, 1.0);
    const HAZARD_YELLOW: Color = Color::new(0.95, 0.78, 0.10, 1.0);
    const COPPER: Color = Color::new(0.62, 0.38, 0.28, 1.0);

    // ---- tiny drawing / animation helpers -----------------------------------
    fn rect(g: &Graphics, x: f32, y: f32, w: f32, h: f32, c: Color) {
        g.draw_rectangle(Vec2::new(x, y), w, h, c);
    }
    fn frame(g: &Graphics, x: f32, y: f32, w: f32, h: f32, t: f32, c: Color) {
        g.draw_rectangle_lines(Vec2::new(x, y), w, h, t, c);
    }
    fn circle(g: &Graphics, x: f32, y: f32, r: f32, c: Color) {
        g.draw_circle(Vec2::new(x, y), r, c);
    }
    fn line(g: &Graphics, x1: f32, y1: f32, x2: f32, y2: f32, th: f32, c: Color) {
        g.draw_line(Vec2::new(x1, y1), Vec2::new(x2, y2), th, c);
    }
    fn alpha(c: Color, a: f32) -> Color {
        Color::new(c.r, c.g, c.b, a)
    }
    /// Small deterministic hash -> 0..1, for per-cell variety that stays put.
    fn rnd(a: u32, b: u32) -> f32 {
        let mut x = a
            .wrapping_mul(374_761_393)
            .wrapping_add(b.wrapping_mul(668_265_263));
        x = (x ^ (x >> 13)).wrapping_mul(1_274_126_177);
        ((x ^ (x >> 16)) & 0xff_ffff) as f32 / 0xff_ffff as f32
    }
    /// Square-wave blink with `duty` fraction on, at `hz`, offset by `phase`.
    fn blink(time: f32, hz: f32, phase: f32, duty: f32) -> bool {
        (time * hz + phase).fract() < duty
    }

    /// Draw prop `idx` (see [`super::PROP_NAMES`]) centred on `center` at
    /// `size_px` px, animated by the continuous clock `time` (seconds).
    pub fn draw_prop(g: &Graphics, idx: usize, center: Vec2, size_px: f32, time: f32) {
        g.save();
        g.translate(center.x, center.y);
        let s = size_px / 100.0;
        g.scale(s, s);
        match idx % PROP_COUNT {
            0 => rack_closed(g, time),
            1 => rack_open(g, time),
            2 => rack_burnt(g, time),
            3 => blade_stack(g, time),
            4 => core_switch(g, time),
            5 => patch_bay(g, time),
            6 => operator_desk(g, time),
            7 => monitor_bank(g, time),
            8 => holo_table(g, time),
            9 => crac_cooler(g, time),
            10 => floor_vent(g, time),
            11 => exhaust_fan(g, time),
            12 => coolant_tank(g, time),
            13 => pipe_run(g, time),
            14 => ups_cabinet(g, time),
            15 => generator(g, time),
            16 => cable_tray(g, time),
            17 => cable_coil(g, time),
            18 => tape_library(g, time),
            19 => supply_crate(g, time),
            20 => security_cam(g, time),
            21 => fire_suppressor(g, time),
            22 => hazard_pad(g, time),
            _ => uplink_obelisk(g, time),
        }
        g.restore();
    }

    /// 0 — closed rack cabinet: vented door, handle, a status LED pair.
    fn rack_closed(g: &Graphics, time: f32) {
        rect(g, -35.0, -45.0, 70.0, 90.0, STEEL);
        frame(g, -35.0, -45.0, 70.0, 90.0, 2.0, TRIM);
        for i in 0..8 {
            rect(g, -27.0, -37.0 + i as f32 * 8.0, 54.0, 3.0, PANEL);
        }
        rect(g, -31.0, -6.0, 4.0, 12.0, TRIM); // door handle
        let ok = blink(time, 0.8, 0.0, 0.85);
        rect(g, 14.0, 32.0, 6.0, 6.0, if ok { LED_GREEN } else { PANEL });
        let busy = blink(time, 3.1, 0.4, 0.5);
        rect(
            g,
            24.0,
            32.0,
            6.0,
            6.0,
            if busy { LED_AMBER } else { PANEL },
        );
    }

    /// 1 — open rack: a column of servers, one slot pulled for maintenance.
    fn rack_open(g: &Graphics, time: f32) {
        rect(g, -35.0, -45.0, 70.0, 90.0, STEEL_DARK);
        frame(g, -35.0, -45.0, 70.0, 90.0, 2.0, TRIM);
        for i in 0..7 {
            let y = -40.0 + i as f32 * 12.0;
            if i == 4 {
                // The pulled slot: dark bay with loose cabling.
                rect(g, -30.0, y, 60.0, 10.0, PANEL);
                line(g, -24.0, y + 3.0, -6.0, y + 8.0, 1.5, COPPER);
                line(g, -6.0, y + 8.0, 12.0, y + 4.0, 1.5, GLOW_CYAN);
                continue;
            }
            rect(g, -30.0, y, 60.0, 10.0, STEEL);
            rect(g, -30.0, y, 4.0, 10.0, TRIM);
            rect(g, 26.0, y, 4.0, 10.0, TRIM);
            // Two activity LEDs per unit, each with its own rhythm.
            for l in 0..2u32 {
                let on = blink(time, 1.5 + rnd(i as u32, l) * 4.0, rnd(l, i as u32), 0.55);
                let c = if !on {
                    PANEL
                } else if rnd(i as u32 * 7 + l, 13) > 0.3 {
                    LED_GREEN
                } else {
                    LED_AMBER
                };
                rect(g, 6.0 + l as f32 * 8.0, y + 3.0, 4.0, 4.0, c);
            }
        }
    }

    /// 2 — burnt-out rack: dead cabinet, door hanging open, scorch, sparks.
    fn rack_burnt(g: &Graphics, time: f32) {
        rect(
            g,
            -35.0,
            -45.0,
            70.0,
            90.0,
            Color::new(0.09, 0.08, 0.11, 1.0),
        );
        frame(
            g,
            -35.0,
            -45.0,
            70.0,
            90.0,
            2.0,
            Color::new(0.20, 0.17, 0.22, 1.0),
        );
        // Scorch blooms around the failed unit.
        circle(g, 2.0, -12.0, 19.0, Color::new(0.03, 0.02, 0.04, 0.9));
        circle(g, 14.0, -26.0, 10.0, Color::new(0.05, 0.03, 0.05, 0.9));
        // The door hangs off its lower hinge.
        g.save();
        g.translate(-35.0, 8.0);
        g.rotate(0.35);
        rect(g, 0.0, 0.0, 66.0, 38.0, Color::new(0.12, 0.10, 0.14, 1.0));
        frame(
            g,
            0.0,
            0.0,
            66.0,
            38.0,
            1.5,
            Color::new(0.22, 0.18, 0.24, 1.0),
        );
        g.restore();
        // Something in there still shorts now and then.
        for k in 0..3u32 {
            if ((time * (11.0 + k as f32 * 3.7) + k as f32 * 1.9).sin()) > 0.94 {
                let x = -14.0 + rnd(k, 5) * 28.0;
                let y = -30.0 + rnd(k, 9) * 30.0;
                rect(g, x, y, 3.0, 3.0, Color::new(1.0, 0.92, 0.4, 0.95));
                line(g, x + 1.0, y + 1.0, x + 5.0, y - 4.0, 1.0, LED_AMBER);
            }
        }
    }

    /// 3 — dense blade stack with a pulsing coolant edge-light.
    fn blade_stack(g: &Graphics, time: f32) {
        for i in 0..9 {
            let y = -42.0 + i as f32 * 9.5;
            rect(
                g,
                -40.0,
                y,
                80.0,
                7.5,
                if i % 2 == 0 { STEEL } else { STEEL_DARK },
            );
            let on = blink(time, 2.0 + rnd(i as u32, 3) * 5.0, rnd(3, i as u32), 0.5);
            rect(
                g,
                -36.0,
                y + 2.0,
                3.5,
                3.5,
                if on { LED_GREEN } else { PANEL },
            );
        }
        frame(g, -40.0, -42.0, 80.0, 85.5, 1.5, TRIM);
        let pulse = 0.55 + 0.35 * (time * 2.2).sin();
        rect(g, 34.0, -42.0, 4.0, 85.5, alpha(GLOW_CYAN, pulse));
    }

    /// 4 — core switch: two rows of ports blinking traffic, cyan uplinks.
    fn core_switch(g: &Graphics, time: f32) {
        rect(g, -45.0, -20.0, 90.0, 40.0, STEEL);
        frame(g, -45.0, -20.0, 90.0, 40.0, 2.0, TRIM);
        rect(g, -45.0, -20.0, 90.0, 6.0, STEEL_DARK);
        let tick = (time * 6.0) as u32;
        for row in 0..2u32 {
            for i in 0..9u32 {
                let x = -40.0 + i as f32 * 8.0;
                let y = if row == 0 { -9.0 } else { 3.0 };
                rect(g, x, y, 6.0, 6.0, PANEL);
                let r = rnd(i * 2 + row, tick);
                let c = if r > 0.6 {
                    LED_GREEN
                } else if r > 0.45 {
                    LED_AMBER
                } else {
                    PANEL
                };
                rect(g, x + 1.5, y + 1.5, 3.0, 3.0, c);
            }
        }
        // The pair of uplink ports, breathing cyan.
        let pulse = 0.5 + 0.5 * (time * 3.0).sin();
        rect(
            g,
            34.0,
            -9.0,
            8.0,
            8.0,
            alpha(GLOW_CYAN, 0.35 + 0.6 * pulse),
        );
        rect(g, 34.0, 3.0, 8.0, 8.0, alpha(GLOW_CYAN, 0.95 - 0.6 * pulse));
    }

    /// 5 — patch bay: colour-coded cables sagging between two port columns.
    fn patch_bay(g: &Graphics, time: f32) {
        rect(g, -45.0, -32.0, 90.0, 64.0, STEEL_DARK);
        frame(g, -45.0, -32.0, 90.0, 64.0, 2.0, TRIM);
        let cable_colors = [COPPER, GLOW_CYAN, GLOW_MAGENTA, LED_GREEN, LED_AMBER, TRIM];
        // A fixed shuffle: left port k patches to right port (k * 5 + 2) % 6.
        for (k, &c) in cable_colors.iter().enumerate() {
            let ly = -24.0 + k as f32 * 9.5;
            let ry = -24.0 + ((k * 5 + 2) % 6) as f32 * 9.5;
            circle(g, -38.0, ly, 3.0, PANEL);
            circle(g, 38.0, ry, 3.0, PANEL);
            let sag = 6.0 + 2.0 * ((time * 0.7 + k as f32).sin()); // cables breathe
            let mid = (ly + ry) / 2.0 + sag;
            line(g, -35.0, ly, 0.0, mid, 2.0, c);
            line(g, 0.0, mid, 35.0, ry, 2.0, c);
        }
    }

    /// 6 — operator desk: terminal with a live scanline, keyboard, cold coffee.
    fn operator_desk(g: &Graphics, time: f32) {
        rect(
            g,
            -45.0,
            -25.0,
            90.0,
            50.0,
            Color::new(0.28, 0.20, 0.24, 1.0),
        );
        frame(
            g,
            -45.0,
            -25.0,
            90.0,
            50.0,
            2.0,
            Color::new(0.40, 0.30, 0.34, 1.0),
        );
        // Terminal.
        rect(g, -22.0, -20.0, 44.0, 28.0, PANEL);
        rect(
            g,
            -19.0,
            -17.0,
            38.0,
            22.0,
            Color::new(0.06, 0.16, 0.13, 1.0),
        );
        for i in 0..4 {
            let w = 12.0 + rnd(i, 7) * 20.0;
            rect(
                g,
                -17.0,
                -14.0 + i as f32 * 5.0,
                w,
                2.0,
                Color::new(0.25, 0.85, 0.45, 0.8),
            );
        }
        let scanl = -17.0 + (time * 0.6).fract() * 22.0;
        rect(g, -19.0, scanl, 38.0, 2.0, Color::new(0.4, 1.0, 0.7, 0.35));
        // Keyboard and the mug that never gets finished.
        rect(g, -16.0, 12.0, 32.0, 9.0, STEEL_DARK);
        for i in 0..8 {
            rect(g, -14.0 + i as f32 * 4.0, 14.0, 2.5, 2.0, TRIM);
            rect(g, -14.0 + i as f32 * 4.0, 17.5, 2.5, 2.0, TRIM);
        }
        circle(g, 33.0, 10.0, 5.5, Color::new(0.85, 0.47, 0.34, 1.0));
        circle(g, 33.0, 10.0, 3.5, Color::new(0.16, 0.09, 0.07, 1.0));
    }

    /// 7 — monitor bank: four feeds — terminal, logs, a graph, and static.
    fn monitor_bank(g: &Graphics, time: f32) {
        let cells = [(-41.0, -31.0), (1.0, -31.0), (-41.0, 3.0), (1.0, 3.0)];
        for (m, &(x, y)) in cells.iter().enumerate() {
            rect(g, x, y, 40.0, 28.0, PANEL);
            frame(g, x, y, 40.0, 28.0, 1.5, TRIM);
            match m {
                0 => {
                    // Green terminal, lines crawling upward.
                    rect(
                        g,
                        x + 3.0,
                        y + 3.0,
                        34.0,
                        22.0,
                        Color::new(0.05, 0.14, 0.10, 1.0),
                    );
                    for i in 0..4u32 {
                        let ph = ((time * 0.5 + i as f32 * 0.25).fract()) * 22.0;
                        let w = 8.0 + rnd(i, 21) * 22.0;
                        rect(
                            g,
                            x + 5.0,
                            y + 3.0 + ph,
                            w,
                            1.8,
                            Color::new(0.3, 0.9, 0.5, 0.8),
                        );
                    }
                }
                1 => {
                    // Amber log feed.
                    rect(
                        g,
                        x + 3.0,
                        y + 3.0,
                        34.0,
                        22.0,
                        Color::new(0.14, 0.09, 0.03, 1.0),
                    );
                    for i in 0..5u32 {
                        let w = 6.0 + rnd(i, (time * 2.0) as u32) * 26.0;
                        rect(
                            g,
                            x + 5.0,
                            y + 5.0 + i as f32 * 4.0,
                            w,
                            1.8,
                            alpha(LED_AMBER, 0.75),
                        );
                    }
                }
                2 => {
                    // Cyan metric graph, scrolling.
                    rect(
                        g,
                        x + 3.0,
                        y + 3.0,
                        34.0,
                        22.0,
                        Color::new(0.03, 0.09, 0.12, 1.0),
                    );
                    let mut lx = x + 4.0;
                    let mut ly = y + 16.0;
                    for i in 0..7 {
                        let nx = lx + 4.6;
                        let ny = y + 20.0 - (2.0 + 12.0 * rnd(i, (time * 1.5) as u32 + i));
                        line(g, lx, ly, nx, ny, 1.5, GLOW_CYAN);
                        lx = nx;
                        ly = ny;
                    }
                }
                _ => {
                    // Dead channel: rolling static.
                    let tick = (time * 20.0) as u32;
                    for i in 0..24u32 {
                        let v = rnd(i, tick);
                        rect(
                            g,
                            x + 3.0 + (i % 6) as f32 * 5.7,
                            y + 3.0 + (i / 6) as f32 * 5.5,
                            5.7,
                            5.5,
                            Color::new(v * 0.5, v * 0.5, v * 0.55, 1.0),
                        );
                    }
                }
            }
        }
    }

    /// 8 — holo table: a spinning wireframe projection over a round pedestal.
    fn holo_table(g: &Graphics, time: f32) {
        circle(g, 0.0, 14.0, 27.0, TRIM);
        circle(g, 0.0, 14.0, 23.0, STEEL_DARK);
        circle(
            g,
            0.0,
            14.0,
            6.0,
            alpha(GLOW_CYAN, 0.5 + 0.3 * (time * 2.0).sin()),
        );
        // The projected wireframe: a triangle spinning above the emitter.
        let mut pts = [(0.0f32, 0.0f32); 3];
        for (k, p) in pts.iter_mut().enumerate() {
            let a = time * 0.9 + k as f32 * (TAU / 3.0);
            *p = (a.cos() * 26.0, a.sin() * 10.0 - 18.0);
        }
        let flick = 0.55 + 0.25 * (time * 17.0).sin();
        for k in 0..3 {
            let (x1, y1) = pts[k];
            let (x2, y2) = pts[(k + 1) % 3];
            line(g, x1, y1, x2, y2, 1.5, alpha(GLOW_CYAN, flick));
            // Projection beams back down to the emitter.
            line(g, x1, y1, 0.0, 12.0, 1.0, alpha(GLOW_CYAN, 0.18));
        }
    }

    /// 9 — CRAC cooling unit: big housing, spinning blower, status lights.
    fn crac_cooler(g: &Graphics, time: f32) {
        rect(g, -40.0, -45.0, 80.0, 90.0, STEEL);
        frame(g, -40.0, -45.0, 80.0, 90.0, 2.0, TRIM);
        for i in 0..3 {
            rect(g, -32.0, -39.0 + i as f32 * 6.0, 64.0, 2.5, PANEL);
        }
        circle(g, 0.0, 12.0, 26.0, PANEL);
        for k in 0..4 {
            let a = time * 4.0 + k as f32 * FRAC_PI_2;
            g.draw_arc(Vec2::new(0.0, 12.0), 23.0, a, a + 0.62, STEEL_DARK);
            g.draw_arc(Vec2::new(0.0, 12.0), 23.0, a + 0.1, a + 0.5, TRIM);
        }
        circle(g, 0.0, 12.0, 5.0, TRIM);
        let ok = blink(time, 1.2, 0.0, 0.9);
        rect(
            g,
            28.0,
            -41.0,
            6.0,
            6.0,
            if ok { LED_GREEN } else { LED_RED },
        );
    }

    /// 10 — raised-floor vent tile with a faint airflow shimmer.
    fn floor_vent(g: &Graphics, time: f32) {
        rect(
            g,
            -42.0,
            -42.0,
            84.0,
            84.0,
            Color::new(0.13, 0.12, 0.17, 1.0),
        );
        frame(g, -42.0, -42.0, 84.0, 84.0, 2.0, TRIM);
        for i in 0..7 {
            let o = -36.0 + i as f32 * 12.0;
            rect(g, o, -36.0, 3.0, 72.0, PANEL);
            rect(g, -36.0, o, 72.0, 3.0, PANEL);
        }
        // Cold air rippling out of the grille.
        for k in 0..2u32 {
            let ph = (time * 0.5 + k as f32 * 0.5).fract();
            let r = 8.0 + ph * 30.0;
            circle(g, 0.0, 0.0, r, Color::new(0.5, 0.8, 1.0, 0.10 * (1.0 - ph)));
        }
    }

    /// 11 — wall exhaust fan: five blades behind a safety cross.
    fn exhaust_fan(g: &Graphics, time: f32) {
        rect(g, -42.0, -42.0, 84.0, 84.0, STEEL_DARK);
        frame(g, -42.0, -42.0, 84.0, 84.0, 2.0, TRIM);
        circle(g, 0.0, 0.0, 36.0, PANEL);
        for k in 0..5 {
            let a = time * 3.2 + k as f32 * (TAU / 5.0);
            g.draw_arc(Vec2::new(0.0, 0.0), 32.0, a, a + 0.7, STEEL);
        }
        circle(g, 0.0, 0.0, 8.0, TRIM);
        line(g, -36.0, 0.0, 36.0, 0.0, 3.0, alpha(TRIM, 0.85));
        line(g, 0.0, -36.0, 0.0, 36.0, 3.0, alpha(TRIM, 0.85));
    }

    /// 12 — coolant tank seen from above: liquid, rising bubbles, bolted hatch.
    fn coolant_tank(g: &Graphics, time: f32) {
        circle(g, 0.0, 0.0, 38.0, TRIM);
        circle(g, 0.0, 0.0, 34.0, Color::new(0.08, 0.20, 0.26, 1.0));
        circle(g, 0.0, 0.0, 30.0, Color::new(0.10, 0.42, 0.52, 0.85));
        for k in 0..5u32 {
            let ph = (time * 0.28 + k as f32 * 0.37).fract();
            let x = (k as f32 * 2.4).sin() * 17.0 * (1.0 - ph * 0.4);
            let y = 20.0 - ph * 40.0;
            let r = 1.5 + rnd(k, 2) * 2.5;
            circle(g, x, y, r, Color::new(0.6, 0.9, 1.0, 0.5 * (1.0 - ph)));
        }
        circle(g, 0.0, 0.0, 9.0, STEEL);
        for k in 0..4 {
            let a = k as f32 * FRAC_PI_2 + 0.4;
            circle(g, a.cos() * 6.0, a.sin() * 6.0, 1.5, PANEL);
        }
    }

    /// 13 — overhead pipe run: coolant + power conduit, flanges, a red valve.
    fn pipe_run(g: &Graphics, time: f32) {
        rect(
            g,
            -50.0,
            -20.0,
            100.0,
            13.0,
            Color::new(0.30, 0.26, 0.34, 1.0),
        );
        rect(g, -50.0, -19.0, 100.0, 3.0, alpha(CREAM, 0.25)); // glint
        rect(g, -50.0, 7.0, 100.0, 13.0, COPPER);
        rect(g, -50.0, 8.0, 100.0, 3.0, alpha(CREAM, 0.2));
        for &x in &[-30.0, 10.0] {
            rect(g, x, -22.0, 6.0, 17.0, TRIM);
        }
        for &x in &[-12.0, 34.0] {
            rect(g, x, 5.0, 6.0, 17.0, TRIM);
        }
        // The valve wheel creeps as pressure is trimmed.
        let a0 = 0.3 * (time * 0.6).sin();
        circle(g, -34.0, 13.0, 9.5, Color::new(0.72, 0.16, 0.20, 1.0));
        circle(g, -34.0, 13.0, 3.0, STEEL_DARK);
        for k in 0..3 {
            let a = a0 + k as f32 * (PI / 3.0);
            line(
                g,
                -34.0 - a.cos() * 9.0,
                13.0 - a.sin() * 9.0,
                -34.0 + a.cos() * 9.0,
                13.0 + a.sin() * 9.0,
                2.0,
                Color::new(0.5, 0.10, 0.14, 1.0),
            );
        }
    }

    /// 14 — UPS cabinet: charge display counting, battery pairs, bolt badge.
    fn ups_cabinet(g: &Graphics, time: f32) {
        rect(g, -30.0, -45.0, 60.0, 90.0, STEEL);
        frame(g, -30.0, -45.0, 60.0, 90.0, 2.0, TRIM);
        rect(g, -22.0, -39.0, 44.0, 14.0, PANEL);
        // Charge readout breathing between 4 and 5 bars: holding, on mains.
        let fill = 4 + if blink(time, 0.5, 0.0, 0.5) { 1 } else { 0 };
        for i in 0..5 {
            let c = if i < fill { LED_GREEN } else { STEEL_DARK };
            rect(g, -19.0 + i as f32 * 8.0, -36.0, 6.0, 8.0, c);
        }
        for row in 0..3 {
            let y = -18.0 + row as f32 * 18.0;
            for col in 0..2 {
                let x = -24.0 + col as f32 * 26.0;
                rect(g, x, y, 22.0, 13.0, STEEL_DARK);
                rect(g, x + 2.0, y + 4.0, 3.0, 5.0, CREAM); // + terminal
                rect(g, x + 17.0, y + 4.0, 3.0, 5.0, TRIM); // - terminal
            }
        }
        // Lightning badge.
        let on = blink(time, 1.0, 0.25, 0.7);
        let c = if on {
            HAZARD_YELLOW
        } else {
            alpha(HAZARD_YELLOW, 0.35)
        };
        line(g, 5.0, 29.0, -1.0, 37.0, 2.5, c);
        line(g, -1.0, 37.0, 4.0, 37.0, 2.5, c);
        line(g, 4.0, 37.0, -3.0, 44.0, 2.5, c);
    }

    /// 15 — backup generator: engine block, alternator, smoking exhaust.
    fn generator(g: &Graphics, time: f32) {
        rect(g, -45.0, 25.0, 90.0, 14.0, STEEL_DARK); // skid
        frame(g, -45.0, 25.0, 90.0, 14.0, 1.5, TRIM);
        rect(
            g,
            -40.0,
            -25.0,
            55.0,
            50.0,
            Color::new(0.30, 0.28, 0.24, 1.0),
        );
        frame(
            g,
            -40.0,
            -25.0,
            55.0,
            50.0,
            2.0,
            Color::new(0.42, 0.39, 0.33, 1.0),
        );
        for i in 0..5 {
            rect(
                g,
                -34.0 + i as f32 * 10.0,
                -20.0,
                4.0,
                40.0,
                Color::new(0.20, 0.19, 0.16, 1.0),
            );
        }
        circle(g, 28.0, 0.0, 16.0, STEEL);
        circle(g, 28.0, 0.0, 5.0, TRIM);
        // Exhaust stack + drifting smoke.
        rect(g, -36.0, -45.0, 10.0, 20.0, STEEL_DARK);
        for k in 0..3u32 {
            let ph = (time * 0.45 + k as f32 / 3.0).fract();
            circle(
                g,
                -31.0 + ph * 10.0 + k as f32 * 2.0,
                -48.0 - ph * 12.0,
                3.0 + ph * 5.0,
                Color::new(0.5, 0.5, 0.55, 0.25 * (1.0 - ph)),
            );
        }
        // Fuel gauge, needle trembling while it runs.
        circle(g, -12.0, 8.0, 7.0, CREAM);
        let a = -1.9 + 0.12 * (time * 9.0).sin();
        line(
            g,
            -12.0,
            8.0,
            -12.0 + a.cos() * 5.5,
            8.0 + a.sin() * 5.5,
            1.5,
            LED_RED,
        );
    }

    /// 16 — open cable tray: a river of colour-coded runs.
    fn cable_tray(g: &Graphics, time: f32) {
        rect(
            g,
            -48.0,
            -20.0,
            96.0,
            40.0,
            Color::new(0.14, 0.13, 0.18, 1.0),
        );
        rect(g, -48.0, -24.0, 96.0, 4.0, TRIM);
        rect(g, -48.0, 20.0, 96.0, 4.0, TRIM);
        let colors = [COPPER, GLOW_CYAN, LED_GREEN, GLOW_MAGENTA, LED_AMBER];
        for (c_idx, &col) in colors.iter().enumerate() {
            let y0 = -13.0 + c_idx as f32 * 6.5;
            let mut lx = -46.0;
            let mut ly = y0;
            for seg in 1..=8 {
                let nx = -46.0 + seg as f32 * 11.5;
                // The live cable (cyan) hums; the rest lie still.
                let wob = if c_idx == 1 {
                    (time * 2.0 + seg as f32).sin()
                } else {
                    0.0
                };
                let ny = y0 + ((nx * 0.15) + c_idx as f32 * 2.1).sin() * 2.5 + wob;
                line(g, lx, ly, nx, ny, 2.5, col);
                lx = nx;
                ly = ny;
            }
        }
    }

    /// 17 — spare cable coil with its loose end and connector.
    fn cable_coil(g: &Graphics, time: f32) {
        circle(g, 0.0, 0.0, 33.0, Color::new(0.48, 0.28, 0.20, 1.0));
        circle(g, 0.0, 0.0, 26.0, COPPER);
        circle(g, 0.0, 0.0, 19.0, Color::new(0.48, 0.28, 0.20, 1.0));
        circle(g, 0.0, 0.0, 12.0, Color::new(0.10, 0.09, 0.13, 1.0));
        // Wind glints slowly circling the coil.
        for k in 0..3 {
            let a = time * 0.4 + k as f32 * (TAU / 3.0);
            g.draw_arc(Vec2::new(0.0, 0.0), 29.5, a, a + 0.9, alpha(CREAM, 0.18));
        }
        line(g, 28.0, 14.0, 42.0, 26.0, 4.0, COPPER);
        rect(g, 40.0, 24.0, 8.0, 8.0, STEEL);
    }

    /// 18 — tape library: cartridge wall and the little robot that never sleeps.
    fn tape_library(g: &Graphics, time: f32) {
        rect(g, -42.0, -45.0, 84.0, 90.0, STEEL_DARK);
        frame(g, -42.0, -45.0, 84.0, 90.0, 2.0, TRIM);
        for row in 0..5u32 {
            for col in 0..4u32 {
                let x = -36.0 + col as f32 * 19.0;
                let y = -40.0 + row as f32 * 12.0;
                if rnd(row * 7 + col, 11) > 0.8 {
                    rect(g, x, y, 16.0, 9.0, PANEL); // empty slot
                } else {
                    let v = 0.2 + rnd(col, row) * 0.15;
                    rect(g, x, y, 16.0, 9.0, Color::new(v, v * 0.95, v * 1.2, 1.0));
                    rect(g, x + 2.0, y + 3.0, 12.0, 2.0, alpha(CREAM, 0.4)); // label
                }
            }
        }
        // The picker arm patrols its rail; its head lamp blinks while seeking.
        rect(g, -36.0, 24.0, 72.0, 4.0, TRIM);
        let ax = (time * 0.7).sin() * 27.0;
        rect(g, ax - 6.0, 18.0, 12.0, 16.0, CREAM);
        let seek = blink(time, 5.0, 0.0, 0.5);
        rect(
            g,
            ax - 2.0,
            20.0,
            4.0,
            4.0,
            if seek { LED_RED } else { PANEL },
        );
    }

    /// 19 — strapped supply crate, stencilled for the datacenter.
    fn supply_crate(g: &Graphics, _time: f32) {
        rect(
            g,
            -38.0,
            -32.0,
            76.0,
            64.0,
            Color::new(0.35, 0.27, 0.20, 1.0),
        );
        frame(
            g,
            -38.0,
            -32.0,
            76.0,
            64.0,
            2.5,
            Color::new(0.24, 0.18, 0.13, 1.0),
        );
        rect(
            g,
            -38.0,
            -6.0,
            76.0,
            12.0,
            Color::new(0.22, 0.17, 0.13, 1.0),
        );
        rect(
            g,
            -8.0,
            -32.0,
            16.0,
            64.0,
            Color::new(0.26, 0.20, 0.15, 1.0),
        );
        for &(x, y) in &[(-33.0, -27.0), (28.0, -27.0), (-33.0, 22.0), (28.0, 22.0)] {
            circle(g, x + 2.5, y + 2.5, 2.0, Color::new(0.15, 0.11, 0.08, 1.0));
        }
        g.draw_text(
            "CLD-01",
            Vec2::new(-22.0, 24.0),
            13.0,
            Color::new(0.9, 0.85, 0.7, 0.55),
        );
    }

    /// 20 — ceiling security camera panning its watch cone.
    fn security_cam(g: &Graphics, time: f32) {
        rect(g, -8.0, -48.0, 16.0, 10.0, STEEL_DARK); // mount plate
        line(g, 0.0, -38.0, 0.0, -22.0, 4.0, STEEL);
        g.save();
        g.translate(0.0, -18.0);
        g.rotate((time * 0.5).sin() * 0.55);
        // Watch cone first, under the body.
        g.draw_arc(
            Vec2::new(0.0, 30.0),
            34.0,
            FRAC_PI_2 - 0.35,
            FRAC_PI_2 + 0.35,
            Color::new(1.0, 0.2, 0.2, 0.08),
        );
        rect(g, -8.0, 0.0, 16.0, 30.0, STEEL);
        frame(g, -8.0, 0.0, 16.0, 30.0, 1.5, TRIM);
        circle(g, 0.0, 30.0, 6.0, PANEL);
        circle(g, 0.0, 30.0, 2.5, GLOW_CYAN);
        let recording = blink(time, 1.0, 0.0, 0.12);
        circle(g, 5.0, 4.0, 2.0, if recording { LED_RED } else { PANEL });
        g.restore();
    }

    /// 21 — fire suppression pair: agent canisters plumbed into a manifold.
    fn fire_suppressor(g: &Graphics, time: f32) {
        rect(g, -26.0, -40.0, 52.0, 7.0, STEEL); // manifold
        for k in 0..4 {
            circle(g, -19.0 + k as f32 * 13.0, -40.0, 2.0, PANEL); // nozzles
        }
        for &x in &[-18.0f32, 18.0] {
            line(g, x, -33.0, x, -15.0, 4.0, TRIM);
            circle(g, x, 4.0, 17.0, TRIM);
            circle(g, x, 4.0, 14.0, Color::new(0.72, 0.14, 0.18, 1.0));
            rect(g, x - 9.0, 0.0, 18.0, 5.0, alpha(CREAM, 0.85)); // band
            circle(g, x, -10.0, 4.5, STEEL);
            // Pressure gauges, needles steady (one twitches).
            circle(g, x, 14.0, 4.0, CREAM);
            let tw = if x > 0.0 {
                0.12 * (time * 7.0).sin()
            } else {
                0.0
            };
            let a = -1.9 + tw;
            line(
                g,
                x,
                14.0,
                x + a.cos() * 3.2,
                14.0 + a.sin() * 3.2,
                1.2,
                LED_RED,
            );
        }
    }

    /// 22 — hazard floor pad: striped border around a KEEP CLEAR zone.
    fn hazard_pad(g: &Graphics, _time: f32) {
        rect(
            g,
            -44.0,
            -44.0,
            88.0,
            88.0,
            Color::new(0.12, 0.11, 0.15, 1.0),
        );
        let black = Color::new(0.05, 0.05, 0.06, 1.0);
        for i in 0..11 {
            let o = -44.0 + i as f32 * 8.0;
            let c = if i % 2 == 0 { HAZARD_YELLOW } else { black };
            let c2 = if i % 2 == 0 { black } else { HAZARD_YELLOW };
            rect(g, o, -44.0, 8.0, 8.0, c);
            rect(g, o, 36.0, 8.0, 8.0, c2);
            rect(g, -44.0, o, 8.0, 8.0, c2);
            rect(g, 36.0, o, 8.0, 8.0, c);
        }
        // Warning triangle.
        line(g, 0.0, -18.0, 16.0, 12.0, 3.0, HAZARD_YELLOW);
        line(g, 16.0, 12.0, -16.0, 12.0, 3.0, HAZARD_YELLOW);
        line(g, -16.0, 12.0, 0.0, -18.0, 3.0, HAZARD_YELLOW);
        rect(g, -1.5, -8.0, 3.0, 10.0, HAZARD_YELLOW);
        rect(g, -1.5, 5.0, 3.0, 3.0, HAZARD_YELLOW);
    }

    /// 23 — the uplink obelisk: a humming monolith with an orbiting escort.
    fn uplink_obelisk(g: &Graphics, time: f32) {
        let breath = 0.5 + 0.5 * (time * 2.0).sin();
        circle(g, 0.0, 0.0, 44.0, alpha(GLOW_MAGENTA, 0.08 + 0.06 * breath));
        rect(
            g,
            -14.0,
            -45.0,
            28.0,
            90.0,
            Color::new(0.07, 0.05, 0.10, 1.0),
        );
        frame(
            g,
            -14.0,
            -45.0,
            28.0,
            90.0,
            1.5,
            Color::new(0.35, 0.15, 0.35, 1.0),
        );
        rect(
            g,
            -2.0,
            -40.0,
            4.0,
            80.0,
            alpha(GLOW_MAGENTA, 0.45 + 0.45 * breath),
        );
        for i in 0..5 {
            rect(
                g,
                -7.0,
                -33.0 + i as f32 * 16.0,
                14.0,
                2.0,
                alpha(GLOW_MAGENTA, 0.5),
            );
        }
        for k in 0..3 {
            let a = time * 1.3 + k as f32 * 2.1;
            let c = if k % 2 == 0 { GLOW_MAGENTA } else { GLOW_CYAN };
            circle(g, a.cos() * 30.0, a.sin() * 38.0, 2.5, alpha(c, 0.85));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_names_are_present_and_unique() {
        assert_eq!(PROP_COUNT, PROP_NAMES.len());
        for (i, a) in PROP_NAMES.iter().enumerate() {
            assert!(!a.is_empty());
            for b in PROP_NAMES.iter().skip(i + 1) {
                assert_ne!(a, b, "duplicate prop name");
            }
        }
    }
}
