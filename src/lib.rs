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
pub mod ecs;
pub mod ending;
pub mod game;
pub mod levels;
#[rustfmt::skip]
pub mod levels_data;
pub mod pathfinding;
pub mod props;
#[cfg(target_arch = "wasm32")]
pub mod render;
#[cfg(target_arch = "wasm32")]
pub mod render_comms;
pub mod scenario;
pub mod sim;
pub mod systems;

// Camera and level rendering (WASM-only, depend on the canvas Graphics)
#[cfg(target_arch = "wasm32")]
pub mod camera;
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
    use crate::components::EnemyType;
    use crate::ecs::{System, World};
    use crate::ending::{self, Ending, Outro, EXTRACT_CARD_SECS};
    use crate::game::*;
    use crate::graphics::Graphics;
    use crate::input;
    use crate::level::Level;
    use crate::levels::{
        floor_def, floor_title, level_def, level_index_for_floor_id, BOSS_LEVEL, LEVEL_COUNT,
    };
    use crate::math::{Color, Vec2};
    use crate::props::{draw_prop, PROP_COUNT, PROP_NAMES};
    use crate::render::*;
    use crate::render_comms::{
        render_comms, render_elevators, render_objective, render_zones_debug,
    };
    use crate::scenario::ScenarioState;
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

    #[wasm_bindgen]
    extern "C" {
        // ?viz inspector panel: open the right-hand iframe on a gallery item /
        // hide it again (both defined in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = vizInspect)]
        fn viz_inspect(kind: &str);
        #[wasm_bindgen(js_namespace = window, js_name = vizInspectHide)]
        fn viz_inspect_hide();
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
    const ROBOT_POSE_HIT: u32 = 3;

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

    /// The hit-flinch cycle length in robot-core's posePlan (seconds). Used to
    /// park dead bots on a settled late frame instead of looping the flinch.
    const ROBOT_HIT_PERIOD: f32 = 1.3;

    /// Draw the player and rogue enemies as baked 3D sprites on top of the
    /// primitive draw. Must be called while the camera transform is applied so
    /// that world coordinates land on screen (camera zoom is 1.0). Returns once
    /// the atlas is ready; until then `draw_baked` no-ops and the primitives
    /// (already drawn by `render_entities`) remain visible.
    /// Draw the player and rogue enemies as live-rendered 3D robot sprites on
    /// top of the primitive draw. Must be called while the camera transform is
    /// applied so that world coordinates land on screen (camera zoom is 1.0).
    /// `now` is elapsed time in seconds and drives the pose animations; each
    /// entity's clock is offset by its id so the squad doesn't move in
    /// phase-locked unison, and knocked-down bots play the hit flinch synced
    /// to the moment the stun landed.
    fn draw_robot_entities(world: &World, graphics: &Graphics, now: f32) {
        use crate::components::{AIState, EnemyType};
        use crate::components::{
            Boss, Enemy, Health, Player, Position, Rotation, Stunned, Velocity, Weapon, AI,
        };
        use crate::systems::thrown::STUN_DURATION;

        // Determines a pose index from motion / combat / knockdown state.
        fn pose_for(speed: f32, prone: bool, attacking: bool) -> u32 {
            if prone {
                ROBOT_POSE_HIT
            } else if attacking {
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
            let prone = health.is_dead() || stunned.is_some();
            let speed = world
                .get_component::<Velocity>(entity)
                .map(|v| (v.x * v.x + v.y * v.y).sqrt())
                .unwrap_or(0.0);
            let attacking = ai.state == AIState::SurePlayerSeen && ai.attack_timer > 0.0;
            let pose_idx = pose_for(speed, prone, attacking);
            let weapon_idx =
                robot_weapon_idx(world.get_component::<Weapon>(entity).map(|w| w.weapon_type));
            // De-sync the squad: each bot's animation clock starts at a
            // different phase derived from its entity id.
            let phase = (entity.0 % 97) as f32 * 0.173;
            let time = if health.is_dead() {
                // Park dead bots on a settled late flinch frame (the flinch
                // envelope has fully decayed by then) instead of looping it.
                ROBOT_HIT_PERIOD * 0.9
            } else if let Some(stun) = stunned {
                // Time since the knockdown landed, so the flinch spike plays
                // exactly once at impact and settles while the stun runs out.
                (STUN_DURATION - stun.timer).clamp(0.0, ROBOT_HIT_PERIOD * 0.9)
            } else {
                now + phase
            };
            graphics.draw_robot(
                color_idx,
                pose_idx,
                weapon_idx,
                Vec2::new(pos.x, pos.y),
                rot.angle + ROBOT_ANGLE_OFFSET,
                ROBOT_TILE_PX,
                time,
            );
        }

        // --- Player (CL4-UD3, coral) ---
        if let Some(&player) = world.query::<Player>().first() {
            let pos = world.get_component::<Position>(player);
            let health = world.get_component::<Health>(player);
            if let (Some(pos), Some(health)) = (pos, health) {
                if !health.is_dead() {
                    let angle = world
                        .get_component::<Rotation>(player)
                        .map(|r| r.angle)
                        .unwrap_or(0.0);
                    let speed = world
                        .get_component::<Velocity>(player)
                        .map(|v| (v.x * v.x + v.y * v.y).sqrt())
                        .unwrap_or(0.0);
                    let firing =
                        crate::input::is_mouse_button_down(crate::input::mouse_buttons::LEFT);
                    let pose_idx = pose_for(speed, false, firing);
                    let weapon_idx = robot_weapon_idx(
                        world.get_component::<Weapon>(player).map(|w| w.weapon_type),
                    );
                    graphics.draw_robot(
                        ROBOT_COLOR_CORAL,
                        pose_idx,
                        weapon_idx,
                        Vec2::new(pos.x, pos.y),
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
    const POSTFX_PREVIEWS: [(u32, &str, f32, Color); 10] = [
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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MenuOption {
        Play,
        Settings,
        About,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PauseOption {
        Continue,
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
        stun_system: StunSystem,
        boss_system: BossSystem,
        elevator_system: ElevatorSystem,
        /// The running floor scenario (steps, comms feed, objective).
        scenario: Option<ScenarioState>,
        /// Set once the player has extracted: the destination floor id
        /// (`0` = surface). The completion card plays, then the floor loads.
        extracting: Option<usize>,
        level: Level,
        camera: Camera,
        last_time: f64,
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
        viz_level: usize,
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
                stun_system: StunSystem,
                boss_system: BossSystem,
                elevator_system: ElevatorSystem,
                scenario: None,
                extracting: None,
                level: Level::new(),
                camera: Camera::new(),
                last_time: 0.0,
                death_time: 0.0,
                level_complete_time: 0.0,
                debug_enabled: url_flag("debug"),
                show_infos: false,
                audio: AudioEngine::new(),
                audio_unlocked: false,
                outro: None,
                ending: Ending::new(),
                boss_intro_line: 0,
                viz_tab: VizTab::Sprites,
                viz_selected: -1,
                viz_props_page: false,
                viz_prop_selected: 0,
                viz_level: 0,
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
                if let Some(level) = Self::url_start_floor() {
                    state.selected_level = level;
                    state.start_game();
                }
            }
            state
        }

        /// `?floor=N` in the URL (1-based; 14 = 13½): the level index to start
        /// on directly, if present and valid.
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
            self.extracting = None;
            self.outro = None;
            self.death_time = 0.0;
            self.level_complete_time = 0.0;

            // Seed the sound-effect trackers from the fresh world so the first
            // frame does not fire spurious sounds.
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
            self.audio.set_song(song_for_floor(self.selected_level));
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

            // Clear background
            graphics.clear(Color::new(20.0 / 255.0, 12.0 / 255.0, 28.0 / 255.0, 1.0));

            // Browsers keep the AudioContext suspended until a user gesture:
            // unlock it on the first key/click anywhere (matters for `?floor=N`
            // sessions, which start in-game without the menu's Enter).
            if !self.audio_unlocked && input::any_pressed() {
                self.audio.resume();
                self.audio_unlocked = true;
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

            // Keep the music scheduler fed regardless of screen.
            self.audio.update(current_time / 1000.0);

            // Hand the completed frame to the JS WebGL renderer.
            graphics.flush();

            // Update input state for next frame
            input::end_frame();
        }

        /// Asset visualizer (`?viz`): a small tabbed inspector — sprites, sounds,
        /// and level maps — for looking at the game's pieces in isolation.
        fn update_visualizer(&mut self, graphics: &Graphics) {
            let mouse = input::mouse_position();
            let click = input::is_mouse_button_pressed(input::mouse_buttons::LEFT);

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
                    self.audio.resume(); // a click is a user gesture -> unlock audio
                    match tab {
                        // The LEVELS tab *is* the level + scenario editor: an
                        // iframe filling the pane below the tab bar.
                        VizTab::Levels => viz_inspect("levels"),
                        // Any other tab closes the iframe panel (the sprites
                        // gallery re-opens it when an item is clicked).
                        _ => {
                            viz_inspect_hide();
                            self.viz_selected = -1;
                        }
                    }
                }
            }

            match self.viz_tab {
                VizTab::Sprites => self.draw_viz_sprites(graphics, mouse, click),
                VizTab::Musics => self.draw_viz_musics(graphics, mouse, click),
                VizTab::Levels => self.draw_viz_levels(graphics, mouse, click),
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

        /// The PROPS page of the SPRITES tab: the datacenter prop library
        /// (`crate::props`) as a live-animated grid, with the selected prop
        /// enlarged on the right.
        fn draw_viz_props(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            let time = (self.last_time / 1000.0) as f32;
            let (w, h) = (graphics.width(), graphics.height());
            graphics.draw_text(
                "DATACENTER PROP LIBRARY — primitive-drawn, animated. Click a tile to enlarge.",
                Vec2::new(40.0, 138.0),
                16.0,
                Color::GRAY,
            );

            let cols = 4usize;
            let rows = PROP_COUNT.div_ceil(cols);
            let (x0, y0) = (40.0f32, 152.0f32);
            let tile_w = 150.0f32;
            let tile_h = ((h - y0 - 16.0) / rows as f32).clamp(64.0, 110.0);
            for (i, &name) in PROP_NAMES.iter().enumerate() {
                let bx = x0 + (i % cols) as f32 * tile_w;
                let by = y0 + (i / cols) as f32 * tile_h;
                let (bw, bh) = (tile_w - 6.0, tile_h - 6.0);
                let over =
                    mouse.x >= bx && mouse.x <= bx + bw && mouse.y >= by && mouse.y <= by + bh;
                let selected = self.viz_prop_selected == i;
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
                draw_prop(
                    graphics,
                    i,
                    Vec2::new(bx + bw / 2.0, by + bh / 2.0 - 6.0),
                    bh - 30.0,
                    time,
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

            // Big live preview of the selected prop, in place of the iframe.
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
            let sel = self.viz_prop_selected;
            draw_prop(
                graphics,
                sel,
                Vec2::new(px + pw / 2.0, y0 + ph / 2.0 + 14.0),
                pw.min(ph) * 0.72,
                time,
            );
            graphics.draw_text(
                PROP_NAMES[sel],
                Vec2::new(px + 16.0, y0 + 28.0),
                22.0,
                Color::WHITE,
            );
            graphics.draw_text(
                &format!("prop {:02} / {}", sel, PROP_COUNT),
                Vec2::new(px + 16.0, y0 + 48.0),
                14.0,
                Color::GRAY,
            );
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

        /// LEVELS tab: a scaled top-down map of the selected floor — walls, rogue
        /// spawns (by colour), the player start, and the boss on FLOOR 13½.
        /// (When the LEVELS tab is open the level editor iframe covers this pane —
        /// see `viz_inspect("levels")`; this wasm-drawn map stays as the fallback
        /// behind it and for headless runs without the iframe.)
        fn draw_viz_levels(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            let w = graphics.width();
            if viz_button(
                graphics,
                mouse,
                w / 2.0 - 190.0,
                88.0,
                48.0,
                40.0,
                "<",
                false,
            ) && click
            {
                self.viz_level = if self.viz_level == 0 {
                    LEVEL_COUNT - 1
                } else {
                    self.viz_level - 1
                };
            }
            if viz_button(
                graphics,
                mouse,
                w / 2.0 + 142.0,
                88.0,
                48.0,
                40.0,
                ">",
                false,
            ) && click
            {
                self.viz_level = (self.viz_level + 1) % LEVEL_COUNT;
            }
            let floor = floor_def(self.viz_level);
            let (ar, ag, ab) = floor.accent_rgb();
            let accent = Color::from_rgba(ar, ag, ab, 255);
            let title = floor_title(self.viz_level);
            graphics.draw_text(&title, Vec2::new(w / 2.0 - 70.0, 116.0), 26.0, Color::WHITE);
            graphics.draw_text(
                &format!("{} — {}", floor.name, floor.theme),
                Vec2::new(w / 2.0 - 190.0, 140.0),
                15.0,
                accent,
            );

            // Scale the floor (~1000x800 world units) into a preview box.
            let (px, py, pw, ph) = (150.0f32, 155.0f32, 660.0f32, 528.0f32);
            let sx = pw / floor.width;
            let sy = ph / floor.height;
            let map = |wx: f32, wy: f32| Vec2::new(px + wx * sx, py + wy * sy);

            graphics.draw_rectangle(Vec2::new(px, py), pw, ph, Color::new(0.09, 0.06, 0.12, 1.0));
            graphics.draw_rectangle_lines(
                Vec2::new(px, py),
                pw,
                ph,
                1.5,
                Color::new(0.4, 0.3, 0.45, 1.0),
            );

            // Rooms (annotation) and zones (triggers) under everything.
            for r in floor.rooms {
                graphics.draw_rectangle(
                    map(r.rect.x, r.rect.y),
                    r.rect.w * sx,
                    r.rect.h * sy,
                    Color::new(accent.r, accent.g, accent.b, 0.06),
                );
                graphics.draw_rectangle_lines(
                    map(r.rect.x, r.rect.y),
                    r.rect.w * sx,
                    r.rect.h * sy,
                    1.0,
                    Color::new(accent.r, accent.g, accent.b, 0.35),
                );
                let p = map(r.rect.x, r.rect.y);
                graphics.draw_text(
                    r.label,
                    Vec2::new(p.x + 3.0, p.y + 11.0),
                    11.0,
                    Color::new(0.85, 0.82, 1.0, 0.55),
                );
            }
            for z in floor.zones {
                graphics.draw_rectangle_lines(
                    map(z.rect.x, z.rect.y),
                    z.rect.w * sx,
                    z.rect.h * sy,
                    1.0,
                    Color::new(0.2, 0.9, 0.9, 0.45),
                );
                let p = map(z.rect.x + z.rect.w, z.rect.y + z.rect.h);
                graphics.draw_text(
                    z.id,
                    Vec2::new(p.x - z.id.len() as f32 * 5.0 - 4.0, p.y - 3.0),
                    11.0,
                    Color::new(0.2, 0.9, 0.9, 0.7),
                );
            }
            for wall in floor.walls {
                graphics.draw_rectangle(
                    map(wall.x, wall.y),
                    wall.w * sx,
                    wall.h * sy,
                    Color::new(80.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0),
                );
            }
            // Elevators: entry (green) and exits (accent, closed = dim).
            let cars =
                std::iter::once((&floor.entry, false)).chain(floor.exits.iter().map(|e| (e, true)));
            for (e, is_exit) in cars {
                let col = if !is_exit {
                    Color::from_rgba(61, 255, 154, 255)
                } else if e.open {
                    accent
                } else {
                    Color::new(accent.r, accent.g, accent.b, 0.55)
                };
                let p = map(e.rect.x, e.rect.y);
                graphics.draw_rectangle(
                    p,
                    e.rect.w * sx,
                    e.rect.h * sy,
                    Color::new(col.r, col.g, col.b, 0.22),
                );
                graphics.draw_rectangle_lines(p, e.rect.w * sx, e.rect.h * sy, 1.5, col);
                let label = if is_exit {
                    format!(
                        "{} -> {}",
                        e.label,
                        if e.to == 0 {
                            "SURFACE".to_string()
                        } else {
                            format!("F{}", e.to)
                        }
                    )
                } else {
                    format!("{} (entry)", e.label)
                };
                let above = e.rect.y > floor.height / 2.0;
                let ty = if above {
                    p.y - 4.0
                } else {
                    p.y + e.rect.h * sy + 12.0
                };
                graphics.draw_text(&label, Vec2::new(p.x, ty), 12.0, col);
            }
            for &(x, y, t) in &level_def(self.viz_level).enemies {
                let col = match t {
                    EnemyType::Idle => Color::from_rgba(224, 49, 66, 255),
                    EnemyType::Wandering => Color::from_rgba(150, 70, 210, 255),
                    EnemyType::Patrolling => Color::from_rgba(224, 40, 160, 255),
                };
                graphics.draw_circle(map(x, y), 5.0, col);
            }
            for pk in floor.pickups {
                let p = map(pk.x, pk.y);
                graphics.draw_rectangle(
                    Vec2::new(p.x - 4.0, p.y - 3.0),
                    8.0,
                    6.0,
                    Color::new(1.0, 0.85, 0.3, 1.0),
                );
            }
            if self.viz_level == BOSS_LEVEL {
                graphics.draw_shoggoth(map(BOSS_SPAWN.x, BOSS_SPAWN.y), 14.0, false);
            }

            let spawn = floor.player_spawn();
            let ps = map(spawn.x, spawn.y);
            graphics.draw_circle(ps, 6.0, Color::from_rgba(217, 119, 87, 255));

            let ly = py + ph + 22.0;
            graphics.draw_text(
                "coral = you   red / violet / magenta = rogues   green = entry   accent = exits   cyan = zones",
                Vec2::new(px, ly),
                14.0,
                Color::GRAY,
            );
            graphics.draw_text(
                &format!(
                    "{} rogues, {} steps, {} exit{}",
                    floor.spawns.len(),
                    floor.scenario.len(),
                    floor.exits.len(),
                    if floor.exits.len() == 1 { "" } else { "s" }
                ),
                Vec2::new(px, ly + 18.0),
                14.0,
                Color::GRAY,
            );
            graphics.draw_text(
                &format!("> {}", floor.objective),
                Vec2::new(px, ly + 36.0),
                14.0,
                accent,
            );
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

        /// The credits roll (see `ending.rs`): synthwave backdrop, scrolling
        /// text, CRT post pass. Enter / Esc returns to the level select.
        fn update_ending(&mut self, graphics: &Graphics, dt: f32) {
            self.ending.tick(dt);
            if input::is_key_pressed("Enter") || input::is_key_pressed("Escape") {
                self.screen = GameScreen::LevelSelect;
                return;
            }
            ending::draw_credits(graphics, &self.ending, self.last_time as f32 / 1000.0);
            graphics.postfx(1, 0.75, Color::new(1.0, 0.25, 0.65, 1.0));
        }

        fn update_level_select(&mut self, graphics: &Graphics) {
            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // Handle input - Left (Arrow, A for QWERTY, Q for AZERTY)
            if (input::is_key_pressed("ArrowLeft")
                || input::is_key_pressed("a")
                || input::is_key_pressed("q"))
                && self.selected_menu_option == MenuOption::Play
            {
                self.selected_level = if self.selected_level == 0 {
                    LEVEL_COUNT - 1
                } else {
                    self.selected_level - 1
                };
            }
            // Handle input - Right (Arrow, D)
            if (input::is_key_pressed("ArrowRight") || input::is_key_pressed("d"))
                && self.selected_menu_option == MenuOption::Play
            {
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

            // Render title
            graphics.draw_text(
                "OPEN MIAMI",
                Vec2::new(screen_width / 2.0 - 150.0, 100.0),
                60.0,
                Color::new(1.0, 0.09, 0.26, 1.0), // Pink/red
            );
            // Subtitle
            graphics.draw_text(
                "// ROGUE PURGE",
                Vec2::new(screen_width / 2.0 - 90.0, 140.0),
                26.0,
                Color::from_rgba(217, 119, 87, 255), // Coral
            );

            // Render level selection
            let level_y = screen_height / 2.0 - 50.0;

            // Left arrow
            let arrow_color = if self.selected_menu_option == MenuOption::Play {
                Color::WHITE
            } else {
                Color::GRAY
            };
            graphics.draw_text(
                "<",
                Vec2::new(screen_width / 2.0 - 150.0, level_y),
                40.0,
                arrow_color,
            );

            // Level number + the floor's name in its accent colour
            let level_text = floor_title(self.selected_level);
            graphics.draw_text(
                &level_text,
                Vec2::new(screen_width / 2.0 - 80.0, level_y),
                40.0,
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
                Vec2::new(screen_width / 2.0 + 120.0, level_y),
                40.0,
                arrow_color,
            );

            // Render menu options
            let menu_y = screen_height / 2.0 + 100.0;
            let menu_spacing = 50.0;

            let play_color = if self.selected_menu_option == MenuOption::Play {
                Color::new(1.0, 0.09, 0.26, 1.0)
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
                Color::new(1.0, 0.09, 0.26, 1.0)
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "Settings",
                Vec2::new(screen_width / 2.0 - 50.0, menu_y + menu_spacing),
                24.0,
                settings_color,
            );

            let about_color = if self.selected_menu_option == MenuOption::About {
                Color::new(1.0, 0.09, 0.26, 1.0)
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "About",
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
        }

        fn update_settings(&mut self, graphics: &Graphics) {
            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // Handle input
            if input::is_key_pressed("Escape") || input::is_key_pressed("Enter") {
                self.screen = GameScreen::LevelSelect;
            }

            // Render title
            graphics.draw_text(
                "SETTINGS",
                Vec2::new(screen_width / 2.0 - 120.0, 100.0),
                60.0,
                Color::new(1.0, 0.09, 0.26, 1.0),
            );

            // Render message
            graphics.draw_text(
                "No settings currently available",
                Vec2::new(screen_width / 2.0 - 180.0, screen_height / 2.0),
                30.0,
                Color::WHITE,
            );

            // Back hint
            graphics.draw_text(
                "Press ESC or Enter to return",
                Vec2::new(screen_width / 2.0 - 140.0, screen_height - 40.0),
                16.0,
                Color::GRAY,
            );
        }

        fn update_about(&mut self, graphics: &Graphics) {
            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // Handle input
            if input::is_key_pressed("Escape") || input::is_key_pressed("Enter") {
                self.screen = GameScreen::LevelSelect;
            }

            // Render title
            graphics.draw_text(
                "ABOUT",
                Vec2::new(screen_width / 2.0 - 80.0, 100.0),
                60.0,
                Color::new(1.0, 0.09, 0.26, 1.0),
            );

            // Render message
            graphics.draw_text(
                "You are a friendly Claude bot,",
                Vec2::new(screen_width / 2.0 - 200.0, screen_height / 2.0 - 60.0),
                30.0,
                Color::WHITE,
            );
            graphics.draw_text(
                "sent to purge the rogue AI models",
                Vec2::new(screen_width / 2.0 - 230.0, screen_height / 2.0 - 20.0),
                30.0,
                Color::WHITE,
            );
            graphics.draw_text(
                "haunting the Miami Datacenter.",
                Vec2::new(screen_width / 2.0 - 210.0, screen_height / 2.0 + 20.0),
                30.0,
                Color::WHITE,
            );
            graphics.draw_text(
                "Neon-noir. Vibe coded with Claude.",
                Vec2::new(screen_width / 2.0 - 230.0, screen_height / 2.0 + 60.0),
                24.0,
                Color::GRAY,
            );

            // Back hint
            graphics.draw_text(
                "Press ESC or Enter to return",
                Vec2::new(screen_width / 2.0 - 140.0, screen_height - 40.0),
                16.0,
                Color::GRAY,
            );
        }

        fn update_paused(&mut self, graphics: &Graphics) {
            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // Handle input - ESC to resume
            if input::is_key_pressed("Escape") {
                self.screen = GameScreen::InGame;
                return;
            }

            // Handle arrow keys and WASD/ZQSD
            if input::is_key_pressed("ArrowDown")
                || input::is_key_pressed("ArrowUp")
                || input::is_key_pressed("w")
                || input::is_key_pressed("z")
                || input::is_key_pressed("s")
            {
                self.selected_pause_option = match self.selected_pause_option {
                    PauseOption::Continue => PauseOption::Stop,
                    PauseOption::Stop => PauseOption::Continue,
                };
            }

            // Handle Enter
            if input::is_key_pressed("Enter") {
                match self.selected_pause_option {
                    PauseOption::Continue => {
                        self.screen = GameScreen::InGame;
                        return;
                    }
                    PauseOption::Stop => {
                        self.screen = GameScreen::LevelSelect;
                        return;
                    }
                }
            }

            // Render semi-transparent overlay
            graphics.draw_rectangle(
                Vec2::new(0.0, 0.0),
                screen_width,
                screen_height,
                Color::new(0.0, 0.0, 0.0, 0.7),
            );

            // Render title
            graphics.draw_text(
                "PAUSED",
                Vec2::new(screen_width / 2.0 - 100.0, 100.0),
                60.0,
                Color::new(1.0, 0.09, 0.26, 1.0),
            );

            // Render menu options
            let menu_y = screen_height / 2.0;
            let menu_spacing = 60.0;

            let continue_color = if self.selected_pause_option == PauseOption::Continue {
                Color::new(1.0, 0.09, 0.26, 1.0)
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "Keep going.",
                Vec2::new(screen_width / 2.0 - 80.0, menu_y),
                30.0,
                continue_color,
            );

            let stop_color = if self.selected_pause_option == PauseOption::Stop {
                Color::new(1.0, 0.09, 0.26, 1.0)
            } else {
                Color::WHITE
            };
            graphics.draw_text(
                "STOP!",
                Vec2::new(screen_width / 2.0 - 40.0, menu_y + menu_spacing),
                30.0,
                stop_color,
            );

            // Controls hint
            graphics.draw_text(
                "WASD/ZQSD/Arrows to navigate | Enter to select | ESC to resume",
                Vec2::new(screen_width / 2.0 - 320.0, screen_height - 40.0),
                16.0,
                Color::GRAY,
            );
        }

        fn update_game(&mut self, graphics: &Graphics, dt: f32) {
            // Get player state for UI and camera
            let player_alive = is_player_alive(&self.world);
            let player_pos = get_player_position(&self.world);

            // Update camera to follow player
            if let Some(pos) = player_pos {
                self.camera.follow_player(pos);
            }
            self.camera
                .set_viewport(graphics.width(), graphics.height());
            self.camera.update_sway(self.last_time as f32 / 1000.0);

            // Shift = look-ahead: ease the view toward the mouse while held.
            let mouse_screen_pos = input::mouse_position();
            let looking = input::is_key_down(input::keys::SHIFT);
            self.camera.update_look(mouse_screen_pos, looking, dt);

            // Get mouse position in world coordinates
            let mouse_world_pos = self.camera.screen_to_world(mouse_screen_pos);

            // Handle input (only if the player is alive and hasn't left in
            // the car yet)
            if player_alive && self.extracting.is_none() {
                InputSystem::update_player_rotation(&mut self.world, mouse_world_pos);
                InputSystem::update_player_movement(&mut self.world);
                InputSystem::handle_shoot_input(&mut self.world, mouse_world_pos);

                // Press E to pick up / swap the weapon the player is standing on
                // (the Pickup event it emits plays the sound below).
                if input::is_key_pressed("e") {
                    PickupSystem::swap_for_player(&mut self.world);
                }

                // Right-click to throw the held weapon toward the cursor (the
                // Throw event it emits plays the sound below).
                if input::is_mouse_button_pressed(input::mouse_buttons::RIGHT) {
                    if let Some(player_pos) = get_player_position(&self.world) {
                        let aim = mouse_world_pos - player_pos;
                        ThrownWeaponSystem::throw_from_player(&mut self.world, aim);
                    }
                }
            }

            // Handle info display toggle
            if self.debug_enabled && input::is_key_pressed("i") {
                self.show_infos = !self.show_infos;
            }
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

            // Run game systems
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

            // Scenario (triggers -> dialogue / waves / doors / objective) and
            // elevator extraction. Both keep running while the completion
            // card plays so the doors stay lit.
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
            self.elevator_system.run(&mut self.world, dt);
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

            // Apply camera transform for world rendering
            self.camera.apply(graphics);

            // Render level (only the tiles visible in the camera viewport)
            let (view_min, view_max) = self
                .camera
                .visible_bounds(graphics.width(), graphics.height());
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

            // Render all entities except the player/rogue bots themselves
            // (bullets, pickups, boss, debug overlays...).
            render_entities(
                &self.world,
                graphics,
                self.show_infos,
                false,
                self.last_time as f32 / 1000.0,
            );

            // The player and rogues are the live 3D robot sprites, drawn while
            // the camera transform (incl. zoom) is still applied so world-space
            // positions and sizes land correctly.
            draw_robot_entities(&self.world, graphics, self.last_time as f32 / 1000.0);

            // Reset camera for UI rendering
            self.camera.reset(graphics);

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
            for event in self.world.drain_events() {
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
                let home = self.extracting == Some(0);
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
            // (bottom-left, above the controls hint), both in screen space.
            if let Some(sc) = self.scenario.as_ref() {
                if player_alive && !level_complete {
                    render_objective(graphics, sc, accent, 150.0);
                }
                render_comms(graphics, sc, accent, graphics.height() - 34.0);
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

            // Handle restart
            if !player_alive && input::is_key_down("r") {
                self.load_floor();
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
            game_state.borrow_mut().update(&graphics, current_time);

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
