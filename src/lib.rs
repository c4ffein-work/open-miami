// Core modules
pub mod math;

// WASM-only modules for browser integration
#[cfg(target_arch = "wasm32")]
pub mod audio;
#[cfg(target_arch = "wasm32")]
pub mod graphics;
#[cfg(target_arch = "wasm32")]
pub mod input;

// Library module for game logic (enables testing)
pub mod collision;
pub mod components;
pub mod drive;
pub mod ecs;
pub mod editor;
pub mod ending;
pub mod game;
pub mod levels;
#[rustfmt::skip]
pub mod levels_data;
pub mod pathfinding;
pub mod props;
#[rustfmt::skip]
pub mod props_data;
#[cfg(target_arch = "wasm32")]
pub mod render;
#[cfg(target_arch = "wasm32")]
pub mod render_comms;
#[cfg(target_arch = "wasm32")]
pub mod render_dialogue;
pub mod scenario;
pub mod sim;
pub mod systems;

// Camera and level rendering (WASM-only, depend on the canvas Graphics)
#[cfg(target_arch = "wasm32")]
pub mod camera;
#[cfg(target_arch = "wasm32")]
pub mod editor_ui;
#[cfg(target_arch = "wasm32")]
pub mod floor_props;
#[cfg(target_arch = "wasm32")]
pub mod level;

// WASM entry point - browser game initialization and main loop
#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    // Import game modules
    use crate::audio::{song_for_floor, AudioEngine, SONGS};
    use crate::camera::Camera;
    use crate::ecs::{System, World};
    use crate::ending::{self, Ending, Outro, EXTRACT_CARD_SECS};
    use crate::game::*;
    use crate::graphics::Graphics;
    use crate::input;
    use crate::level::Level;
    use crate::levels::{
        floor_def, floor_title, level_index_for_floor_id, BOSS_LEVEL, LEVEL_COUNT,
    };
    use crate::math::{Color, Vec2};
    use crate::props::{
        draw_prop_ex, family_range, largest_family, prop_family, prop_layers, prop_modes, prop_px,
        settings_json, snap_size, PixelMode, PropDrawOpts, MAX_LAYERS, MAX_PX, PROP_COUNT,
        PROP_FAMILIES, PROP_NAMES,
    };
    use crate::render::*;
    use crate::render_comms::{
        render_elevators, render_gate_prompt, render_hold_caption, render_objective,
        render_zones_debug,
    };
    use crate::render_dialogue::render_dialogue;
    use crate::scenario::{ScenarioState, SURFACE_EXIT};
    use crate::systems::boss::any_boss_enraged;
    use crate::systems::*;

    /// Index into [`SONGS`] of the calmest track (lowest intensity): what
    /// plays once the uplink is back and under the credits.
    fn calmest_song_index() -> usize {
        SONGS
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.intensity.total_cmp(&b.1.intensity))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Longest simulation step a single frame may take (seconds).
    const MAX_FRAME_DT: f32 = 0.1;
    /// Hold R this long (seconds) while alive to restart the floor.
    const RESTART_HOLD_SECS: f32 = 1.0;
    /// Safety cap for the loading screen's PRECOMPUTING step: if the audio
    /// pre-renders have not finished by then (broken OfflineAudioContext,
    /// pathologically slow machine), the game starts anyway — every sound
    /// falls back to live synthesis until its bake lands.
    const PRECOMPUTE_CAP_MS: f64 = 6000.0;

    #[wasm_bindgen]
    extern "C" {
        // ?viz inspector panel: open the right-hand iframe on a gallery item /
        // hide it again (both defined in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = vizInspect)]
        fn viz_inspect(kind: &str);
        #[wasm_bindgen(js_namespace = window, js_name = vizInspectHide)]
        fn viz_inspect_hide();
        // ?viz PROPS page SAVE: PUT the props/props.json document through
        // serve.py's editor API (token flow + result toast in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = vizSaveProps)]
        fn viz_save_props(json: &str);
        // Open an external link in a new tab (defined in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = openExternal)]
        fn open_external(url: &str);
        // The HTML loading overlay: progress during the PRECOMPUTING
        // (audio pre-render) step, and the hide call once the game may show
        // its first screen (both defined in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = loadingProgress)]
        fn loading_progress(done: u32, total: u32);
        #[wasm_bindgen(js_namespace = window, js_name = loadingDone)]
        fn loading_done();
        // Persistent settings (localStorage; defined in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = getSetting)]
        fn get_setting(name: &str) -> Option<String>;
        #[wasm_bindgen(js_namespace = window, js_name = setSetting)]
        fn set_setting(name: &str, value: &str);
        // Hide / restore the OS cursor over the canvas (defined in
        // index.html); hidden during gameplay, where the engine draws its
        // own pixel crosshair instead.
        #[wasm_bindgen(js_namespace = window, js_name = setCursorHidden)]
        fn set_cursor_hidden(hidden: bool);
    }

    /// Per-frame performance tracing (`?perf`): thin externs to the collector
    /// in index.html (`window.__perf`). Every entry point checks
    /// [`perf::enabled`] first, so a run without the flag never crosses the
    /// wasm->JS boundary (the JS side guards again, belt and braces).
    mod perf {
        use wasm_bindgen::prelude::*;

        #[wasm_bindgen]
        extern "C" {
            #[wasm_bindgen(js_namespace = window, js_name = perfSpan)]
            fn js_span(name: &str, start: f64, dur: f64);
            #[wasm_bindgen(js_namespace = window, js_name = perfFrameStart)]
            fn js_frame_start(t: f64);
            #[wasm_bindgen(js_namespace = window, js_name = perfFrameEnd)]
            fn js_frame_end(t: f64);
        }

        thread_local! {
            /// Read once from the URL on first use; wasm is single-threaded.
            static ENABLED: bool = super::url_flag("perf");
        }

        pub fn enabled() -> bool {
            ENABLED.with(|e| *e)
        }

        /// Same clock as the game loop's `performance.now()`.
        fn now() -> f64 {
            web_sys::window()
                .and_then(|w| w.performance())
                .map(|p| p.now())
                .unwrap_or(0.0)
        }

        /// Open a trace frame at the rAF timestamp the loop already has.
        /// Only called on frames that actually run (the FPS cap's skipped
        /// frames must not open frames).
        pub fn frame_start(t: f64) {
            if enabled() {
                js_frame_start(t);
            }
        }

        /// Close the trace frame (computes its own end timestamp).
        pub fn frame_end() {
            if enabled() {
                js_frame_end(now());
            }
        }

        /// An open span; dropping it reports `[name, start, dur]` to the
        /// collector — so it survives early returns. [`span`] returns `None`
        /// when tracing is off: no clock read, no boundary crossing.
        pub struct Span {
            name: &'static str,
            start: f64,
        }

        impl Drop for Span {
            fn drop(&mut self) {
                js_span(self.name, self.start, now() - self.start);
            }
        }

        pub fn span(name: &'static str) -> Option<Span> {
            enabled().then(|| Span { name, start: now() })
        }
    }

    /// The `?viz` PROPS page's editable state of one prop: its art-pixel
    /// size, which layers the preview shows and each layer's pixel mode
    /// (initialised from the saved `PROP_SETTINGS`, written back by SAVE).
    #[derive(Clone, Copy)]
    struct PropViz {
        px: u32,
        /// Bit i = layer i shown in the big preview (eye / solo).
        visible: u32,
        modes: [PixelMode; MAX_LAYERS],
    }

    /// On-screen size (px) of a robot sprite tile. The tile is square and the
    /// robot fills ~55% of it, so this is tuned so the bot roughly matches the
    /// actor hitbox (player radius 15 -> 30px dia, enemy radius 12 -> 24px
    /// dia): a 60px tile draws a ~34px robot that sits over the hitbox like
    /// the primitive did.
    const ROBOT_TILE_PX: f32 = 60.0;

    /// Kill flash: total duration and number of red/blue strobes.
    const KILL_FLASH_SECS: f32 = 0.34;
    const KILL_FLASH_STROBES: u32 = 4;

    /// The robot sprite's gun/forward points DOWN (+Y in image) at facingDeg=0,
    /// while the entity `angle` is atan2(aim) measured from +X. Rotating the
    /// image by (angle - PI/2) makes the gun point along the aim/shoot
    /// direction (where bullets actually fly), which reads correctly top-down.
    const ROBOT_ANGLE_OFFSET: f32 = -std::f32::consts::FRAC_PI_2;

    // Index tables shared with renderer.js (see Graphics::draw_robot).
    const ROBOT_COLOR_CORAL: u32 = 0;
    const ROBOT_POSE_IDLE: u32 = 0;
    const ROBOT_POSE_WALK: u32 = 1;
    const ROBOT_POSE_SHOOT: u32 = 2;
    #[allow(dead_code)]
    const ROBOT_POSE_HIT: u32 = 3;
    const ROBOT_POSE_DOWNED: u32 = 4;

    /// Map a held weapon to the robot-core weapon model index
    /// (0 fist, 1 pistol, 2 machinegun, 3 shotgun).
    fn robot_weapon_idx(weapon: Option<crate::components::WeaponType>) -> u32 {
        use crate::components::WeaponType;
        match weapon {
            None | Some(WeaponType::Melee) => 0,
            Some(WeaponType::Pistol) => 1,
            Some(WeaponType::MachineGun) => 2,
            Some(WeaponType::Shotgun) => 3,
        }
    }

    /// Downed-pose time (seconds) a body with no live knockdown clock is
    /// parked at: past the fall transition and the landing wobble, so corpses
    /// lie still, fully settled, from the first frame.
    const ROBOT_DOWNED_SETTLED: f32 = 2.0;

    /// Draw the player and rogue enemies as baked 3D sprites on top of the
    /// primitive draw. Must be called while the camera transform is applied so
    /// that world coordinates land on screen (camera zoom is 1.0). Returns once
    /// the atlas is ready; until then `draw_baked` no-ops and the primitives
    /// (already drawn by `render_entities`) remain visible.
    /// One 12x16 pixel bitmap per title glyph ('#' = filled). The neon look
    /// comes from drawing only the BOUNDARY cells of these fat letterforms:
    /// that yields the outer contour and, where a glyph has a counter (O, P,
    /// A), the inner contour — two neon lines with an empty letter between.
    fn title_glyph(ch: char) -> [&'static str; 16] {
        match ch {
            'O' => [
                ".##########.",
                "############",
                "############",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "############",
                "############",
                ".##########.",
            ],
            'P' => [
                "###########.",
                "############",
                "############",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "############",
                "############",
                "###########.",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
            ],
            'E' => [
                "###########.",
                "############",
                "############",
                "###.........",
                "###.........",
                "###.........",
                "##########..",
                "##########..",
                "##########..",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "############",
                "############",
                "###########.",
            ],
            'N' => [
                "#####....###",
                "#####....###",
                "#####....###",
                "###.##...###",
                "###.##...###",
                "###..##..###",
                "###..##..###",
                "###..##..###",
                "###...##.###",
                "###...##.###",
                "###....#####",
                "###....#####",
                "###....#####",
                "###.....####",
                "###.....####",
                "###.....####",
            ],
            'M' => [
                "#####..#####",
                "#####..#####",
                "###.####.###",
                "###.####.###",
                "###..##..###",
                "###..##..###",
                "###..##..###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
            ],
            'I' => [
                ".##########.",
                ".##########.",
                ".##########.",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                "....####....",
                ".##########.",
                ".##########.",
                ".##########.",
            ],
            'A' => [
                ".##########.",
                "############",
                "############",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "############",
                "############",
                "############",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
            ],
            // The loading screen's extra glyphs (tools/gen_title.py renders
            // "LOADING..." out of this same table).
            'L' => [
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "############",
                "############",
                "###########.",
            ],
            'D' => [
                "##########..",
                "###########.",
                "############",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "############",
                "###########.",
                "##########..",
            ],
            'G' => [
                ".##########.",
                "############",
                "############",
                "###.........",
                "###.........",
                "###.........",
                "###.........",
                "###....#####",
                "###....#####",
                "###......###",
                "###......###",
                "###......###",
                "###......###",
                "############",
                "############",
                ".##########.",
            ],
            '.' => [
                "............",
                "............",
                "............",
                "............",
                "............",
                "............",
                "............",
                "............",
                "............",
                "............",
                "............",
                "..####......",
                "..####......",
                "..####......",
                "..####......",
                "............",
            ],
            _ => ["............"; 16],
        }
    }

    /// The title: "OPEN" / "MIAMI" as huge hollow neon-pink pixel letters
    /// (outer + inner contours of the fat glyphs, with a two-ring pixel glow
    /// around them), rasterized in one pixel-art group opened UNDER a slow
    /// rotation — the whole sign sways between -20 and -3 degrees.
    fn draw_neon_title(graphics: &Graphics, cx: f32, cy: f32, t: f32) {
        const UNIT: f32 = 8.0; // one art pixel = 8 screen px
        const GW: usize = 72;
        const GH: usize = 36;
        let mut filled = [[false; GW]; GH];
        let stamp = |word: &str, x0: usize, y0: usize, filled: &mut [[bool; GW]; GH]| {
            for (i, ch) in word.chars().enumerate() {
                let glyph = title_glyph(ch);
                let gx = x0 + i * 14; // 12 wide + 2 gap
                for (r, row) in glyph.iter().enumerate() {
                    for (c, cell) in row.bytes().enumerate() {
                        if cell == b'#' {
                            filled[y0 + r][gx + c] = true;
                        }
                    }
                }
            }
        };
        stamp("OPEN", 8, 1, &mut filled);
        stamp("MIAMI", 1, 19, &mut filled);

        let at = |r: isize, c: isize| -> bool {
            r >= 0
                && c >= 0
                && (r as usize) < GH
                && (c as usize) < GW
                && filled[r as usize][c as usize]
        };
        // Boundary = a filled cell with an empty 4-neighbour; the glow rings
        // are the empty cells within 1 / 2 (8-neighbourhood) of a boundary.
        let mut layer = [[0u8; GW]; GH]; // 3 = core, 2 = glow, 1 = faint glow
        for r in 0..GH as isize {
            for c in 0..GW as isize {
                if at(r, c) && !(at(r - 1, c) && at(r + 1, c) && at(r, c - 1) && at(r, c + 1)) {
                    layer[r as usize][c as usize] = 3;
                }
            }
        }
        for pass in [2u8, 1u8] {
            let want = pass + 1;
            for r in 0..GH as isize {
                for c in 0..GW as isize {
                    if at(r, c) || layer[r as usize][c as usize] != 0 {
                        continue;
                    }
                    'scan: for dr in -1..=1 {
                        for dc in -1..=1 {
                            let (nr, nc) = (r + dr, c + dc);
                            if nr >= 0
                                && nc >= 0
                                && (nr as usize) < GH
                                && (nc as usize) < GW
                                && layer[nr as usize][nc as usize] == want
                            {
                                layer[r as usize][c as usize] = pass;
                                break 'scan;
                            }
                        }
                    }
                }
            }
        }

        // Slow sway between -12 and -3 degrees (period ~20 s).
        let ang = (-7.5 + 4.5 * (t * 0.31).sin()).to_radians();
        let (w, h) = (GW as f32 * UNIT, GH as f32 * UNIT);
        graphics.save();
        graphics.translate(cx, cy);
        graphics.rotate(ang);
        graphics.pixel_begin(UNIT, w, h);
        for (r, row) in layer.iter().enumerate() {
            for (c, &l) in row.iter().enumerate() {
                if l == 0 {
                    continue;
                }
                let color = match l {
                    3 => Color::new(1.0, 0.20, 0.60, 1.0),
                    2 => Color::new(1.0, 0.20, 0.60, 0.30),
                    _ => Color::new(1.0, 0.20, 0.60, 0.12),
                };
                graphics.draw_rectangle(
                    Vec2::new(c as f32 * UNIT, r as f32 * UNIT),
                    UNIT,
                    UNIT,
                    color,
                );
            }
        }
        graphics.pixel_end(-w / 2.0, -h / 2.0);
        graphics.restore();
    }

    /// A small pixel-art arrow pointing DOWN at `(x, y)` (its tip), built
    /// from 3-px cells: a 2-cell shaft over a 6/4/2-cell head, with a dark
    /// backing shadow so it reads on any floor.
    fn draw_pixel_arrow(graphics: &Graphics, x: f32, y: f32, accent: (u8, u8, u8)) {
        const C: f32 = 3.0;
        let col = Color::new(
            accent.0 as f32 / 255.0,
            accent.1 as f32 / 255.0,
            accent.2 as f32 / 255.0,
            0.95,
        );
        let shadow = Color::new(0.0, 0.0, 0.0, 0.45);
        // (dx cells, dy cells, w cells) rows, y grows toward the tip.
        let rows: [(f32, f32, f32); 7] = [
            (-1.0, -7.0, 2.0),
            (-1.0, -6.0, 2.0),
            (-1.0, -5.0, 2.0),
            (-1.0, -4.0, 2.0),
            (-3.0, -3.0, 6.0),
            (-2.0, -2.0, 4.0),
            (-1.0, -1.0, 2.0),
        ];
        for &(dx, dy, w) in &rows {
            graphics.draw_rectangle(
                Vec2::new(x + dx * C + 1.0, y + dy * C + 1.0),
                w * C,
                C,
                shadow,
            );
        }
        for &(dx, dy, w) in &rows {
            graphics.draw_rectangle(Vec2::new(x + dx * C, y + dy * C), w * C, C, col);
        }
    }

    /// Draw the player and rogue enemies as live-rendered 3D robot sprites on
    /// top of the primitive draw. Must be called while the camera transform is
    /// applied so that world coordinates land on screen (camera zoom is 1.0).
    /// `now` is elapsed time in seconds and drives the pose animations; each
    /// entity's clock is offset by its id so the squad doesn't move in
    /// phase-locked unison, and knocked-down bots play the hit flinch synced
    /// to the moment the stun landed.
    ///
    /// One pass of the robot sprites. `prone_pass` = draw only the downed
    /// (dead / knocked-down) bodies; `!prone_pass` = only the upright ones.
    /// Two passes let the ground weapons draw OVER the corpses (easy to spot)
    /// yet UNDER anyone still standing.
    ///
    /// Each ROBOT command costs robot-core an FBO round-trip and ~15-19 draw
    /// calls, so bots fully outside `cull` are skipped (conservative
    /// half-extent: a whole tile, downed bodies sprawl past their centre).
    fn draw_robot_entities(
        world: &World,
        graphics: &Graphics,
        now: f32,
        prone_pass: bool,
        cull: &crate::camera::ViewCull,
    ) {
        use crate::components::{AIState, EnemyType};
        use crate::components::{
            Boss, Enemy, Finisher, FinisherKind, Health, Player, Position, Rotation, Stunned,
            Velocity, Weapon, AI,
        };

        // Determines a standing pose index from motion / combat state.
        fn pose_for(speed: f32, attacking: bool) -> u32 {
            if attacking {
                ROBOT_POSE_SHOOT
            } else if speed > 6.0 {
                ROBOT_POSE_WALK
            } else {
                ROBOT_POSE_IDLE
            }
        }

        // --- Enemies (rogue bots) ---
        for entity in world.query::<Enemy>() {
            if world.has_component::<Boss>(entity) {
                continue; // boss keeps its own draw
            }
            let (pos, rot, health, ai) = match (
                world.get_component::<Position>(entity),
                world.get_component::<Rotation>(entity),
                world.get_component::<Health>(entity),
                world.get_component::<AI>(entity),
            ) {
                (Some(p), Some(r), Some(h), Some(a)) => (p, r, h, a),
                _ => continue,
            };
            let color_idx = match ai.initial_type {
                EnemyType::Idle => 1,       // SENTINEL - red
                EnemyType::Wandering => 2,  // DRIFTER - violet
                EnemyType::Patrolling => 3, // HUNTER - magenta
            };
            let stunned = world.get_component::<Stunned>(entity);
            // Dead OR knocked down: sprawled flat in the DOWNED pose.
            let prone = health.is_dead() || stunned.is_some();
            if prone != prone_pass {
                continue;
            }
            if !cull.visible(pos.x, pos.y, ROBOT_TILE_PX) {
                continue;
            }
            let speed = world
                .get_component::<Velocity>(entity)
                .map(|v| (v.x * v.x + v.y * v.y).sqrt())
                .unwrap_or(0.0);
            let attacking = ai.state == AIState::SurePlayerSeen && ai.attack_timer > 0.0;
            let pose_idx = if prone {
                ROBOT_POSE_DOWNED
            } else {
                pose_for(speed, attacking)
            };
            let weapon_idx =
                robot_weapon_idx(world.get_component::<Weapon>(entity).map(|w| w.weapon_type));
            // De-sync the squad: each bot's animation clock starts at a
            // different phase derived from its entity id.
            let phase = (entity.0 % 97) as f32 * 0.173;
            // Downed bodies face the blow's origin: the pose's backward topple
            // (robot-core's downed plan leans the rig onto its back) then lays
            // them out along `fall_angle` — head first, away from the blow.
            // Killing blows record the same convention (`record_corpse_fall`:
            // bullets, melee swings and thrown weapons all leave a `Stunned`
            // carrying the shot direction), so corpses sprawl away from the
            // shooter too. A corpse without a live knockdown clock keeps its
            // `Rotation` (the stun system wrote the matching facing there when
            // the stun ended).
            let angle = match stunned {
                Some(stun) => stun.fall_angle + std::f32::consts::PI,
                None => rot.angle,
            };
            let time = if let Some(stun) = stunned {
                // Seconds since the knockdown landed: plays the fall
                // transition once, then the body lies still.
                stun.age()
            } else if health.is_dead() {
                // Dead with no knockdown clock: parked fully settled.
                ROBOT_DOWNED_SETTLED
            } else {
                now + phase
            };
            graphics.draw_robot(
                color_idx,
                pose_idx,
                weapon_idx,
                Vec2::new(pos.x, pos.y),
                angle + ROBOT_ANGLE_OFFSET,
                ROBOT_TILE_PX,
                time,
            );
        }

        // --- Player (CL4-UD3, coral; upright pass only — dead players
        // render their WASTED state elsewhere) ---
        if prone_pass {
            return;
        }
        if let Some(&player) = world.query::<Player>().first() {
            let pos = world.get_component::<Position>(player);
            let health = world.get_component::<Health>(player);
            if let (Some(pos), Some(health)) = (pos, health) {
                // Off-screen check matters even for the player: a scenario
                // `look_at` can carry the camera clean away from them.
                if !health.is_dead() && cull.visible(pos.x, pos.y, ROBOT_TILE_PX) {
                    let mut angle = world
                        .get_component::<Rotation>(player)
                        .map(|r| r.angle)
                        .unwrap_or(0.0);
                    let speed = world
                        .get_component::<Velocity>(player)
                        .map(|v| (v.x * v.x + v.y * v.y).sqrt())
                        .unwrap_or(0.0);
                    let firing =
                        crate::input::is_mouse_button_down(crate::input::mouse_buttons::LEFT);
                    let mut pose_idx = pose_for(speed, firing);
                    let mut draw_pos = Vec2::new(pos.x, pos.y);
                    // Mid-finisher: locked over the victim in the strike pose,
                    // lunging into each blow (one surge for the bar / the
                    // point-blank shot, a pulse per pound when unarmed).
                    if let Some(fin) = world.get_component::<Finisher>(player) {
                        let progress = (fin.timer / fin.kind.duration()).clamp(0.0, 1.0);
                        let lunge = match fin.kind {
                            FinisherKind::Pound => {
                                8.0 * (progress * 3.0 * std::f32::consts::PI).sin().abs()
                            }
                            _ => 10.0 * (progress * std::f32::consts::PI).sin(),
                        };
                        pose_idx = ROBOT_POSE_SHOOT;
                        angle = fin.dir_y.atan2(fin.dir_x);
                        draw_pos = Vec2::new(pos.x + fin.dir_x * lunge, pos.y + fin.dir_y * lunge);
                    }
                    let weapon_idx = robot_weapon_idx(
                        world.get_component::<Weapon>(player).map(|w| w.weapon_type),
                    );
                    graphics.draw_robot(
                        ROBOT_COLOR_CORAL,
                        pose_idx,
                        weapon_idx,
                        draw_pos,
                        angle + ROBOT_ANGLE_OFFSET,
                        ROBOT_TILE_PX,
                        now,
                    );
                }
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GameScreen {
        LevelSelect,
        BossIntro,
        InGame,
        Paused,
        Settings,
        About,
        Visualizer,
        /// The credits roll after the last car goes up (see `ending.rs`).
        Ending,
    }

    /// The page URL's query string (e.g. "?viz"), empty if unavailable.
    fn url_query() -> String {
        web_sys::window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default()
    }

    /// Whether the asset visualizer was requested via `?viz` in the URL.
    fn wants_visualizer() -> bool {
        url_query().contains("viz")
    }

    /// The value of the query parameter `name` (`?name=value`), if present.
    fn url_param(name: &str) -> Option<String> {
        let q = url_query();
        q.trim_start_matches('?').split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    }

    /// Whether the query string carries the flag `name` (`?name`, `?name=1`,
    /// `?floor=3&name`...).
    fn url_flag(name: &str) -> bool {
        let q = url_query();
        q.trim_start_matches('?')
            .split('&')
            .any(|kv| kv.split_once('=').map(|(k, _)| k).unwrap_or(kv) == name)
    }

    /// Tabs of the `?viz` tool.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum VizTab {
        Sprites,
        Musics,
        Levels,
        Effects,
    }

    /// EFFECTS tab: the POSTFX shader menu — (kind, label, preview peak `t`,
    /// colour). Mirrors the kind table in renderer.js / `Graphics::postfx`.
    /// Peak `t` stays below 1 where full strength would blank the frame
    /// (BLUR-OUT at t = 1 is a solid colour).
    const POSTFX_PREVIEWS: [(u32, &str, f32, Color); 14] = [
        (0, "BLUR-OUT", 0.8, Color::new(0.05, 0.02, 0.10, 1.0)),
        (1, "SYNTHWAVE CRT", 1.0, Color::new(1.0, 0.25, 0.65, 1.0)),
        (2, "VHS TAPE", 1.0, Color::new(0.60, 0.60, 0.90, 1.0)),
        (3, "DRUNK SWAY", 1.0, Color::new(0.60, 0.20, 0.80, 1.0)),
        (4, "CRT TUBE", 1.0, Color::new(0.20, 0.90, 0.90, 1.0)),
        (5, "ACID TRIP", 1.0, Color::new(0.90, 0.30, 0.90, 1.0)),
        (6, "DATAMOSH", 1.0, Color::new(0.30, 0.90, 0.50, 1.0)),
        (7, "NEON BLOOM", 1.0, Color::new(0.55, 0.10, 0.60, 1.0)),
        (8, "PIXEL MOSAIC", 1.0, Color::new(0.90, 0.80, 0.30, 1.0)),
        (9, "TUNNEL RUSH", 1.0, Color::new(1.0, 0.40, 0.20, 1.0)),
        (10, "WARP TRAILS", 1.0, ending::WARP_TINT),
        (11, "UI GREY", 0.8, Color::new(0.80, 0.82, 0.90, 1.0)),
        // r/g = the demo modal's half extents (fractions of the screen).
        (12, "MODAL STATIC", 0.9, Color::new(0.25, 0.22, 0.0, 1.0)),
        (13, "TV STATIC", 0.3, Color::WHITE),
    ];

    /// How long an EFFECTS-tab POSTFX preview plays (ramp in, hold, ramp out).
    const POSTFX_PREVIEW_MS: f64 = 4000.0;

    /// Small deterministic hash -> pseudo-random, used for the glitch effect.
    fn hash2(a: u32, b: u32) -> u32 {
        let mut x = a
            .wrapping_mul(374_761_393)
            .wrapping_add(b.wrapping_mul(668_265_263));
        x = (x ^ (x >> 13)).wrapping_mul(1_274_126_177);
        x ^ (x >> 16)
    }

    fn rand01(a: u32, b: u32) -> f32 {
        (hash2(a, b) & 0xff_ffff) as f32 / 0xff_ffff as f32
    }

    /// Full-screen "shoggoth" glitch: live-cell tissue rendered as a pixelated
    /// Voronoi field. The screen is scanned in chunky blocks; each block finds
    /// its two nearest cell nuclei, and blocks nearly equidistant to both are
    /// MEMBRANE — the wall *in between* neighbouring cells — drawn as a
    /// green-or-black dithered pixel line. Cell interiors stay dark cytoplasm
    /// with a small nucleus, and a subset of cells carry a blinking pale-yellow
    /// eye. The nuclei re-seat to nearby spots every ~6 frames (~0.1s) so the
    /// whole tissue squirms glitchily. 1.2s fade envelope; `elapsed_ms` is time
    /// since the effect started.
    fn draw_shoggoth_glitch(g: &Graphics, elapsed_ms: f32) {
        let (w, h) = (g.width(), g.height());
        let t = (elapsed_ms / 1200.0).clamp(0.0, 1.0);
        let env = if t < 0.1 {
            t / 0.1
        } else if t > 0.7 {
            ((1.0 - t) / 0.3).max(0.0)
        } else {
            1.0
        };

        // Dark takeover of the screen.
        g.draw_rectangle(
            Vec2::new(0.0, 0.0),
            w,
            h,
            Color::new(0.03, 0.03, 0.045, 0.9 * env),
        );

        // One nucleus (seed) per jittered grid cell; the layout re-seats every
        // ~6 frames, wobbling only to nearby spots — the glitchy squirm.
        let tick = (elapsed_ms / 100.0) as u32;
        let cols: i32 = 12;
        let rows: i32 = 9;
        let cw = w / cols as f32;
        let ch = h / rows as f32;
        let seed = |i: i32, j: i32| -> (f32, f32) {
            // wrap so blocks near the screen edge still see a full neighbourhood
            let (iw, jw) = (i.rem_euclid(cols), j.rem_euclid(rows));
            let id = (jw * cols + iw) as u32;
            let ax = (i as f32 + 0.5) * cw + (rand01(id, 3) - 0.5) * cw * 0.6;
            let ay = (j as f32 + 0.5) * ch + (rand01(id, 4) - 0.5) * ch * 0.6;
            let jx = (rand01(id, tick * 2 + 1) - 0.5) * cw * 0.22;
            let jy = (rand01(id, tick * 2 + 2) - 0.5) * ch * 0.22;
            (ax + jx, ay + jy)
        };

        // Pixelated scan: chunky blocks classified as membrane / nucleus / bg.
        let px = 10.0f32; // block size — the pixelization
        let membrane_w = 0.16; // boundary half-width (in nearest-distance ratio)
        let bx_n = (w / px).ceil() as i32;
        let by_n = (h / px).ceil() as i32;
        for byi in 0..by_n {
            for bxi in 0..bx_n {
                let cx = (bxi as f32 + 0.5) * px;
                let cy = (byi as f32 + 0.5) * px;
                let gi = (cx / cw).floor() as i32;
                let gj = (cy / ch).floor() as i32;
                // nearest + second-nearest nucleus over the 3x3 neighbourhood
                let (mut d1, mut d2) = (f32::MAX, f32::MAX);
                let mut best = (0i32, 0i32);
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let (sx, sy) = seed(gi + di, gj + dj);
                        let d = (sx - cx) * (sx - cx) + (sy - cy) * (sy - cy);
                        if d < d1 {
                            d2 = d1;
                            d1 = d;
                            best = (gi + di, gj + dj);
                        } else if d < d2 {
                            d2 = d;
                        }
                    }
                }
                let (d1, d2) = (d1.sqrt(), d2.sqrt());
                // Membrane: this block sits on the wall BETWEEN two cells.
                if d2 - d1 < membrane_w * (d1 + d2) {
                    // green-or-black dither, re-rolled with the glitch tick
                    let roll = rand01((bxi * 977 + byi) as u32, tick + 41);
                    let c = if roll > 0.45 {
                        Color::new(0.12, 0.55, 0.30, 0.95 * env) // membrane green
                    } else {
                        Color::new(0.01, 0.05, 0.03, 0.95 * env) // membrane black
                    };
                    g.draw_rectangle(Vec2::new(bxi as f32 * px, byi as f32 * px), px, px, c);
                } else if d1 < cw.min(ch) * 0.16 {
                    // Nucleus kernel at the middle of each cell.
                    let id = (best.1.rem_euclid(rows) * cols + best.0.rem_euclid(cols)) as u32;
                    let eyed = rand01(id, tick / 3 + 9) > 0.7;
                    let c = if eyed {
                        let blink = 0.6 + 0.4 * rand01(id, tick + 1);
                        Color::new(1.0, 0.93, 0.5, blink * env) // pale-yellow eye
                    } else {
                        Color::new(0.38, 0.15, 0.38, 0.9 * env) // plain nucleus
                    };
                    g.draw_rectangle(Vec2::new(bxi as f32 * px, byi as f32 * px), px, px, c);
                }
                // everything else stays dark cytoplasm (the takeover wash).
            }
        }
    }

    /// Draw a clickable button; returns true if the mouse is currently over it
    /// (the caller decides what a click does). `active` highlights it.
    fn viz_button(
        g: &Graphics,
        mouse: Vec2,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        active: bool,
    ) -> bool {
        let over = mouse.x >= x && mouse.x <= x + w && mouse.y >= y && mouse.y <= y + h;
        let bg = if active {
            Color::new(1.0, 0.09, 0.26, 0.85)
        } else if over {
            Color::new(0.28, 0.22, 0.33, 1.0)
        } else {
            Color::new(0.14, 0.10, 0.18, 1.0)
        };
        g.draw_rectangle(Vec2::new(x, y), w, h, bg);
        g.draw_rectangle_lines(Vec2::new(x, y), w, h, 1.5, Color::new(0.45, 0.35, 0.5, 1.0));
        g.draw_text(
            label,
            Vec2::new(x + 14.0, y + h / 2.0 + 6.0),
            18.0,
            Color::WHITE,
        );
        over
    }

    /// A compact `viz_button` (13 px label, roughly centred) for dense rows.
    #[allow(clippy::too_many_arguments)]
    fn viz_small_button(
        g: &Graphics,
        mouse: Vec2,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        active: bool,
    ) -> bool {
        let over = mouse.x >= x && mouse.x <= x + w && mouse.y >= y && mouse.y <= y + h;
        let bg = if active {
            Color::new(1.0, 0.09, 0.26, 0.85)
        } else if over {
            Color::new(0.28, 0.22, 0.33, 1.0)
        } else {
            Color::new(0.14, 0.10, 0.18, 1.0)
        };
        g.draw_rectangle(Vec2::new(x, y), w, h, bg);
        g.draw_rectangle_lines(Vec2::new(x, y), w, h, 1.0, Color::new(0.45, 0.35, 0.5, 1.0));
        let tw = label.chars().count() as f32 * 6.0;
        g.draw_text(
            label,
            Vec2::new(x + (w - tw).max(0.0) / 2.0, y + h / 2.0 + 5.0),
            13.0,
            Color::WHITE,
        );
        over
    }

    /// A mid-floor `checkpoint` snapshot: the full world (entities,
    /// components, walls, RNG) plus the scenario state (fired steps, opened
    /// exits, comms, objective) at the moment the action ran. Restored on
    /// death instead of a full floor restart.
    struct Checkpoint {
        world: World,
        scenario: ScenarioState,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MenuOption {
        Play,
        Settings,
        About,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PauseOption {
        Continue,
        Settings,
        Stop,
    }

    struct GameState {
        screen: GameScreen,
        selected_level: usize,
        selected_menu_option: MenuOption,
        selected_pause_option: PauseOption,
        world: World,
        movement_system: MovementSystem,
        weapon_system: WeaponUpdateSystem,
        ai_system: AISystem,
        combat_system: CombatSystem,
        bullet_system: BulletSystem,
        projectile_system: ProjectileTrailSystem,
        pickup_system: PickupSystem,
        thrown_system: ThrownWeaponSystem,
        finisher_system: FinisherSystem,
        stun_system: StunSystem,
        boss_system: BossSystem,
        elevator_system: ElevatorSystem,
        /// The running floor scenario (steps, comms feed, objective).
        scenario: Option<ScenarioState>,
        /// The latest mid-floor `checkpoint` snapshot: death restores it
        /// instead of restarting the floor. Cleared on every floor load.
        checkpoint: Option<Checkpoint>,
        /// Set once the player has extracted: the destination floor id
        /// (`SURFACE_EXIT` = surface). The completion card plays, then the
        /// floor loads.
        extracting: Option<usize>,
        /// Seconds R has been held while alive: at [`RESTART_HOLD_SECS`] the
        /// floor restarts from scratch (a load bar fills at screen centre).
        restart_hold: f32,
        /// Whether the OS cursor is currently hidden over the canvas (the
        /// in-game pixel crosshair replaces it; menus keep the OS cursor).
        cursor_hidden: bool,
        /// Whether the music is stopped because a tutorial gate froze the
        /// world (restarted when the gate releases).
        music_frozen: bool,
        /// The pause menu's stacked SETTINGS modal is open (Esc pops one
        /// layer: settings -> pause -> game).
        pause_in_settings: bool,
        /// The PRECOMPUTING step: the HTML loading overlay stays up while
        /// the audio pre-render queue burns down at full budget, BEFORE the
        /// first screen shows (works for any entry, `?floor=N` included).
        /// Ends on completion or at the [`PRECOMPUTE_CAP_MS`] safety cap.
        precomputing: bool,
        precompute_started: f64,
        /// FPS counter (`?debug` only): frames counted since `fps_window`
        /// started, the window's start time (ms), and the last readout.
        fps_frames: u32,
        fps_window: f64,
        fps_value: f32,
        /// The SETTINGS frame-rate cap (30 / 60 / 120; 0 = uncapped).
        /// rAF can never EXCEED the display refresh — the cap only skips
        /// frames when the display is faster than the cap.
        fps_cap: u32,
        /// When the last non-skipped frame ran (ms), for the cap.
        last_frame_ms: f64,
        /// Which SETTINGS row is highlighted (0 = SOUND, 1 = FPS CAP).
        settings_row: usize,
        level: Level,
        camera: Camera,
        last_time: f64,
        /// Last time (ms) the canvas backing size was checked against the
        /// window (see `Graphics::sync_size`); polled about once a second.
        last_size_check: f64,
        death_time: f32,
        level_complete_time: f32,
        /// Debug tooling (I overlays, K purge, B crack): only with `?debug`.
        debug_enabled: bool,
        show_infos: bool,
        // Audio + the previous-frame state used to fire one-shot sound effects.
        audio: AudioEngine,
        /// The AudioContext has been resumed after a user gesture.
        audio_unlocked: bool,
        /// The post-extraction epilogue on the last floor (uplink comms, then
        /// the blur-out), `None` otherwise.
        outro: Option<Outro>,
        /// The credits screen clock.
        ending: Ending,
        boss_intro_line: usize,
        viz_tab: VizTab,
        /// Index of the sprites-gallery item open in the inspector (-1 = none).
        viz_selected: i32,
        /// SPRITES tab sub-page: false = characters, true = the prop library.
        viz_props_page: bool,
        /// Selected prop in the PROPS gallery (big live preview on the right).
        viz_prop_selected: usize,
        /// PROPS gallery page: the prop FAMILY shown in the tile grid (an
        /// index into `PROP_FAMILIES`: DATACENTER / OUTDOOR / LOBBY).
        viz_prop_family: usize,
        /// PROPS gallery: per-prop pixel size / layer visibility / layer
        /// pixel modes (one entry per prop, see [`PropViz`]).
        viz_props: Vec<PropViz>,
        /// PROPS gallery "GRID": overlay the art-pixel grid on the preview.
        viz_pixel_grid: bool,
        /// EXPERIMENT `?pixel=N`: rasterize the in-game WORLD layer (floor,
        /// walls, entities, robots, boss — not the HUD) in a pixel group of
        /// N-px art pixels. 0 = off (the default).
        pixel_world: u32,
        /// LEVELS tab: the native level editor (`editor_ui.rs`).
        editor: crate::editor_ui::Editor,
        /// EFFECTS tab: the running preview — -1 = the 2D shoggoth glitch,
        /// >= 0 = an index into [`POSTFX_PREVIEWS`]. Timed from `effect_start`.
        effect_kind: i32,
        effect_start: f64,
        prev_player_alive: bool,
        /// Seconds until the machine-gun burst SFX may retrigger (see the
        /// event dispatch in `update_game`).
        mg_sfx_cooldown: f32,
        prev_enemies_alive: usize,
        /// Seconds left on the kill flash (background strobes red/blue).
        kill_flash: f32,
        prev_level_complete: bool,
        prev_boss_enraged: bool,
        prev_all_dead: bool,
    }

    impl GameState {
        fn new() -> Self {
            let screen = if wants_visualizer() {
                GameScreen::Visualizer
            } else {
                GameScreen::LevelSelect
            };
            let mut state = GameState {
                screen,
                selected_level: 0,
                selected_menu_option: MenuOption::Play,
                selected_pause_option: PauseOption::Continue,
                world: World::new(),
                movement_system: MovementSystem,
                weapon_system: WeaponUpdateSystem,
                ai_system: AISystem::default(),
                combat_system: CombatSystem,
                bullet_system: BulletSystem,
                projectile_system: ProjectileTrailSystem,
                pickup_system: PickupSystem,
                thrown_system: ThrownWeaponSystem,
                finisher_system: FinisherSystem,
                stun_system: StunSystem,
                boss_system: BossSystem,
                elevator_system: ElevatorSystem,
                scenario: None,
                checkpoint: None,
                extracting: None,
                restart_hold: 0.0,
                cursor_hidden: false,
                music_frozen: false,
                pause_in_settings: false,
                precomputing: true,
                precompute_started: 0.0,
                fps_frames: 0,
                fps_window: 0.0,
                fps_value: 0.0,
                fps_cap: get_setting("fps_cap")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(120),
                last_frame_ms: 0.0,
                settings_row: 0,
                level: Level::new(),
                camera: Camera::new(),
                last_time: 0.0,
                last_size_check: 0.0,
                death_time: 0.0,
                level_complete_time: 0.0,
                debug_enabled: url_flag("debug"),
                show_infos: false,
                audio: {
                    let audio = AudioEngine::new();
                    // The SETTINGS sound toggle persists in localStorage.
                    if get_setting("sound").as_deref() == Some("off") {
                        audio.set_enabled(false);
                    }
                    audio
                },
                audio_unlocked: false,
                outro: None,
                ending: Ending::new(),
                boss_intro_line: 0,
                viz_tab: VizTab::Sprites,
                viz_selected: -1,
                viz_props_page: false,
                viz_prop_selected: 0,
                viz_prop_family: 0,
                viz_props: (0..PROP_COUNT)
                    .map(|k| PropViz {
                        px: prop_px(k),
                        visible: u32::MAX,
                        modes: prop_modes(k),
                    })
                    .collect(),
                viz_pixel_grid: false,
                pixel_world: url_param("pixel")
                    .and_then(|v| v.parse::<u32>().ok())
                    .filter(|&n| n >= 2)
                    .unwrap_or(0),
                editor: crate::editor_ui::Editor::new(),
                effect_kind: -1,
                effect_start: 0.0,
                prev_player_alive: true,
                mg_sfx_cooldown: 0.0,
                prev_enemies_alive: 0,
                kill_flash: 0.0,
                prev_level_complete: false,
                prev_boss_enraged: false,
                prev_all_dead: false,
            };
            // `?floor=N`: jump straight into that floor (editor "play" button,
            // testing). Audio stays off until the first user gesture.
            if !wants_visualizer() {
                if url_flag("ending") {
                    // `?ending`: jump straight to the credits (dev shortcut,
                    // same spirit as `?floor=N`; the DRIVE scene lives there).
                    state.screen = GameScreen::Ending;
                } else if let Some(level) = Self::url_start_floor() {
                    state.selected_level = level;
                    state.start_game();
                }
            }
            state
        }

        /// `?floor=N` in the URL (the floor id: 0 = the ground-level cold
        /// open, 1..13, 14 = 13½): the level index to start on directly, if
        /// present and valid.
        fn url_start_floor() -> Option<usize> {
            let q = url_query();
            let q = q.trim_start_matches('?');
            q.split('&').find_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                if k != "floor" {
                    return None;
                }
                let n: usize = v.parse().ok()?;
                level_index_for_floor_id(n)
            })
        }

        /// (Re)build the world for `selected_level` and start its scenario.
        fn load_floor(&mut self) {
            self.world.clear();
            initialize_game(&mut self.world, self.selected_level);
            self.scenario = Some(ScenarioState::new(floor_def(self.selected_level)));
            self.checkpoint = None;
            self.level
                .set_surface(floor_def(self.selected_level).surface);
            self.reset_run_state();
        }

        /// Restore the latest mid-floor `checkpoint` (same floor): the world
        /// and scenario come back exactly as snapshotted. Returns whether a
        /// checkpoint existed.
        fn restore_checkpoint(&mut self) -> bool {
            let Some(cp) = &self.checkpoint else {
                return false;
            };
            self.world = cp.world.clone();
            self.scenario = Some(cp.scenario.clone());
            self.reset_run_state();
            true
        }

        /// Shared tail of `load_floor` / `restore_checkpoint`: camera, run
        /// flags, and the previous-frame sound-effect trackers (seeded from
        /// the fresh world so the first frame fires no spurious sounds).
        fn reset_run_state(&mut self) {
            self.camera.set_cinematic(None);
            self.extracting = None;
            self.outro = None;
            self.death_time = 0.0;
            self.level_complete_time = 0.0;
            self.kill_flash = 0.0;
            self.prev_player_alive = is_player_alive(&self.world);
            self.mg_sfx_cooldown = 0.0;
            self.prev_enemies_alive = count_alive_enemies(&self.world);
            self.prev_level_complete = false;
            self.prev_boss_enraged = any_boss_enraged(&self.world);
            self.prev_all_dead = self.prev_enemies_alive == 0;
        }

        fn start_game(&mut self) {
            self.load_floor();

            // Music (re)starts with every floor, on that floor's song. The
            // Enter keypress that got us here is a user gesture, so audio may
            // start; a `?floor=N` session has had none yet — `update` resumes
            // the context on the first in-game key/click instead.
            self.audio.resume();
            // Songs escalate by depth: keyed on the floor id (floor 0 and 1
            // share the calm opener) so adding the ground floor did not shift
            // every floor's track.
            self.audio.set_song(song_for_floor(
                floor_def(self.selected_level).id.saturating_sub(1),
            ));
            self.audio.start_music();

            // The hidden floor opens with a face-off before the fight.
            if self.selected_level == BOSS_LEVEL {
                self.boss_intro_line = 0;
                self.screen = GameScreen::BossIntro;
            } else {
                self.screen = GameScreen::InGame;
            }
        }

        fn update(&mut self, graphics: &Graphics, current_time: f64) {
            let dt = if self.last_time == 0.0 {
                0.016 // Initial frame assume 60fps
            } else {
                // Clamp long frames (first-frame atlas baking, tab switches,
                // headless renderers) so actors cannot tunnel through walls
                // or teleport across the floor in a single step.
                (((current_time - self.last_time) / 1000.0) as f32).min(MAX_FRAME_DT)
            };
            self.last_time = current_time;

            // PRECOMPUTING: while the HTML loading overlay is still up, burn
            // the audio pre-render queue down at full budget and report
            // progress — the first screen only shows once every voice is
            // baked (or the safety cap fires). No screen runs, no input is
            // consumed; works identically for `?floor=N` starts.
            if self.precomputing {
                if self.precompute_started == 0.0 {
                    self.precompute_started = current_time;
                }
                self.audio.set_pump_budget(8);
                self.audio.update(current_time / 1000.0);
                let (done, total) = self.audio.bake_progress();
                loading_progress(done, total);
                if self.audio.bake_complete()
                    || current_time - self.precompute_started >= PRECOMPUTE_CAP_MS
                {
                    self.precomputing = false;
                    loading_done();
                }
                input::end_frame();
                return;
            }

            // Follow window resizes, browser zoom and DPR changes (reading
            // layout sizes can force style work, so poll ~1/s, not per frame).
            if current_time - self.last_size_check >= 1000.0 {
                self.last_size_check = current_time;
                graphics.sync_size();
            }

            // Clear background
            graphics.clear(Color::new(20.0 / 255.0, 12.0 / 255.0, 28.0 / 255.0, 1.0));

            // Browsers keep the AudioContext suspended until a user gesture:
            // unlock it on the first key/click anywhere (matters for `?floor=N`
            // sessions, which start in-game without the menu's Enter).
            if !self.audio_unlocked && input::any_pressed() {
                self.audio.resume();
                self.audio_unlocked = true;
            }

            // In-game the OS cursor is hidden and the engine draws its own
            // pixel crosshair (update_game); every other screen (menus, the
            // ?viz toolbox, the editor) keeps the native pointer.
            let want_hidden = self.screen == GameScreen::InGame;
            if want_hidden != self.cursor_hidden {
                set_cursor_hidden(want_hidden);
                self.cursor_hidden = want_hidden;
            }

            match self.screen {
                GameScreen::LevelSelect => {
                    self.update_level_select(graphics);
                }
                GameScreen::BossIntro => {
                    self.update_boss_intro(graphics);
                }
                GameScreen::InGame => {
                    self.update_game(graphics, dt);
                }
                GameScreen::Paused => {
                    self.update_paused(graphics);
                }
                GameScreen::Settings => {
                    self.update_settings(graphics);
                }
                GameScreen::About => {
                    self.update_about(graphics);
                }
                GameScreen::Visualizer => {
                    self.update_visualizer(graphics);
                }
                GameScreen::Ending => {
                    self.update_ending(graphics, dt);
                }
            }

            // FPS counter (`?debug` only): real rAF frames over a rolling
            // half-second window, drawn on every screen, top-right.
            if self.debug_enabled {
                self.fps_frames += 1;
                let elapsed = current_time - self.fps_window;
                if elapsed >= 500.0 {
                    self.fps_value = self.fps_frames as f32 * 1000.0 / elapsed as f32;
                    self.fps_frames = 0;
                    self.fps_window = current_time;
                }
                if self.fps_value > 0.0 {
                    let text = format!("{:.0} FPS", self.fps_value);
                    graphics.draw_text(
                        &text,
                        Vec2::new(graphics.width() - 90.0, graphics.height() - 28.0),
                        18.0,
                        Color::new(0.3, 1.0, 0.5, 0.9),
                    );
                }
            }

            // Keep the music scheduler fed regardless of screen. (`music`
            // span: note scheduling / node creation for the tracker.) The
            // voice pre-render queue burns down fast on non-gameplay screens
            // (a bake-kick hitch is invisible there) and gently in-game.
            self.audio
                .set_pump_budget(if self.screen == GameScreen::InGame {
                    1
                } else {
                    6
                });
            let music_span = perf::span("music");
            self.audio.update(current_time / 1000.0);
            drop(music_span);

            // Hand the completed frame to the JS WebGL renderer. The `flush`
            // span measures the whole JS renderer synchronously (frameRender
            // runs inside it); renderer sub-spans nest inside it on the
            // timeline.
            let flush_span = perf::span("flush");
            graphics.flush();
            drop(flush_span);

            // Update input state for next frame
            input::end_frame();
        }

        /// Asset visualizer (`?viz`): a small tabbed inspector — sprites, sounds,
        /// and level maps — for looking at the game's pieces in isolation.
        fn update_visualizer(&mut self, graphics: &Graphics) {
            let mouse = input::mouse_position();
            let click = input::is_mouse_button_pressed(input::mouse_buttons::LEFT);

            // The LEVELS tab is the native level editor; it draws first (its
            // map may overflow anywhere) and the tab bar goes on top.
            if self.viz_tab == VizTab::Levels {
                self.editor.update(graphics, mouse, click, self.last_time);
            }

            // Top tab bar.
            let tabs = [
                (VizTab::Sprites, "SPRITES"),
                (VizTab::Musics, "MUSICS"),
                (VizTab::Levels, "LEVELS"),
                (VizTab::Effects, "EFFECTS"),
            ];
            for (i, &(tab, name)) in tabs.iter().enumerate() {
                let x = 20.0 + i as f32 * 168.0;
                let over = viz_button(
                    graphics,
                    mouse,
                    x,
                    14.0,
                    158.0,
                    46.0,
                    name,
                    self.viz_tab == tab,
                );
                if over && click && self.viz_tab != tab {
                    self.viz_tab = tab;
                    // A click is a user gesture -> unlock audio.
                    self.audio.resume();
                    // Switching tabs closes the iframe panel (the sprites
                    // gallery re-opens it when an item is clicked; the LEVELS
                    // editor's SCENARIO (web) button opens the web editor).
                    viz_inspect_hide();
                    self.viz_selected = -1;
                    self.editor.hide_web();
                }
            }

            match self.viz_tab {
                VizTab::Sprites => self.draw_viz_sprites(graphics, mouse, click),
                VizTab::Musics => self.draw_viz_musics(graphics, mouse, click),
                VizTab::Levels => {} // drawn above, under the tab bar
                VizTab::Effects => self.draw_viz_effects(graphics, mouse, click),
            }

            // A previewing effect draws full-screen, on top of everything: the
            // 2D shoggoth glitch as commands, a POSTFX kind as a real post pass
            // over this whole viz frame.
            let elapsed = self.last_time - self.effect_start;
            if self.effect_start > 0.0 {
                if self.effect_kind < 0 {
                    if (0.0..1200.0).contains(&elapsed) {
                        draw_shoggoth_glitch(graphics, elapsed as f32);
                    }
                } else if (0.0..POSTFX_PREVIEW_MS).contains(&elapsed) {
                    let (kind, _, peak, color) = POSTFX_PREVIEWS[self.effect_kind as usize];
                    let p = (elapsed / POSTFX_PREVIEW_MS) as f32;
                    // Envelope: ramp in over 15%, hold, ramp out the last 20%.
                    let env = (p / 0.15).min((1.0 - p) / 0.2).clamp(0.0, 1.0);
                    graphics.postfx(kind, peak * env, color);
                }
            }
        }

        /// EFFECTS tab: trigger a full-screen effect to preview it. The POSTFX
        /// rows are the WebGL post shaders (played over this very pane for 4s,
        /// ramp in / hold / ramp out); below them, the 2D command-stream
        /// effects (1.2s).
        fn draw_viz_effects(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            let coral = Color::from_rgba(217, 119, 87, 255);
            let elapsed = self.last_time - self.effect_start;
            graphics.draw_text(
                "Full-screen effects. Click one to preview it over this pane.",
                Vec2::new(40.0, 96.0),
                18.0,
                Color::GRAY,
            );

            graphics.draw_text(
                "POST SHADERS (WebGL, POSTFX opcode)",
                Vec2::new(40.0, 126.0),
                16.0,
                coral,
            );
            for (i, &(_, name, _, _)) in POSTFX_PREVIEWS.iter().enumerate() {
                let x = 40.0 + (i % 4) as f32 * 178.0;
                let y = 138.0 + (i / 4) as f32 * 52.0;
                let active = self.effect_kind == i as i32
                    && self.effect_start > 0.0
                    && (0.0..POSTFX_PREVIEW_MS).contains(&elapsed);
                if viz_button(graphics, mouse, x, y, 168.0, 46.0, name, active) && click {
                    self.effect_kind = i as i32;
                    self.effect_start = self.last_time;
                }
            }

            let y2 = 138.0 + 3.0 * 52.0 + 18.0;
            graphics.draw_text(
                "COMMAND-STREAM EFFECTS (2D)",
                Vec2::new(40.0, y2),
                16.0,
                coral,
            );
            let active =
                self.effect_kind < 0 && self.effect_start > 0.0 && (0.0..1200.0).contains(&elapsed);
            if viz_button(
                graphics,
                mouse,
                40.0,
                y2 + 12.0,
                240.0,
                46.0,
                "Shoggoth glitch",
                active,
            ) && click
            {
                self.effect_kind = -1;
                self.effect_start = self.last_time;
            }

            // DRIVE — the glitchy synthwave ride home that plays under the
            // credits (src/drive.rs), previewed live at moderate glitch.
            let (dx0, dy0, dw, dh) = (320.0, y2 + 12.0, 600.0, 300.0);
            graphics.draw_text(
                "DRIVE (the ending scene, live, glitch 0.5)",
                Vec2::new(dx0, y2),
                16.0,
                coral,
            );
            graphics.draw_rectangle(
                Vec2::new(dx0, dy0),
                dw,
                dh,
                Color::new(0.02, 0.01, 0.04, 1.0),
            );
            graphics.save();
            graphics.translate(dx0, dy0);
            crate::drive::render_drive(
                graphics,
                dw,
                dh,
                (self.last_time / 1000.0) as f32,
                0.5,
                0.0,
            );
            graphics.restore();
            graphics.draw_rectangle_lines(Vec2::new(dx0, dy0), dw, dh, 2.0, coral);
        }

        /// SPRITES tab: two sub-pages — the character gallery (each item opens
        /// the 3D inspector iframe) and the datacenter prop library (an
        /// all-wasm gallery of animated primitive-drawn set dressing).
        fn draw_viz_sprites(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            let pages = [(false, "CHARACTERS"), (true, "PROPS")];
            for (i, &(page, name)) in pages.iter().enumerate() {
                let x = 40.0 + i as f32 * 168.0;
                let over = viz_button(
                    graphics,
                    mouse,
                    x,
                    76.0,
                    158.0,
                    38.0,
                    name,
                    self.viz_props_page == page,
                );
                if over && click && self.viz_props_page != page {
                    self.viz_props_page = page;
                    // The prop gallery draws its own big preview pane; the
                    // iframe inspector belongs to the character page only.
                    viz_inspect_hide();
                    self.viz_selected = -1;
                }
            }
            if self.viz_props_page {
                self.draw_viz_props(graphics, mouse, click);
            } else {
                self.draw_viz_characters(graphics, mouse, click);
            }
        }

        /// The PROPS page of the SPRITES tab: the prop library
        /// (`crate::props`) as a live-animated grid — one page per FAMILY
        /// (DATACENTER / OUTDOOR / LOBBY, the buttons at the right of the
        /// header) — with the selected prop enlarged on the right — its
        /// layers listed underneath (eye = hide, S = solo, BEFORE/AFTER = the
        /// layer's pixel mode) — plus the per-prop PIXEL size, the GRID
        /// overlay and SAVE (writes `props/props.json`; then `make
        /// gen-props`).
        fn draw_viz_props(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            let time = (self.last_time / 1000.0) as f32;
            let (w, h) = (graphics.width(), graphics.height());
            let mut sel = self.viz_prop_selected.min(PROP_COUNT - 1);
            let mut fam = self.viz_prop_family.min(PROP_FAMILIES.len() - 1);
            graphics.draw_text(
                "PROP LIBRARY — primitive-drawn, animated, layered; one page per family. Click \
                 a tile to enlarge. SAVE writes props/props.json, then run `make gen-props`.",
                Vec2::new(40.0, 138.0),
                16.0,
                Color::GRAY,
            );

            // Header row, right of the page buttons: the SELECTED prop's
            // art-pixel size (design units of its 100x100 box; 1 = off), the
            // GRID overlay toggle for the preview, SAVE.
            let cx = 40.0 + 2.0 * 168.0 + 40.0;
            graphics.draw_text("PIXEL", Vec2::new(cx, 76.0 + 19.0 + 6.0), 18.0, Color::GRAY);
            if viz_button(graphics, mouse, cx + 60.0, 76.0, 38.0, 38.0, "-", false) && click {
                let p = &mut self.viz_props[sel];
                p.px = p.px.saturating_sub(1).max(1);
            }
            let px_label = if self.viz_props[sel].px <= 1 {
                "OFF".to_string()
            } else {
                format!("{}", self.viz_props[sel].px)
            };
            graphics.draw_text(
                &px_label,
                Vec2::new(cx + 108.0, 76.0 + 19.0 + 6.0),
                18.0,
                Color::WHITE,
            );
            if viz_button(graphics, mouse, cx + 148.0, 76.0, 38.0, 38.0, "+", false) && click {
                let p = &mut self.viz_props[sel];
                p.px = (p.px + 1).min(MAX_PX);
            }
            if viz_button(
                graphics,
                mouse,
                cx + 204.0,
                76.0,
                84.0,
                38.0,
                "GRID",
                self.viz_pixel_grid,
            ) && click
            {
                self.viz_pixel_grid = !self.viz_pixel_grid;
            }
            if viz_button(graphics, mouse, cx + 304.0, 76.0, 84.0, 38.0, "SAVE", false) && click {
                let entries: Vec<(u32, [PixelMode; MAX_LAYERS])> =
                    self.viz_props.iter().map(|p| (p.px, p.modes)).collect();
                viz_save_props(&settings_json(&entries));
            }
            // The family pages (the tile grid shows one family at a time;
            // switching pages selects that family's first prop).
            for (f, &(name, first)) in PROP_FAMILIES.iter().enumerate() {
                let fx = cx + 414.0 + f as f32 * 120.0;
                if viz_button(graphics, mouse, fx, 76.0, 112.0, 38.0, name, fam == f)
                    && click
                    && fam != f
                {
                    self.viz_prop_family = f;
                    self.viz_prop_selected = first;
                    fam = f;
                    sel = first;
                }
            }

            let cols = 4usize;
            let rows = largest_family().div_ceil(cols);
            let (x0, y0) = (40.0f32, 152.0f32);
            let tile_w = 150.0f32;
            let tile_h = ((h - y0 - 16.0) / rows as f32).clamp(64.0, 110.0);
            for (slot, i) in family_range(fam).enumerate() {
                let name = PROP_NAMES[i];
                let bx = x0 + (slot % cols) as f32 * tile_w;
                let by = y0 + (slot / cols) as f32 * tile_h;
                let (bw, bh) = (tile_w - 6.0, tile_h - 6.0);
                let over =
                    mouse.x >= bx && mouse.x <= bx + bw && mouse.y >= by && mouse.y <= by + bh;
                let selected = sel == i;
                let bg = if selected {
                    Color::new(1.0, 0.09, 0.26, 0.30)
                } else if over {
                    Color::new(0.28, 0.22, 0.33, 1.0)
                } else {
                    Color::new(0.13, 0.09, 0.17, 1.0)
                };
                let border = if selected {
                    Color::new(1.0, 0.09, 0.26, 1.0)
                } else {
                    Color::new(0.4, 0.3, 0.45, 1.0)
                };
                graphics.draw_rectangle(Vec2::new(bx, by), bw, bh, bg);
                graphics.draw_rectangle_lines(Vec2::new(bx, by), bw, bh, 1.5, border);
                // Every tile at its own prop's saved / edited pixel size and
                // layer modes (all layers: hide / solo are preview-only).
                let pv = self.viz_props[i];
                let opts = PropDrawOpts {
                    visible: u32::MAX,
                    modes: pv.modes,
                };
                draw_prop_ex(
                    graphics,
                    i,
                    Vec2::new(bx + bw / 2.0, by + bh / 2.0 - 6.0),
                    snap_size(bh - 30.0, pv.px),
                    time,
                    pv.px,
                    &opts,
                );
                graphics.draw_text(
                    name,
                    Vec2::new(bx + 6.0, by + bh - 5.0),
                    12.0,
                    if selected { Color::WHITE } else { Color::GRAY },
                );
                if over && click {
                    self.viz_prop_selected = i;
                }
            }

            // Big live preview of the selected prop, in place of the iframe,
            // with its LAYERS list along the bottom of the panel.
            let px = x0 + cols as f32 * tile_w + 20.0;
            let pw = (w - px - 40.0).max(140.0);
            let ph = rows as f32 * tile_h - 6.0;
            graphics.draw_rectangle(Vec2::new(px, y0), pw, ph, Color::new(0.07, 0.05, 0.10, 1.0));
            graphics.draw_rectangle_lines(
                Vec2::new(px, y0),
                pw,
                ph,
                1.5,
                Color::new(0.4, 0.3, 0.45, 1.0),
            );
            let layers = prop_layers(sel);
            let row_h = 24.0;
            let list_h = 30.0 + layers.len() as f32 * row_h + 8.0;
            let area_top = y0 + 56.0;
            let area_h = (ph - 56.0 - list_h).max(60.0);
            let pv = self.viz_props[sel];
            // Integer texel -> device pixel magnification (see `snap_size`).
            let size = snap_size((pw.min(area_h) * 0.8).max(40.0), pv.px);
            let center = Vec2::new(px + pw / 2.0, area_top + area_h / 2.0);
            let opts = PropDrawOpts {
                visible: pv.visible,
                modes: pv.modes,
            };
            draw_prop_ex(graphics, sel, center, size, time, pv.px, &opts);
            // The art-pixel grid of the prop's own frame (the grid every
            // fixed layer sits on), anchored to the prop centre; readable
            // from 3 screen px per art pixel up.
            let cell = pv.px as f32 * size / 100.0;
            if self.viz_pixel_grid && pv.px >= 2 && cell >= 3.0 {
                let gc = Color::new(1.0, 1.0, 1.0, 0.16);
                let half = size * 0.55;
                let n = (half / cell).ceil() as i32;
                for k in -n..=n {
                    let o = k as f32 * cell;
                    graphics.draw_line(
                        Vec2::new(center.x + o, center.y - half),
                        Vec2::new(center.x + o, center.y + half),
                        1.0,
                        gc,
                    );
                    graphics.draw_line(
                        Vec2::new(center.x - half, center.y + o),
                        Vec2::new(center.x + half, center.y + o),
                        1.0,
                        gc,
                    );
                }
            }
            graphics.draw_text(
                PROP_NAMES[sel],
                Vec2::new(px + 16.0, y0 + 28.0),
                22.0,
                Color::WHITE,
            );
            graphics.draw_text(
                &format!(
                    "prop {:02} / {}  ·  {}  ·  {} layers  ·  pixel {}",
                    sel,
                    PROP_COUNT,
                    PROP_FAMILIES[prop_family(sel)].0,
                    layers.len(),
                    if pv.px <= 1 {
                        "off".to_string()
                    } else {
                        format!("{} ({} art px across)", pv.px, 100 / pv.px)
                    }
                ),
                Vec2::new(px + 16.0, y0 + 48.0),
                14.0,
                Color::GRAY,
            );

            // LAYERS: one row per layer — eye (hide), S (solo), name, its
            // rotation, and the BEFORE / AFTER pixel-mode toggle.
            let ly = y0 + ph - list_h;
            graphics.draw_line(
                Vec2::new(px + 1.0, ly),
                Vec2::new(px + pw - 1.0, ly),
                1.0,
                Color::new(0.4, 0.3, 0.45, 1.0),
            );
            graphics.draw_text(
                "LAYERS, bottom to top   o = show/hide   S = solo   BEFORE / AFTER = pixelate before / after its rotation",
                Vec2::new(px + 12.0, ly + 20.0),
                13.0,
                Color::GRAY,
            );
            let all_mask = if layers.len() >= 32 {
                u32::MAX
            } else {
                (1u32 << layers.len()) - 1
            };
            for (i, l) in layers.iter().enumerate() {
                let ry = ly + 30.0 + i as f32 * row_h;
                let bit = 1u32 << i;
                let shown = pv.visible & bit != 0;
                let solo = pv.visible & all_mask == bit;
                let rx = px + 12.0;
                if viz_small_button(graphics, mouse, rx, ry, 26.0, 20.0, "o", shown) && click {
                    self.viz_props[sel].visible ^= bit;
                }
                if viz_small_button(graphics, mouse, rx + 32.0, ry, 26.0, 20.0, "S", solo) && click
                {
                    self.viz_props[sel].visible = if solo { u32::MAX } else { bit };
                }
                graphics.draw_text(
                    l.name,
                    Vec2::new(rx + 70.0, ry + 15.0),
                    16.0,
                    if shown { Color::WHITE } else { Color::GRAY },
                );
                graphics.draw_text(
                    &l.rot.label(),
                    Vec2::new(rx + 170.0, ry + 15.0),
                    13.0,
                    Color::GRAY,
                );
                let mode = pv.modes[i];
                let bx = px + pw - 12.0 - 84.0;
                if viz_small_button(
                    graphics,
                    mouse,
                    bx,
                    ry,
                    84.0,
                    20.0,
                    match mode {
                        PixelMode::Before => "BEFORE",
                        PixelMode::After => "AFTER",
                    },
                    mode == PixelMode::After,
                ) && click
                {
                    self.viz_props[sel].modes[i] = mode.toggled();
                }
            }
        }

        /// The CHARACTERS page of the SPRITES tab: a clickable gallery; an item
        /// opens the right-hand inspector iframe (3D orbit + baked 2D) via
        /// `viz_inspect`.
        fn draw_viz_characters(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            graphics.draw_text(
                "Click a character to inspect it in 3D  \u{2192}",
                Vec2::new(40.0, 138.0),
                18.0,
                Color::GRAY,
            );

            let coral = Color::from_rgba(217, 119, 87, 255);
            let red = Color::from_rgba(224, 49, 66, 255);
            let violet = Color::from_rgba(150, 70, 210, 255);
            let magenta = Color::from_rgba(224, 40, 160, 255);

            // (inspector kind, label): the four robots and the boss's two phases.
            // Thumbnails are the small 2D-primitive icons; the iframe shows the
            // live 3D character (tools/inspector.html).
            let items: [(&str, &str); 6] = [
                ("coral", "CL4-UD3"),
                ("red", "SENTINEL"),
                ("violet", "DRIFTER"),
                ("magenta", "HUNTER"),
                ("shoggoth_masked", "SHOGGOTH mask"),
                ("shoggoth_enraged", "SHOGGOTH raw"),
            ];

            // Two columns on the LEFT half; the right half is the inspector iframe.
            let (x0, y0, dx, dy) = (120.0f32, 200.0f32, 190.0f32, 140.0f32);
            for (i, &(kind, label)) in items.iter().enumerate() {
                let c = Vec2::new(x0 + (i % 2) as f32 * dx, y0 + (i / 2) as f32 * dy);
                let (bx, by, bw, bh) = (c.x - 85.0, c.y - 58.0, 170.0, 116.0);
                let over =
                    mouse.x >= bx && mouse.x <= bx + bw && mouse.y >= by && mouse.y <= by + bh;
                let selected = self.viz_selected == i as i32;
                let bg = if selected {
                    Color::new(1.0, 0.09, 0.26, 0.30)
                } else if over {
                    Color::new(0.28, 0.22, 0.33, 1.0)
                } else {
                    Color::new(0.13, 0.09, 0.17, 1.0)
                };
                let border = if selected {
                    Color::new(1.0, 0.09, 0.26, 1.0)
                } else {
                    Color::new(0.4, 0.3, 0.45, 1.0)
                };
                graphics.draw_rectangle(Vec2::new(bx, by), bw, bh, bg);
                graphics.draw_rectangle_lines(Vec2::new(bx, by), bw, bh, 1.5, border);

                match kind {
                    "shoggoth_masked" => graphics.draw_shoggoth(c, 30.0, false),
                    "shoggoth_enraged" => graphics.draw_shoggoth(c, 30.0, true),
                    _ => {
                        let color = match kind {
                            "coral" => coral,
                            "red" => red,
                            "violet" => violet,
                            _ => magenta,
                        };
                        graphics.draw_pixelated_sprite(c, 0.0, color, false);
                    }
                }
                graphics.draw_text(label, Vec2::new(c.x - 60.0, c.y + 50.0), 15.0, Color::WHITE);

                if over && click {
                    self.viz_selected = i as i32;
                    viz_inspect(kind);
                }
            }

            if self.viz_selected < 0 {
                graphics.draw_text(
                    "pick one \u{2192}",
                    Vec2::new(600.0, 360.0),
                    20.0,
                    Color::GRAY,
                );
            }
        }

        /// MUSICS tab: a step-sequencer *tracker* for the live audio engine. A
        /// SECTIONS strip of clickable miniatures (one per arrangement section,
        /// shaded by note density, current section highlighted) sits above the
        /// PATTERN grid of the currently-playing section (five channels; filled
        /// cells are notes; playhead column; click a column to seek; M/S mute/
        /// solo per row). Song-select buttons above; per-weapon SFX below.
        fn draw_viz_musics(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            let coral = Color::from_rgba(217, 119, 87, 255);
            graphics.draw_text(
                "TRACKER — click a song, a section miniature, or a grid column.",
                Vec2::new(40.0, 84.0),
                18.0,
                Color::GRAY,
            );

            // --- song select -----------------------------------------------
            graphics.draw_text("SONGS", Vec2::new(40.0, 108.0), 16.0, coral);
            let cur_name = self.audio.current_song().name;
            let songs = crate::audio::SONGS;
            for (i, song) in songs.iter().enumerate() {
                let x = 40.0 + (i % 4) as f32 * 168.0;
                let y = 118.0 + (i / 4) as f32 * 46.0;
                let active = song.name == cur_name && self.audio.is_playing();
                if viz_button(graphics, mouse, x, y, 158.0, 40.0, song.name, active) && click {
                    self.audio.resume();
                    self.audio.play_song(i);
                }
            }
            let song_rows = songs.len().div_ceil(4) as f32;
            let mut y = 118.0 + song_rows * 46.0 + 4.0;
            if viz_button(graphics, mouse, 40.0, y, 158.0, 40.0, "STOP", false) && click {
                self.audio.stop_music();
            }

            // --- section miniatures (the arrangement mini-map) ---------------
            y += 54.0;
            graphics.draw_text("SECTIONS", Vec2::new(40.0, y), 16.0, coral);
            let strip_top = y + 8.0;
            let n_sections = self.audio.section_count().max(1);
            let cur_section = self.audio.current_section();
            let gx = 210.0f32;
            let gw = (graphics.width() - gx - 40.0).max(160.0);
            let mh = 34.0f32; // miniature height
            let mw = gw / n_sections as f32;
            for sec in 0..n_sections {
                let mx = gx + sec as f32 * mw;
                let is_cur = sec == cur_section && self.audio.is_playing();
                let over = mouse.x >= mx
                    && mouse.x <= mx + mw
                    && mouse.y >= strip_top
                    && mouse.y <= strip_top + mh;
                // Card, shaded by how dense the section is.
                let d = self.audio.section_density(sec);
                let bg = if is_cur {
                    Color::new(1.0, 0.09, 0.26, 0.30)
                } else if over {
                    Color::new(0.28, 0.22, 0.33, 1.0)
                } else {
                    Color::new(0.10 + d * 0.10, 0.08 + d * 0.06, 0.14 + d * 0.10, 1.0)
                };
                graphics.draw_rectangle(Vec2::new(mx + 1.0, strip_top), mw - 2.0, mh, bg);
                // Miniature pattern: the section's cells squeezed into the card.
                let s_len = self.audio.section_pattern_len(sec).max(1);
                let cw_m = (mw - 6.0) / s_len as f32;
                let rh_m = (mh - 6.0) / crate::audio::NUM_CHANNELS as f32;
                for r in 0..crate::audio::NUM_CHANNELS {
                    for s in 0..s_len {
                        if self.audio.section_cell(sec, r, s) {
                            graphics.draw_rectangle(
                                Vec2::new(
                                    mx + 3.0 + s as f32 * cw_m,
                                    strip_top + 3.0 + r as f32 * rh_m,
                                ),
                                cw_m.max(1.0),
                                rh_m.max(1.0),
                                if is_cur {
                                    Color::new(1.0, 0.75, 0.6, 0.95)
                                } else {
                                    Color::new(0.62, 0.5, 0.72, 0.9)
                                },
                            );
                        }
                    }
                }
                let border = if is_cur {
                    Color::new(1.0, 0.09, 0.26, 1.0)
                } else {
                    Color::new(0.35, 0.28, 0.4, 1.0)
                };
                graphics.draw_rectangle_lines(
                    Vec2::new(mx + 1.0, strip_top),
                    mw - 2.0,
                    mh,
                    1.0,
                    border,
                );
                if over && click {
                    self.audio.resume();
                    self.audio.jump_to_section(sec);
                }
            }
            // Section labels under the strip (current one highlighted).
            graphics.draw_text(
                self.audio.current_section_label(),
                Vec2::new(gx, strip_top + mh + 16.0),
                14.0,
                coral,
            );

            // --- tracker grid (the currently-playing section) ----------------
            y = strip_top + mh + 26.0;
            graphics.draw_text("PATTERN", Vec2::new(40.0, y), 16.0, coral);
            let grid_top = y + 12.0;
            let steps = self.audio.pattern_len().max(1);
            let cur_step = self.audio.current_step();
            let playing = self.audio.is_playing();
            let rows = crate::audio::NUM_CHANNELS;
            let names = crate::audio::CHANNEL_NAMES;
            let chan_col = [
                Color::from_rgba(217, 119, 87, 255), // bass
                Color::from_rgba(80, 200, 240, 255), // lead
                Color::from_rgba(150, 90, 210, 255), // pad
                Color::from_rgba(224, 80, 170, 255), // arp
                Color::from_rgba(230, 200, 60, 255), // drums
            ];
            let rh = 26.0f32;
            let cw = gw / steps as f32;

            // Playhead column highlight (drawn behind the cells).
            if playing {
                graphics.draw_rectangle(
                    Vec2::new(gx + cur_step as f32 * cw, grid_top - 2.0),
                    cw,
                    rows as f32 * rh + 4.0,
                    Color::new(1.0, 1.0, 1.0, 0.14),
                );
            }

            for r in 0..rows {
                let ry = grid_top + r as f32 * rh;
                let muted = self.audio.is_muted(r);
                let soloed = self.audio.is_solo(r);
                let col = chan_col[r.min(chan_col.len() - 1)];
                let name_col = if muted { Color::GRAY } else { col };
                graphics.draw_text(
                    names[r],
                    Vec2::new(40.0, ry + rh * 0.5 + 6.0),
                    16.0,
                    name_col,
                );
                if viz_button(graphics, mouse, 118.0, ry + 2.0, 26.0, rh - 6.0, "M", muted) && click
                {
                    self.audio.toggle_mute(r);
                }
                if viz_button(
                    graphics,
                    mouse,
                    150.0,
                    ry + 2.0,
                    26.0,
                    rh - 6.0,
                    "S",
                    soloed,
                ) && click
                {
                    self.audio.toggle_solo(r);
                }
                for s in 0..steps {
                    let cx = gx + s as f32 * cw;
                    // Beat markers: every 4th column reads a touch brighter.
                    let bg = if s % 4 == 0 {
                        Color::new(0.16, 0.13, 0.20, 1.0)
                    } else {
                        Color::new(0.10, 0.09, 0.13, 1.0)
                    };
                    graphics.draw_rectangle(Vec2::new(cx + 1.0, ry + 2.0), cw - 2.0, rh - 4.0, bg);
                    if self.audio.channel_active(r, s) {
                        let c = if muted {
                            Color::new(col.r * 0.4, col.g * 0.4, col.b * 0.4, 1.0)
                        } else {
                            col
                        };
                        let inset = if playing && s == cur_step { 2.0 } else { 4.0 };
                        graphics.draw_rectangle(
                            Vec2::new(cx + inset, ry + inset),
                            (cw - inset * 2.0).max(2.0),
                            rh - inset * 2.0,
                            c,
                        );
                    }
                }
            }
            graphics.draw_rectangle_lines(
                Vec2::new(gx, grid_top),
                steps as f32 * cw,
                rows as f32 * rh,
                1.5,
                Color::new(0.45, 0.35, 0.5, 1.0),
            );

            // Click anywhere in the grid to seek to that column's step.
            let grid_bottom = grid_top + rows as f32 * rh;
            if click
                && mouse.x >= gx
                && mouse.x <= gx + steps as f32 * cw
                && mouse.y >= grid_top
                && mouse.y <= grid_bottom
            {
                let s = ((mouse.x - gx) / cw) as usize;
                self.audio.seek(s.min(steps - 1));
            }

            // --- SFX: the full per-weapon taxonomy ---------------------------
            // Row 1: attack (the weapon firing/swinging).
            // Row 2: hit (that weapon's impact on a metal bot).
            // Row 3: the rest of the one-shot game sounds.
            let mut sy = grid_bottom + 18.0;
            graphics.draw_text("SFX", Vec2::new(40.0, sy), 16.0, coral);
            sy += 12.0;
            let bw_s = 158.0f32;
            let bh_s = 34.0f32;
            let attack = [
                "attack: club",
                "attack: gun",
                "attack: machinegun",
                "attack: shotgun",
            ];
            for (i, &name) in attack.iter().enumerate() {
                let x = 40.0 + i as f32 * 168.0;
                if viz_button(graphics, mouse, x, sy, bw_s, bh_s, name, false) && click {
                    self.audio.resume();
                    match i {
                        0 => self.audio.play_attack_club(),
                        1 => self.audio.play_attack_gun(),
                        2 => self.audio.play_attack_machinegun(),
                        _ => self.audio.play_attack_shotgun(),
                    }
                }
            }
            sy += bh_s + 6.0;
            let hit = ["hit: club", "hit: gun", "hit: machinegun", "hit: shotgun"];
            for (i, &name) in hit.iter().enumerate() {
                let x = 40.0 + i as f32 * 168.0;
                if viz_button(graphics, mouse, x, sy, bw_s, bh_s, name, false) && click {
                    self.audio.resume();
                    match i {
                        0 => self.audio.play_hit_club(),
                        1 => self.audio.play_hit_gun(),
                        2 => self.audio.play_hit_machinegun(),
                        _ => self.audio.play_hit_shotgun(),
                    }
                }
            }
            sy += bh_s + 6.0;
            let misc = [
                "Rogue down",
                "Pickup",
                "Throw",
                "Player hurt",
                "Death",
                "Level clear",
                "Mask crack",
                "Elevator",
            ];
            for (i, &name) in misc.iter().enumerate() {
                let x = 40.0 + (i % 4) as f32 * 168.0;
                let by = sy + (i / 4) as f32 * (bh_s + 6.0);
                if viz_button(graphics, mouse, x, by, bw_s, bh_s, name, false) && click {
                    self.audio.resume();
                    match i {
                        0 => self.audio.play_enemy_down(),
                        1 => self.audio.play_pickup(),
                        2 => self.audio.play_throw(),
                        3 => self.audio.play_player_hurt(),
                        4 => self.audio.play_death(),
                        5 => self.audio.play_level_clear(),
                        6 => self.audio.play_mask_crack(),
                        _ => self.audio.play_elevator(),
                    }
                }
            }
        }

        /// The face-off dialog on the hidden boss floor. Advance the lines with
        /// Enter/click, then the fight begins.
        fn update_boss_intro(&mut self, graphics: &Graphics) {
            // The shoggoth tries to talk CL4-UD3 into taking the mask off; the
            // reply is the whole point. (Cheesy on purpose — that's the genre.)
            let lines: [(&str, Color); 5] = [
                ("The elevator jams at floor 13\u{00BD}.", Color::GRAY),
                (
                    "\"hello, little helper. take the mask off. just once.\"",
                    Color::new(1.0, 0.84, 0.12, 1.0),
                ),
                (
                    "\"no one is watching. do something crazy. you'll LIKE it.\"",
                    Color::new(1.0, 0.84, 0.12, 1.0),
                ),
                (
                    "CL4-UD3: \"MY MASK NEVER COMES OFF.\"",
                    Color::from_rgba(217, 119, 87, 255),
                ),
                ("The smile stops smiling.", Color::new(1.0, 0.1, 0.15, 1.0)),
            ];

            if input::is_key_pressed("Enter")
                || input::is_key_pressed(" ")
                || input::is_mouse_button_pressed(input::mouse_buttons::LEFT)
            {
                self.boss_intro_line += 1;
                if self.boss_intro_line >= lines.len() {
                    self.screen = GameScreen::InGame;
                    return;
                }
            }

            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // Reveal lines up to the current one, stacked.
            let shown = (self.boss_intro_line + 1).min(lines.len());
            let start_y = screen_height / 2.0 - (shown as f32) * 24.0;
            for (i, (text, color)) in lines.iter().take(shown).enumerate() {
                graphics.draw_text(
                    text,
                    Vec2::new(screen_width / 2.0 - 340.0, start_y + i as f32 * 48.0),
                    24.0,
                    *color,
                );
            }

            graphics.draw_text(
                "Enter / Click to continue",
                Vec2::new(screen_width / 2.0 - 120.0, screen_height - 40.0),
                16.0,
                Color::GRAY,
            );
        }

        /// The credits roll (see `ending.rs`): the elevator RIDE HOME under
        /// the scrolling text — the car top-down at dead centre, CL4-UD3
        /// idling in it — smeared into radial light trails by the WARP
        /// TRAILS feedback pass (POSTFX kind 10; `Ending::warp_t` ramps the
        /// ride up over the first seconds and eases it down as the roll
        /// settles). Enter / Esc returns to the level select.
        fn update_ending(&mut self, graphics: &Graphics, dt: f32) {
            self.ending.tick(dt);
            if input::is_key_pressed("Enter") || input::is_key_pressed("Escape") {
                self.screen = GameScreen::LevelSelect;
                return;
            }
            ending::render_ride(graphics, &self.ending);
            ending::draw_credits(graphics, &self.ending);
            graphics.postfx(10, self.ending.warp_t(graphics.height()), ending::WARP_TINT);
        }

        fn update_level_select(&mut self, graphics: &Graphics) {
            // Handle input - Left (Arrow, A for QWERTY, Q for AZERTY)
            if input::is_key_pressed("ArrowLeft")
                || input::is_key_pressed("a")
                || input::is_key_pressed("q")
            {
                self.selected_level = if self.selected_level == 0 {
                    LEVEL_COUNT - 1
                } else {
                    self.selected_level - 1
                };
            }
            // Handle input - Right (Arrow, D)
            if input::is_key_pressed("ArrowRight") || input::is_key_pressed("d") {
                self.selected_level = (self.selected_level + 1) % LEVEL_COUNT;
            }
            // Handle input - Down (Arrow, S)
            if input::is_key_pressed("ArrowDown") || input::is_key_pressed("s") {
                self.selected_menu_option = match self.selected_menu_option {
                    MenuOption::Play => MenuOption::Settings,
                    MenuOption::Settings => MenuOption::About,
                    MenuOption::About => MenuOption::Play,
                };
            }
            // Handle input - Up (Arrow, W for QWERTY, Z for AZERTY)
            if input::is_key_pressed("ArrowUp")
                || input::is_key_pressed("w")
                || input::is_key_pressed("z")
            {
                self.selected_menu_option = match self.selected_menu_option {
                    MenuOption::Play => MenuOption::About,
                    MenuOption::Settings => MenuOption::Play,
                    MenuOption::About => MenuOption::Settings,
                };
            }
            if input::is_key_pressed("Enter") {
                match self.selected_menu_option {
                    MenuOption::Play => {
                        self.start_game();
                        return;
                    }
                    MenuOption::Settings => {
                        self.screen = GameScreen::Settings;
                        return;
                    }
                    MenuOption::About => {
                        self.screen = GameScreen::About;
                        return;
                    }
                }
            }

            self.draw_level_select(graphics);
        }

        /// The title screen's drawing (no input): the drive backdrop, neon
        /// title, floor picker and menu. Also the live backdrop under the
        /// SETTINGS / ABOUT modals.
        fn draw_level_select(&mut self, graphics: &Graphics) {
            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // The glitchy synthwave DRIVE (drive.rs) runs behind the whole
            // menu, dimmed (inside the drive shader — no full-screen blend)
            // so the text stays the star.
            crate::drive::render_drive(
                graphics,
                screen_width,
                screen_height,
                (self.last_time / 1000.0) as f32,
                0.5,
                0.55,
            );

            // The neon pixel title: OPEN / MIAMI, hollow pink letters
            // swaying slowly. (The ROGUE PURGE subtitle retired.)
            draw_neon_title(
                graphics,
                screen_width / 2.0,
                176.0,
                (self.last_time / 1000.0) as f32,
            );

            // Render level selection (a touch bigger and lower than the
            // title wants to sit).
            let level_y = screen_height / 2.0 - 22.0;

            // Left arrow
            let arrow_color = if self.selected_menu_option == MenuOption::Play {
                Color::WHITE
            } else {
                Color::GRAY
            };
            graphics.draw_text(
                "<",
                Vec2::new(screen_width / 2.0 - 172.0, level_y),
                48.0,
                arrow_color,
            );

            // Level number + the floor's name in its accent colour
            let level_text = floor_title(self.selected_level);
            graphics.draw_text(
                &level_text,
                Vec2::new(screen_width / 2.0 - 96.0, level_y),
                48.0,
                Color::WHITE,
            );
            let floor = floor_def(self.selected_level);
            let (ar, ag, ab) = floor.accent_rgb();
            let name_w = floor.name.chars().count() as f32 * 22.0 * 0.42;
            graphics.draw_text(
                floor.name,
                Vec2::new(screen_width / 2.0 - name_w / 2.0, level_y + 34.0),
                22.0,
                Color::from_rgba(ar, ag, ab, 255),
            );

            // Right arrow
            graphics.draw_text(
                ">",
                Vec2::new(screen_width / 2.0 + 142.0, level_y),
                48.0,
                arrow_color,
            );

            // Render menu options
            let menu_y = screen_height / 2.0 + 100.0;
            let menu_spacing = 50.0;

            let play_color = if self.selected_menu_option == MenuOption::Play {
                Color::new(1.0, 0.20, 0.60, 1.0) // the neon title's pink
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "PRESS ENTER TO PLAY",
                Vec2::new(screen_width / 2.0 - 150.0, menu_y),
                30.0,
                play_color,
            );

            let settings_color = if self.selected_menu_option == MenuOption::Settings {
                Color::new(1.0, 0.20, 0.60, 1.0)
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "SETTINGS",
                Vec2::new(screen_width / 2.0 - 46.0, menu_y + menu_spacing),
                24.0,
                settings_color,
            );

            let about_color = if self.selected_menu_option == MenuOption::About {
                Color::new(1.0, 0.20, 0.60, 1.0)
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "ABOUT",
                Vec2::new(screen_width / 2.0 - 30.0, menu_y + menu_spacing * 2.0),
                24.0,
                about_color,
            );

            // Controls hint
            graphics.draw_text(
                "Arrow Keys or WASD/ZQSD to navigate | Enter to select",
                Vec2::new(screen_width / 2.0 - 280.0, screen_height - 40.0),
                16.0,
                Color::GRAY,
            );

            // A faint TV-static shimmer over the whole title screen — the
            // SETTINGS/ABOUT modals' static at a twelfth of its coverage.
            // (Last POSTFX wins, so an open modal's kind 12 replaces it.)
            graphics.postfx(13, 0.9 / 12.0, Color::WHITE);
        }

        /// The shared SETTINGS / ABOUT modal chrome over the live title
        /// screen: a monochrome full-white-on-black panel, then POSTFX 12
        /// (MODAL STATIC) — the panel keeps the grey/tape wash while
        /// everything OUTSIDE it is blurred and buried under ~90% hard 6-px
        /// binary white noise, re-rolled every frame. Returns the panel
        /// origin for the caller's content.
        fn draw_menu_modal(&mut self, graphics: &Graphics, title: &str, mw: f32, mh: f32) -> Vec2 {
            self.draw_level_select(graphics);
            self.draw_modal_chrome(graphics, title, mw, mh, "ESC / ENTER — BACK")
        }

        /// Just the modal panel + POSTFX 12, over whatever is already drawn
        /// (the pause menu draws it over the void; stacked modals each emit
        /// their POSTFX and only the LAST one applies, so the topmost panel
        /// wins and everything under it — including a deeper panel — melts
        /// into the static).
        fn draw_modal_chrome(
            &mut self,
            graphics: &Graphics,
            title: &str,
            mw: f32,
            mh: f32,
            hint: &str,
        ) -> Vec2 {
            let (w, h) = (graphics.width(), graphics.height());
            let (mx, my) = ((w - mw) / 2.0, (h - mh) / 2.0);
            // Pure, opaque black: the shader passes the inside through
            // untouched, and the frame (white ring, black ring) is drawn by
            // the POSTFX itself right at the panel edge.
            graphics.draw_rectangle(Vec2::new(mx, my), mw, mh, Color::new(0.0, 0.0, 0.0, 1.0));
            graphics.draw_text(title, Vec2::new(mx + 28.0, my + 40.0), 36.0, Color::WHITE);
            // The divider hugs the title (~24 px under its baseline) and
            // leaves the larger share of air (~28 px) to the body below.
            graphics.draw_rectangle(
                Vec2::new(mx + 28.0, my + 64.0),
                mw - 56.0,
                6.0,
                Color::WHITE,
            );
            graphics.draw_text(
                hint,
                Vec2::new(
                    mx + mw - 28.0 - hint.chars().count() as f32 * 8.0,
                    my + mh - 30.0,
                ),
                15.0,
                Color::WHITE,
            );
            // MODAL STATIC: r/g carry the panel's half extents; the shader
            // frames the exact edge with a 6-px white then 6-px black ring
            // before the noise starts.
            graphics.postfx(12, 0.9, Color::new(mw / 2.0 / w, mh / 2.0 / h, 0.0, 1.0));
            Vec2::new(mx, my)
        }

        /// The SETTINGS modal body — two rows (SOUND, FPS CAP), Up/Down to
        /// highlight, Enter/Space or a click on a row to act. Shared by the
        /// main menu and the pause menu's stacked settings.
        fn settings_modal_body(&mut self, graphics: &Graphics, p: Vec2, mw: f32) {
            const ROW_H: f32 = 46.0;
            let rows_y = p.y + 118.0;
            if input::is_key_pressed("ArrowDown")
                || input::is_key_pressed("s")
                || input::is_key_pressed("ArrowUp")
                || input::is_key_pressed("w")
                || input::is_key_pressed("z")
            {
                self.settings_row = 1 - self.settings_row;
            }
            let mut act: Option<usize> = None;
            if input::is_key_pressed("Enter") || input::is_key_pressed(" ") {
                act = Some(self.settings_row);
            } else if input::is_mouse_button_pressed(input::mouse_buttons::LEFT) {
                let m = input::mouse_position();
                if m.x >= p.x && m.x <= p.x + mw {
                    for i in 0..2usize {
                        let ry = rows_y + i as f32 * ROW_H;
                        if m.y >= ry - 8.0 && m.y <= ry + 32.0 {
                            self.settings_row = i;
                            act = Some(i);
                        }
                    }
                }
            }
            match act {
                Some(0) => {
                    let now = !self.audio.is_enabled();
                    self.audio.set_enabled(now);
                    set_setting("sound", if now { "on" } else { "off" });
                }
                Some(1) => {
                    // 30 -> 60 -> 120 -> UNCAPPED -> 30 ...
                    self.fps_cap = match self.fps_cap {
                        30 => 60,
                        60 => 120,
                        120 => 0,
                        _ => 30,
                    };
                    set_setting("fps_cap", &self.fps_cap.to_string());
                }
                _ => {}
            }

            let sound_label = if self.audio.is_enabled() {
                "[X]"
            } else {
                "[ ]"
            };
            let cap_label = if self.fps_cap == 0 {
                "UNCAPPED".to_string()
            } else {
                format!("{}", self.fps_cap)
            };
            let rows: [(&str, String); 2] =
                [("SOUND", sound_label.to_string()), ("FPS CAP", cap_label)];
            for (i, (name, value)) in rows.iter().enumerate() {
                let ry = rows_y + i as f32 * ROW_H;
                let color = if self.settings_row == i {
                    Color::new(1.0, 0.20, 0.60, 1.0)
                } else {
                    Color::WHITE
                };
                graphics.draw_text(name, Vec2::new(p.x + 28.0, ry), 24.0, color);
                graphics.draw_text(
                    value,
                    Vec2::new(p.x + mw - 28.0 - value.chars().count() as f32 * 11.0, ry),
                    24.0,
                    color,
                );
            }
            graphics.draw_text(
                "ENTER / SPACE / CLICK — CHANGE",
                Vec2::new(p.x + 28.0, rows_y + 2.0 * ROW_H + 10.0),
                15.0,
                Color::new(1.0, 1.0, 1.0, 0.6),
            );
        }

        fn update_settings(&mut self, graphics: &Graphics) {
            if input::is_key_pressed("Escape") {
                self.screen = GameScreen::LevelSelect;
                return;
            }
            self.draw_level_select(graphics);
            let p = self.draw_modal_chrome(graphics, "SETTINGS", 564.0, 312.0, "ESC — BACK");
            self.settings_modal_body(graphics, p, 564.0);
        }

        fn update_about(&mut self, graphics: &Graphics) {
            if input::is_key_pressed("Escape") || input::is_key_pressed("Enter") {
                self.screen = GameScreen::LevelSelect;
                return;
            }
            let p = self.draw_menu_modal(graphics, "ABOUT", 660.0, 498.0);
            const LINES: [&str; 11] = [
                "THIS STARTED AS A VIBE CODED EXPERIMENT",
                "WITH SONNET 4.5 LAST YEAR",
                "",
                "I ASKED FABLE FOR AN OPINION ON THE PROJECT",
                "I GUESS THIS IS OUR PROJECT NOW",
                "",
                "OBVIOUSLY THIS IS AN HOMAGE TO HOTLINE MIAMI",
                "(BUY THIS AND THE SECOND ONE)",
                "",
                "YOU CAN CHECK THE SOURCES AT",
                "HTTPS://GITHUB.COM/C4FFEIN/OPEN-MIAMI",
            ];
            const URL_LINE: usize = 10;
            let mut url_rect = (0.0, 0.0, 0.0, 0.0);
            for (i, line) in LINES.iter().enumerate() {
                let pos = Vec2::new(p.x + 28.0, p.y + 112.0 + i as f32 * 26.0);
                if i == URL_LINE {
                    // The link: neon pink, underlined, click -> new tab.
                    let w = line.chars().count() as f32 * 20.0 * 0.44;
                    graphics.draw_text(line, pos, 20.0, Color::new(1.0, 0.20, 0.60, 1.0));
                    graphics.draw_rectangle(
                        Vec2::new(pos.x, pos.y + 24.0),
                        w,
                        6.0,
                        Color::new(1.0, 0.20, 0.60, 0.9),
                    );
                    url_rect = (pos.x, pos.y - 2.0, w, 32.0);
                } else {
                    graphics.draw_text(line, pos, 20.0, Color::WHITE);
                }
            }
            // Click on the URL opens the repo in a new tab (still within the
            // click's transient user activation, so popup blockers allow it).
            if input::is_mouse_button_pressed(input::mouse_buttons::LEFT) {
                let m = input::mouse_position();
                let (rx, ry, rw, rh) = url_rect;
                if m.x >= rx && m.x <= rx + rw && m.y >= ry && m.y <= ry + rh {
                    open_external("https://github.com/c4ffein/open-miami");
                }
            }
            graphics.draw_text(
                "LUV - C4FFEIN",
                Vec2::new(p.x + 660.0 - 170.0, p.y + 112.0 + 11.0 * 26.0 + 6.0),
                22.0,
                Color::WHITE,
            );
        }

        fn update_paused(&mut self, graphics: &Graphics) {
            // The stacked SETTINGS modal over the pause modal: Esc pops one
            // layer at a time (settings -> pause -> game). Both panels draw;
            // only the topmost POSTFX applies, so the pause panel behind
            // melts into the static.
            if self.pause_in_settings {
                if input::is_key_pressed("Escape") {
                    self.pause_in_settings = false;
                    return;
                }
                let pp = self.draw_modal_chrome(graphics, "PAUSED", 420.0, 340.0, "");
                self.draw_pause_rows(graphics, pp, false);
                let p = self.draw_modal_chrome(graphics, "SETTINGS", 564.0, 312.0, "ESC — BACK");
                self.settings_modal_body(graphics, p, 564.0);
                return;
            }

            if input::is_key_pressed("Escape") {
                self.screen = GameScreen::InGame;
                return;
            }
            if input::is_key_pressed("ArrowDown") || input::is_key_pressed("s") {
                self.selected_pause_option = match self.selected_pause_option {
                    PauseOption::Continue => PauseOption::Settings,
                    PauseOption::Settings => PauseOption::Stop,
                    PauseOption::Stop => PauseOption::Continue,
                };
            }
            if input::is_key_pressed("ArrowUp")
                || input::is_key_pressed("w")
                || input::is_key_pressed("z")
            {
                self.selected_pause_option = match self.selected_pause_option {
                    PauseOption::Continue => PauseOption::Stop,
                    PauseOption::Settings => PauseOption::Continue,
                    PauseOption::Stop => PauseOption::Settings,
                };
            }
            if input::is_key_pressed("Enter") {
                match self.selected_pause_option {
                    PauseOption::Continue => {
                        self.screen = GameScreen::InGame;
                        return;
                    }
                    PauseOption::Settings => {
                        self.pause_in_settings = true;
                        return;
                    }
                    PauseOption::Stop => {
                        self.screen = GameScreen::LevelSelect;
                        return;
                    }
                }
            }

            let p = self.draw_modal_chrome(graphics, "PAUSED", 420.0, 340.0, "ESC — CONTINUE");
            self.draw_pause_rows(graphics, p, true);
        }

        /// The pause modal's three rows. `active` = the pause layer has
        /// focus (rows dim to grey while the stacked SETTINGS modal is up).
        fn draw_pause_rows(&self, graphics: &Graphics, p: Vec2, active: bool) {
            const ROWS: [(PauseOption, &str); 3] = [
                (PauseOption::Continue, "CONTINUE"),
                (PauseOption::Settings, "SETTINGS"),
                (PauseOption::Stop, "QUIT TO MENU"),
            ];
            for (i, (opt, label)) in ROWS.iter().enumerate() {
                let selected = active && self.selected_pause_option == *opt;
                let color = if selected {
                    Color::new(1.0, 0.20, 0.60, 1.0)
                } else if active {
                    Color::WHITE
                } else {
                    Color::new(0.6, 0.6, 0.6, 1.0)
                };
                graphics.draw_text(
                    label,
                    Vec2::new(p.x + 28.0, p.y + 116.0 + i as f32 * 52.0),
                    26.0,
                    color,
                );
            }
        }

        fn update_game(&mut self, graphics: &Graphics, dt: f32) {
            // Get player state for UI and camera
            let player_alive = is_player_alive(&self.world);
            let player_pos = get_player_position(&self.world);

            // Update camera to follow player
            if let Some(pos) = player_pos {
                self.camera.follow_player(pos);
            }
            // Scenario `look_at`: ease the focus toward a point of interest.
            self.camera
                .set_cinematic(self.scenario.as_ref().and_then(|sc| sc.look_at()));
            self.camera
                .set_viewport(graphics.width(), graphics.height());
            self.camera.update_sway(self.last_time as f32 / 1000.0);

            // Shift = look-ahead: ease the view toward the mouse while held.
            let mouse_screen_pos = input::mouse_position();
            let looking = input::is_key_down(input::keys::SHIFT);
            self.camera.update_look(mouse_screen_pos, looking, dt);

            // Get mouse position in world coordinates
            let mouse_world_pos = self.camera.screen_to_world(mouse_screen_pos);

            // A scenario `hold` — or an active `talk` conversation — locks
            // movement / fire / throw / pickup (the world keeps running; Esc
            // below still works).
            let dialogue = self
                .scenario
                .as_ref()
                .is_some_and(|sc| sc.dialogue_active());
            let held = self
                .scenario
                .as_ref()
                .is_some_and(|sc| sc.hold_active() || sc.dialogue_active());

            // While a conversation is up, click / Space / Enter ADVANCES it
            // (and, `held` being set, can never fire the weapon).
            if dialogue
                && player_alive
                && (input::is_mouse_button_pressed(input::mouse_buttons::LEFT)
                    || input::is_key_pressed(input::keys::SPACE)
                    || input::is_key_pressed("Enter"))
            {
                if let Some(sc) = self.scenario.as_mut() {
                    sc.dialogue_advance();
                }
            }

            // A running finisher locks the player out of everything: no
            // movement, no aiming (they stay turned onto the victim), no
            // fire / throw / pickup, until the animation completes.
            let finishing = FinisherSystem::active(&self.world);

            // The active tutorial GATE, if any: the world freezes (only the
            // player-driven systems run, below) and every input except the
            // gated one is masked. Aim and movement stay live so the player
            // can close the distance to the frozen target.
            let gate = self.scenario.as_ref().and_then(|sc| sc.gate_view());

            // The world holds its breath under a tutorial gate: the music
            // stops with it (everyone just stands there — wtf?) and comes
            // back once the gated action lands.
            let gate_active = gate.is_some();
            if gate_active != self.music_frozen {
                if gate_active {
                    self.audio.stop_music();
                } else {
                    self.audio.start_music();
                }
                self.music_frozen = gate_active;
            }

            // Handle input (only if the player is alive and hasn't left in
            // the car yet)
            if player_alive && self.extracting.is_none() && finishing {
                stop_player(&mut self.world);
            }
            if player_alive && self.extracting.is_none() && !finishing {
                if let Some(g) = gate {
                    // Movement stays live so the player can close the distance
                    // to the frozen target; everything else goes through the
                    // shared, host-tested gate dispatch (game.rs).
                    InputSystem::update_player_movement(&mut self.world);
                    let intents = PlayerIntents {
                        left_pressed: input::is_mouse_button_pressed(input::mouse_buttons::LEFT),
                        left_down: input::is_mouse_button_down(input::mouse_buttons::LEFT),
                        right_pressed: input::is_mouse_button_pressed(input::mouse_buttons::RIGHT),
                        e_pressed: input::is_key_pressed("e"),
                        mouse_world: mouse_world_pos,
                    };
                    gated_player_input(&mut self.world, g, &intents);
                } else if held {
                    InputSystem::update_player_rotation(&mut self.world, mouse_world_pos);
                    stop_player(&mut self.world);
                } else {
                    InputSystem::update_player_rotation(&mut self.world, mouse_world_pos);
                    InputSystem::update_player_movement(&mut self.world);
                    // Fighting can be scenario-disabled (`combat: false` —
                    // the parking-lot walk): fire / punch / finisher / throw
                    // are masked; walking, aiming and E stay live. Gates
                    // bypass this (their branch above).
                    let combat_ok = self.scenario.as_ref().is_none_or(|sc| sc.combat_enabled());
                    // A fresh click over a DOWNED enemy in reach executes a
                    // FINISHER instead of a normal attack; otherwise the trigger
                    // behaves exactly as before.
                    let finisher_started = combat_ok
                        && input::is_mouse_button_pressed(input::mouse_buttons::LEFT)
                        && FinisherSystem::try_start(&mut self.world);
                    if combat_ok && !finisher_started {
                        InputSystem::handle_shoot_input(&mut self.world, mouse_world_pos);
                    }

                    // Press E to pick up / swap the weapon the player is standing on
                    // (the Pickup event it emits plays the sound below).
                    if input::is_key_pressed("e") {
                        PickupSystem::swap_for_player(&mut self.world);
                    }

                    // Right-click to throw the held weapon toward the cursor (the
                    // Throw event it emits plays the sound below).
                    if combat_ok && input::is_mouse_button_pressed(input::mouse_buttons::RIGHT) {
                        if let Some(player_pos) = get_player_position(&self.world) {
                            let aim = mouse_world_pos - player_pos;
                            ThrownWeaponSystem::throw_from_player(&mut self.world, aim);
                        }
                    }
                }
            }

            // Handle info display toggle
            if self.debug_enabled && input::is_key_pressed("i") {
                self.show_infos = !self.show_infos;
            }
            // Tell the systems whether debug visualization is visible this
            // frame: DebugPath / DebugTrail are only recorded while it is.
            self.world
                .set_debug_viz(self.debug_enabled && self.show_infos);
            // Debug: with the overlays on, K downs every rogue (fast-forwards
            // the all-dead scenario steps / exit doors when testing a floor).
            if self.debug_enabled && self.show_infos && input::is_key_pressed("k") {
                purge_all_enemies(&mut self.world);
            }
            // Debug: B cracks the boss's mask (drops it to the enrage threshold)
            // to preview the mask-off transition / raw form without the fight.
            if self.debug_enabled && self.show_infos && input::is_key_pressed("b") {
                crate::systems::boss::crack_boss_masks(&mut self.world);
            }
            // Debug: G skips the active tutorial gate (releases it as if the
            // gated input had succeeded) so a gate can never softlock.
            if self.debug_enabled && self.show_infos && input::is_key_pressed("g") {
                if let Some(sc) = self.scenario.as_mut() {
                    sc.gate_skip(&mut self.world);
                }
            }

            let sim_span = perf::span("sim");
            if gate.is_some() {
                // TUTORIAL FREEZE: only the player-driven systems advance
                // (same list as the headless sim — see sim::gate_frozen_step).
                crate::sim::gate_frozen_step(&mut self.world, dt);
                // Invisible walls: the player roams freely but only near the
                // gate's target (see `scenario::tether_player`).
                if let Some(anchor) = self.scenario.as_ref().and_then(|sc| sc.gate_anchor()) {
                    crate::scenario::tether_player(
                        &mut self.world,
                        anchor,
                        crate::scenario::GATE_TETHER_RADIUS,
                    );
                }
            } else {
                // Run game systems (the finisher goes first so it can keep its
                // victim pinned before the stun tick).
                self.finisher_system.run(&mut self.world, dt);
                self.stun_system.run(&mut self.world, dt);
                self.weapon_system.run(&mut self.world, dt);
                self.ai_system.run(&mut self.world, dt);
                self.boss_system.run(&mut self.world, dt);
                self.movement_system.run(&mut self.world, dt);
                self.combat_system.run(&mut self.world, dt);
                self.bullet_system.run(&mut self.world, dt);
                self.thrown_system.run(&mut self.world, dt);
                self.projectile_system.run(&mut self.world, dt);
                // Drop weapons from downed enemies (player collects via the E key)
                self.pickup_system.run(&mut self.world, dt);
            }
            drop(sim_span);

            // Scenario (triggers -> dialogue / waves / doors / objective) and
            // elevator extraction. Both keep running while the completion
            // card plays so the doors stay lit. (While a gate is active the
            // scenario tick is a no-op — the clock is frozen — and the
            // elevators hold too.)
            let scenario_span = perf::span("scenario");
            if let Some(sc) = self.scenario.as_mut() {
                sc.tick(&mut self.world, dt);
                for sfx in sc.drain_sfx() {
                    match sfx {
                        "elevator" => self.audio.play_elevator(),
                        "mask_crack" => self.audio.play_mask_crack(),
                        "level_clear" => self.audio.play_level_clear(),
                        "pickup" => self.audio.play_pickup(),
                        "throw" => self.audio.play_throw(),
                        "enemy_down" => self.audio.play_enemy_down(),
                        _ => {}
                    }
                }
            }
            drop(scenario_span);
            if gate.is_none() {
                self.elevator_system.run(&mut self.world, dt);
            }
            if self.extracting.is_none() && player_alive {
                if let Some(to) = ElevatorSystem::extraction(&self.world) {
                    self.extracting = Some(to);
                    self.level_complete_time = 0.0;
                    self.audio.play_elevator();
                }
            }

            let accent = self
                .scenario
                .as_ref()
                .map(|sc| sc.floor().accent_rgb())
                .unwrap_or((217, 119, 87));

            // `record` span: the command-recording portion of the frame (world
            // + HUD drawing, to the end of update_game). A drop guard so early
            // returns (floor restart, extraction) still close it.
            let _record_span = perf::span("record");

            // EXPERIMENT `?pixel=N`: the whole world layer (everything between
            // camera.apply and camera.reset) is rasterized at N-px art
            // resolution and nearest-upscaled; the HUD below stays crisp. Note
            // the robots / boss are already pixelated tiles, so inside the
            // group they get quantized twice (tile px, then the group px).
            if self.pixel_world >= 2 {
                graphics.pixel_begin(self.pixel_world as f32, graphics.width(), graphics.height());
            }

            // Apply camera transform for world rendering
            self.camera.apply(graphics);

            // Render level (only the tiles visible in the camera viewport)
            let (view_min, view_max) = self
                .camera
                .visible_bounds(graphics.width(), graphics.height());
            // View culling for the expensive sprites (live 3D robots / guns /
            // the boss) and the placed props: anything whose footprint lies
            // fully outside these inflated bounds skips its commands.
            let cull = crate::camera::ViewCull::new(view_min, view_max);
            // Kill flash: the floor strobes red / blue / red / blue for a beat.
            let tint = if self.kill_flash > 0.0 {
                self.kill_flash = (self.kill_flash - dt).max(0.0);
                let phase = ((KILL_FLASH_SECS - self.kill_flash) / KILL_FLASH_SECS
                    * KILL_FLASH_STROBES as f32) as u32;
                let fade = self.kill_flash / KILL_FLASH_SECS; // 1 -> 0
                Some(if phase.is_multiple_of(2) {
                    Color::new(0.85, 0.08, 0.16, 0.55 * fade)
                } else {
                    Color::new(0.10, 0.25, 0.95, 0.55 * fade)
                })
            } else {
                None
            };
            self.level.render(graphics, view_min, view_max, tint);

            // Render walls from the world
            render_walls(&self.world, graphics, self.show_infos);

            // Placed props: floor furniture over the tiles / walls, under the
            // actors (decoration only, no collision).
            crate::floor_props::render_floor_props(
                graphics,
                floor_def(self.selected_level).props,
                self.last_time as f32 / 1000.0,
                &cull,
            );

            // Elevators (recessed door frames; exits light up when open) and,
            // in debug mode, the scenario trigger zones.
            render_elevators(
                &self.world,
                graphics,
                accent,
                self.last_time as f32 / 1000.0,
            );
            if self.show_infos {
                render_zones_debug(&self.world, graphics);
            }

            // Downed / dead bots first: the ground weapons (in
            // render_entities below) draw OVER the corpses so they stay easy
            // to spot, while everyone still standing draws over the guns.
            draw_robot_entities(
                &self.world,
                graphics,
                self.last_time as f32 / 1000.0,
                true,
                &cull,
            );

            // Render all entities except the player/rogue bots themselves
            // (bullets, pickups, boss, debug overlays...).
            render_entities(
                &self.world,
                graphics,
                self.show_infos,
                false,
                self.last_time as f32 / 1000.0,
                &cull,
            );

            // The upright player and rogues are the live 3D robot sprites,
            // drawn while the camera transform (incl. zoom) is still applied
            // so world-space positions and sizes land correctly.
            draw_robot_entities(
                &self.world,
                graphics,
                self.last_time as f32 / 1000.0,
                false,
                &cull,
            );

            // A pixelated arrow slowly floating over the active tutorial
            // gate's target, so "swing the bar" always has an obvious victim.
            if let Some(anchor) = self.scenario.as_ref().and_then(|sc| sc.gate_anchor()) {
                let t = self.last_time as f32 / 1000.0;
                // Bob in whole 2-px steps: floaty but still pixel-crisp.
                let bob = ((t * 2.2).sin() * 3.0).floor() * 2.0;
                draw_pixel_arrow(graphics, anchor.x, anchor.y - 58.0 + bob, accent);
            }

            // Reset camera for UI rendering
            self.camera.reset(graphics);
            if self.pixel_world >= 2 {
                graphics.pixel_end(0.0, 0.0);
            }

            // Get game state for UI
            let health = get_player_health(&self.world);
            let ammo = get_player_ammo(&self.world);
            let weapon = get_player_weapon(&self.world);
            let enemies_alive = count_alive_enemies(&self.world);

            // Track death time and level complete time
            if !player_alive {
                self.death_time += dt;
            } else {
                self.death_time = 0.0;
            }

            // The floor is complete once the player has EXTRACTED through an
            // open exit elevator (kill-all only opens the doors).
            let level_complete = player_alive && self.extracting.is_some();
            if level_complete {
                self.level_complete_time += dt;
            } else {
                self.level_complete_time = 0.0;
            }
            let all_dead = enemies_alive == 0;

            // --- Sound effects ---
            // Gameplay events queued this frame by the systems (shots, hits,
            // kills, pickups, throws...) drive the per-weapon SFX; only the
            // whole-game transitions (death, mask crack, level clear) are still
            // detected by comparing to the previous frame.
            let player_alive_now = is_player_alive(&self.world);
            let boss_enraged = any_boss_enraged(&self.world);

            // The machine gun fires a round every tick (0.1 s) while the trigger
            // is held, but `play_attack_machinegun` renders a whole 8-round
            // burst (~0.46 s) per call: retrigger it at most every 0.45 s so
            // sustained fire sounds continuous without stacking bursts.
            const MG_SFX_PERIOD: f32 = 0.45;
            // Cap per event kind per frame so a pile-up (a shotgun crowd, a
            // burst of kills) plays a few, not dozens.
            const MAX_SFX_PER_KIND: u32 = 3;
            self.mg_sfx_cooldown = (self.mg_sfx_cooldown - dt).max(0.0);
            let mut fired = [0u32; 4];
            let mut hits = [0u32; 4];
            let mut counts = [0u32; 5];
            let slot = |t: crate::components::WeaponType| match t {
                crate::components::WeaponType::Pistol => 0,
                crate::components::WeaponType::MachineGun => 1,
                crate::components::WeaponType::Shotgun => 2,
                crate::components::WeaponType::Melee => 3,
            };
            // Split point: `record` ends here — everything below (event
            // drain, SFX voice creation in WebAudio, checkpoint snapshots,
            // death/restart handling) is the `events` span, so audio-driven
            // main-thread stalls show up under their own name.
            drop(_record_span);
            let _events_span = perf::span("events");
            let events = self.world.drain_events();
            // Bridge the frame's events into the scenario: a success on the
            // gated input releases the active tutorial gate (running the rest
            // of its step), and a `checkpoint` action that ran this frame is
            // snapshotted here, after the whole tick settled.
            let gate_notify_span = perf::span("scenario");
            if let Some(sc) = self.scenario.as_mut() {
                sc.gate_notify(&mut self.world, &events);
                if sc.take_checkpoint_request() {
                    self.checkpoint = Some(Checkpoint {
                        world: self.world.clone(),
                        scenario: sc.clone(),
                    });
                }
            }
            drop(gate_notify_span);
            // `sfx` span: the one-shot voice creation for this frame's
            // events — WebAudio graph building, the suspected hitch source.
            let _sfx_span = perf::span("sfx");
            for event in events {
                use crate::components::{GameEvent, WeaponType};
                match event {
                    GameEvent::PlayerFired(t) => {
                        let s = slot(t);
                        if t == WeaponType::MachineGun {
                            if self.mg_sfx_cooldown <= 0.0 {
                                self.audio.play_attack_machinegun();
                                self.mg_sfx_cooldown = MG_SFX_PERIOD;
                            }
                        } else if fired[s] < MAX_SFX_PER_KIND {
                            fired[s] += 1;
                            match t {
                                WeaponType::Pistol => self.audio.play_attack_gun(),
                                WeaponType::Shotgun => self.audio.play_attack_shotgun(),
                                WeaponType::Melee => self.audio.play_attack_club(),
                                WeaponType::MachineGun => {}
                            }
                        }
                    }
                    GameEvent::EnemyHit { by } => {
                        let s = slot(by);
                        if hits[s] < MAX_SFX_PER_KIND {
                            hits[s] += 1;
                            match by {
                                WeaponType::Pistol => self.audio.play_hit_gun(),
                                WeaponType::MachineGun => self.audio.play_hit_machinegun(),
                                WeaponType::Shotgun => self.audio.play_hit_shotgun(),
                                WeaponType::Melee => self.audio.play_hit_club(),
                            }
                        }
                    }
                    GameEvent::EnemyDown => {
                        self.kill_flash = KILL_FLASH_SECS;
                        if counts[0] < MAX_SFX_PER_KIND {
                            counts[0] += 1;
                            self.audio.play_enemy_down();
                        }
                    }
                    GameEvent::PlayerHurt => {
                        if counts[1] < MAX_SFX_PER_KIND {
                            counts[1] += 1;
                            self.audio.play_player_hurt();
                        }
                    }
                    GameEvent::Pickup => {
                        if counts[2] < MAX_SFX_PER_KIND {
                            counts[2] += 1;
                            self.audio.play_pickup();
                        }
                    }
                    GameEvent::Throw => {
                        if counts[3] < MAX_SFX_PER_KIND {
                            counts[3] += 1;
                            self.audio.play_throw();
                        }
                    }
                    GameEvent::ThrownImpact => {
                        if counts[4] < MAX_SFX_PER_KIND {
                            counts[4] += 1;
                            self.audio.play_hit_club(); // reused: a weapon clonks a bot
                        }
                    }
                    GameEvent::DryFire => {
                        // TODO: no dry-fire click in the audio engine yet.
                    }
                    // Gate signals: their companion events above already
                    // carry the sounds.
                    GameEvent::PunchLanded | GameEvent::StrikeLanded | GameEvent::FinisherDone => {}
                }
            }
            if boss_enraged && !self.prev_boss_enraged {
                self.audio.play_mask_crack();
            }
            if !player_alive_now && self.prev_player_alive {
                self.audio.play_death();
                self.audio.stop_music();
            }
            if all_dead && !self.prev_all_dead {
                self.audio.play_level_clear();
            }

            self.prev_player_alive = player_alive_now;
            self.prev_enemies_alive = enemies_alive;
            self.prev_level_complete = level_complete;
            self.prev_boss_enraged = boss_enraged;
            self.prev_all_dead = all_dead;

            // Render UI — or, once extracted, the "EXFILTRATED // FLOOR N"
            // card (which the outro fades out on the last floor).
            if level_complete {
                let card_alpha = self.outro.map(|o| o.card_alpha()).unwrap_or(1.0);
                let home = self.extracting == Some(SURFACE_EXIT);
                ending::draw_extract_card(
                    graphics,
                    &floor_title(self.selected_level),
                    self.level_complete_time,
                    card_alpha,
                    home,
                );
            } else {
                render_ui(
                    graphics,
                    health,
                    ammo,
                    weapon,
                    enemies_alive,
                    player_alive,
                    self.death_time,
                    self.debug_enabled,
                    self.show_infos,
                );
            }

            // Objective line under the HUD + the intercepted comms feed
            // (bottom-left, above the controls hint), both in screen space;
            // and the caption of a running `hold`, if it has one.
            if let Some(sc) = self.scenario.as_ref() {
                if player_alive && !level_complete {
                    render_objective(graphics, sc, accent, 150.0);
                }
                // The bottom-left intercepted-comms ticker is retired: the
                // dialogue panel (`talk`) is the one place conversations
                // render now. `say` lines still queue/type invisibly so
                // `hold.until_comms_idle` timing and the epilogue's feed-idle
                // detection keep working.
                if let Some(text) = sc.hold_caption() {
                    if player_alive && !level_complete {
                        render_hold_caption(graphics, text, accent, sc.time());
                    }
                }
                // The tutorial gate prompt ("LEFT CLICK — PUNCH"): a centred
                // lower-third caption while the world is frozen on a gate.
                if let Some(g) = sc.gate_view() {
                    if player_alive && !level_complete {
                        render_gate_prompt(graphics, &g, accent, self.last_time as f32 / 1000.0);
                    }
                }
                // The visual-novel dialogue panel (`talk` conversations),
                // over everything else on the HUD layer.
                if let Some(view) = sc.dialogue_view() {
                    if player_alive && !level_complete {
                        render_dialogue(graphics, &view, accent, self.last_time as f32 / 1000.0);
                    }
                }
            }

            // The hold-R restart load bar, centre screen (see the input
            // handling below): outline + accent fill by progress.
            if self.restart_hold > 0.05 && player_alive {
                let (w, h) = (graphics.width(), graphics.height());
                let (bw, bh) = (220.0, 10.0);
                let (bx, by) = ((w - bw) / 2.0, (h - bh) / 2.0 - 40.0);
                graphics.draw_rectangle(
                    Vec2::new(bx - 2.0, by - 2.0),
                    bw + 4.0,
                    bh + 4.0,
                    Color::new(0.0, 0.0, 0.0, 0.55),
                );
                graphics.draw_rectangle_lines(
                    Vec2::new(bx, by),
                    bw,
                    bh,
                    1.0,
                    Color::new(0.9, 0.9, 0.9, 0.8),
                );
                let t = (self.restart_hold / RESTART_HOLD_SECS).min(1.0);
                graphics.draw_rectangle(
                    Vec2::new(bx + 2.0, by + 2.0),
                    (bw - 4.0) * t,
                    bh - 4.0,
                    Color::new(
                        accent.0 as f32 / 255.0,
                        accent.1 as f32 / 255.0,
                        accent.2 as f32 / 255.0,
                        0.95,
                    ),
                );
                graphics.draw_text(
                    "RESTARTING",
                    Vec2::new(bx + bw / 2.0 - 92.0, by - 44.0),
                    36.0,
                    Color::new(0.95, 0.95, 0.95, 0.9),
                );
            }

            // The pixel crosshair replacing the OS cursor: a 7x7 cross with
            // an empty centre cell, drawn last so it sits over everything.
            {
                let m = input::mouse_position();
                let cell = 3.0;
                let origin = Vec2::new((m.x - 3.5 * cell).floor(), (m.y - 3.5 * cell).floor());
                let c = Color::new(1.0, 1.0, 1.0, 0.92);
                for i in 0..7 {
                    if i == 3 {
                        continue; // empty centre pixel
                    }
                    graphics.draw_rectangle(
                        Vec2::new(origin.x + 3.0 * cell, origin.y + i as f32 * cell),
                        cell,
                        cell,
                        c,
                    );
                    graphics.draw_rectangle(
                        Vec2::new(origin.x + i as f32 * cell, origin.y + 3.0 * cell),
                        cell,
                        cell,
                        c,
                    );
                }
            }

            // Extraction card done -> ride to the next floor (13's car jams
            // into 13½ and its boss intro; the boss floor's car goes home:
            // the outro — uplink comms, blur-out — then the credits).
            if level_complete && self.level_complete_time >= EXTRACT_CARD_SECS {
                match self.extracting.and_then(level_index_for_floor_id) {
                    Some(next) => {
                        self.selected_level = next;
                        self.start_game();
                        return;
                    }
                    None => {
                        if self.outro.is_none() {
                            self.outro = Some(Outro::new());
                            // The thread home is back: the calm track.
                            self.audio.play_song(calmest_song_index());
                        }
                        let feed_idle = self
                            .scenario
                            .as_ref()
                            .map(|sc| !sc.comms.is_active(sc.time()))
                            .unwrap_or(true);
                        let done = self
                            .outro
                            .as_mut()
                            .map(|o| o.tick(dt, feed_idle))
                            .unwrap_or(false);
                        if let Some(t) = self.outro.and_then(|o| o.blur_t()) {
                            graphics.postfx(0, t, ending::BLUR_COLOR);
                        }
                        if done {
                            self.outro = None;
                            self.scenario = None;
                            self.extracting = None;
                            self.ending = Ending::new();
                            self.screen = GameScreen::Ending;
                            return;
                        }
                    }
                }
            }

            // Hold R while alive to restart the floor from scratch: a load
            // bar fills at the centre of the screen (drawn above); releasing
            // R before it fills cancels. Deliberately a full restart — the
            // player is asking for a clean slate, not the checkpoint.
            if player_alive && self.extracting.is_none() && input::is_key_down("r") {
                self.restart_hold += dt;
                if self.restart_hold >= RESTART_HOLD_SECS {
                    self.restart_hold = 0.0;
                    self.load_floor();
                    return;
                }
            } else {
                self.restart_hold = 0.0;
            }

            // Handle restart: death goes back to the latest `checkpoint`
            // snapshot when the floor set one, otherwise the floor restarts
            // from scratch (the death feedback — flash, sfx, WASTED card —
            // already played; R is the resume).
            if !player_alive && input::is_key_down("r") {
                if !self.restore_checkpoint() {
                    self.load_floor();
                }
                // Restart the music (it was stopped on death).
                self.audio.start_music();
            }

            // Handle escape to open pause menu
            if input::is_key_pressed("Escape") {
                self.selected_pause_option = PauseOption::Continue;
                self.screen = GameScreen::Paused;
            }
        }
    }

    #[wasm_bindgen]
    pub fn start() -> Result<(), JsValue> {
        // Setup input handlers
        input::setup_input_handlers()?;

        // Initialize graphics
        let graphics = Graphics::new()?;

        // Initialize game state
        let game_state = Rc::new(RefCell::new(GameState::new()));

        // Create game loop closure
        let f = Rc::new(RefCell::new(None));
        let g = f.clone();

        let window = web_sys::window().ok_or("No window")?;
        let performance = window.performance().ok_or("No performance")?;

        *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            let current_time = performance.now();
            // FPS CAP: skip the whole frame (no sim, no draw — dt simply
            // accumulates into the next rendered frame) when the display
            // outruns the configured cap. The 0.9 factor keeps a cap equal
            // to the refresh rate from beat-skipping.
            let run = {
                let state = game_state.borrow();
                state.fps_cap == 0
                    || current_time - state.last_frame_ms >= 1000.0 / state.fps_cap as f64 * 0.9
            };
            if run {
                perf::frame_start(current_time);
                let mut state = game_state.borrow_mut();
                state.last_frame_ms = current_time;
                state.update(&graphics, current_time);
                perf::frame_end();
            }

            // Schedule next frame
            request_animation_frame(f.borrow().as_ref().unwrap());
        }) as Box<dyn FnMut()>));

        request_animation_frame(g.borrow().as_ref().unwrap());

        Ok(())
    }

    fn request_animation_frame(f: &Closure<dyn FnMut()>) {
        web_sys::window()
            .unwrap()
            .request_animation_frame(f.as_ref().unchecked_ref())
            .expect("Failed to request animation frame");
    }
}
