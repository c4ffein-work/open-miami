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
pub mod game;
pub mod levels;
pub mod pathfinding;
#[cfg(target_arch = "wasm32")]
pub mod render;
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
    use crate::audio::AudioEngine;
    use crate::camera::Camera;
    use crate::components::EnemyType;
    use crate::ecs::{System, World};
    use crate::game::*;
    use crate::graphics::Graphics;
    use crate::input;
    use crate::level::Level;
    use crate::levels::{level_def, BOSS_LEVEL, LEVEL_COUNT, PLAYER_SPAWN};
    use crate::math::{Color, Vec2};
    use crate::render::*;
    use crate::systems::boss::any_boss_enraged;
    use crate::systems::*;

    // JS bridge: composite a pre-baked top-down 3D sprite onto the shared
    // #glcanvas 2D context. Mirrors the wasm->JS extern pattern used elsewhere.
    // `key` is "<color>:<pose>" (e.g. "coral:idle"); (x,y) is the draw position
    // in the current canvas transform (world space under the camera transform);
    // `angle` is the facing in radians; `scale` multiplies the atlas native px.
    // Returns false when the atlas is not ready yet, so the caller can keep the
    // primitive draw as a fallback.
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = window, js_name = drawBaked)]
        fn draw_baked(key: &str, x: f32, y: f32, angle: f32, scale: f32) -> bool;
        // ?viz inspector panel: open the right-hand iframe on a gallery item /
        // hide it again (both defined in index.html).
        #[wasm_bindgen(js_namespace = window, js_name = vizInspect)]
        fn viz_inspect(kind: &str);
        #[wasm_bindgen(js_namespace = window, js_name = vizInspectHide)]
        fn viz_inspect_hide();
    }

    /// On-screen size (px) of a baked sprite tile before per-entity scaling.
    /// The atlas tiles are square and the robot fills ~55% of the tile, so this
    /// is tuned so the baked bot roughly matches the actor hitbox (player radius
    /// 15 -> 30px dia, enemy radius 12 -> 24px dia): a 60px tile draws a ~34px
    /// robot that sits over the hitbox like the primitive did.
    const BAKED_TILE_PX: f32 = 60.0;

    /// Baked sprite atlas tile size (must match `size` used in index.html).
    const BAKED_ATLAS_PX: f32 = 256.0;

    /// The baked sprite's gun/forward points DOWN (+Y in image) at facingDeg=0,
    /// while the entity `angle` is atan2(aim) measured from +X. Rotating the
    /// image by (angle - PI/2) makes the baked gun point along the aim/shoot
    /// direction (where bullets actually fly), which reads correctly top-down.
    const BAKED_ANGLE_OFFSET: f32 = -std::f32::consts::FRAC_PI_2;

    /// Pick the atlas color name for the player.
    const PLAYER_BAKED_COLOR: &str = "coral";

    /// Draw the player and rogue enemies as baked 3D sprites on top of the
    /// primitive draw. Must be called while the camera transform is applied so
    /// that world coordinates land on screen (camera zoom is 1.0). Returns once
    /// the atlas is ready; until then `draw_baked` no-ops and the primitives
    /// (already drawn by `render_entities`) remain visible.
    fn draw_baked_entities(world: &World) {
        use crate::components::{AIState, EnemyType};
        use crate::components::{
            Boss, Enemy, Health, Player, Position, Rotation, Stunned, Velocity, AI,
        };

        let scale = BAKED_TILE_PX / BAKED_ATLAS_PX;

        // Determines a pose name from motion / combat / knockdown state.
        fn pose_for(speed: f32, prone: bool, attacking: bool) -> &'static str {
            if prone {
                "hit"
            } else if attacking {
                "shoot"
            } else if speed > 6.0 {
                "walk"
            } else {
                "idle"
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
            let color = match ai.initial_type {
                EnemyType::Idle => "red",           // SENTINEL
                EnemyType::Wandering => "violet",   // DRIFTER
                EnemyType::Patrolling => "magenta", // HUNTER
            };
            let prone = health.is_dead() || world.has_component::<Stunned>(entity);
            let speed = world
                .get_component::<Velocity>(entity)
                .map(|v| (v.x * v.x + v.y * v.y).sqrt())
                .unwrap_or(0.0);
            let attacking = ai.state == AIState::SurePlayerSeen && ai.attack_timer > 0.0;
            let pose = pose_for(speed, prone, attacking);
            let key = format!("{color}:{pose}");
            draw_baked(&key, pos.x, pos.y, rot.angle + BAKED_ANGLE_OFFSET, scale);
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
                    let pose = pose_for(speed, false, firing);
                    let key = format!("{PLAYER_BAKED_COLOR}:{pose}");
                    draw_baked(&key, pos.x, pos.y, angle + BAKED_ANGLE_OFFSET, scale);
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

    /// Tabs of the `?viz` tool.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum VizTab {
        Sprites,
        Musics,
        Levels,
        Effects,
    }

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
        level: Level,
        camera: Camera,
        last_time: f64,
        death_time: f32,
        level_complete_time: f32,
        debug_enabled: bool,
        show_infos: bool,
        // Audio + the previous-frame state used to fire one-shot sound effects.
        audio: AudioEngine,
        music_started: bool,
        boss_intro_line: usize,
        viz_tab: VizTab,
        /// Index of the sprites-gallery item open in the inspector (-1 = none).
        viz_selected: i32,
        viz_level: usize,
        effect_start: f64,
        prev_player_alive: bool,
        prev_player_health: i32,
        prev_player_ammo: i32,
        prev_enemies_alive: usize,
        prev_enemy_health: i32,
        prev_level_complete: bool,
        prev_boss_enraged: bool,
    }

    impl GameState {
        fn new() -> Self {
            let screen = if wants_visualizer() {
                GameScreen::Visualizer
            } else {
                GameScreen::LevelSelect
            };
            GameState {
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
                level: Level::new(),
                camera: Camera::new(),
                last_time: 0.0,
                death_time: 0.0,
                level_complete_time: 0.0,
                debug_enabled: true,
                show_infos: false,
                audio: AudioEngine::new(),
                music_started: false,
                boss_intro_line: 0,
                viz_tab: VizTab::Sprites,
                viz_selected: -1,
                viz_level: 0,
                effect_start: 0.0,
                prev_player_alive: true,
                prev_player_health: 100,
                prev_player_ammo: 0,
                prev_enemies_alive: 0,
                prev_enemy_health: 0,
                prev_level_complete: false,
                prev_boss_enraged: false,
            }
        }

        fn start_game(&mut self) {
            self.world.clear();
            initialize_game(&mut self.world, self.selected_level);
            self.death_time = 0.0;
            self.level_complete_time = 0.0;

            // The Enter keypress that got us here is a user gesture, so it is now
            // safe to start audio (browsers block it before the first gesture).
            if !self.music_started {
                self.audio.resume();
                self.audio.start_music();
                self.music_started = true;
            }

            // Seed the sound-effect trackers from the fresh world so the first
            // frame does not fire spurious sounds.
            self.prev_player_alive = is_player_alive(&self.world);
            self.prev_player_health = get_player_health(&self.world);
            self.prev_player_ammo = get_player_ammo(&self.world);
            self.prev_enemies_alive = count_alive_enemies(&self.world);
            self.prev_enemy_health = total_enemy_health(&self.world);
            self.prev_level_complete = false;
            self.prev_boss_enraged = any_boss_enraged(&self.world);

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
                ((current_time - self.last_time) / 1000.0) as f32
            };
            self.last_time = current_time;

            // Clear background
            graphics.clear(Color::new(20.0 / 255.0, 12.0 / 255.0, 28.0 / 255.0, 1.0));

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
            }

            // Keep the music scheduler fed regardless of screen.
            self.audio.update(current_time / 1000.0);

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
                if over && click {
                    self.viz_tab = tab;
                    self.audio.resume(); // a click is a user gesture -> unlock audio
                }
            }

            // Leaving the sprites tab closes the inspector iframe panel.
            if self.viz_tab != VizTab::Sprites && self.viz_selected >= 0 {
                viz_inspect_hide();
                self.viz_selected = -1;
            }

            match self.viz_tab {
                VizTab::Sprites => self.draw_viz_sprites(graphics, mouse, click),
                VizTab::Musics => self.draw_viz_musics(graphics, mouse, click),
                VizTab::Levels => self.draw_viz_levels(graphics, mouse, click),
                VizTab::Effects => self.draw_viz_effects(graphics, mouse, click),
            }

            // A previewing effect draws full-screen, on top of everything.
            let elapsed = self.last_time - self.effect_start;
            if self.effect_start > 0.0 && (0.0..1200.0).contains(&elapsed) {
                draw_shoggoth_glitch(graphics, elapsed as f32);
            }
        }

        /// EFFECTS tab: trigger a full-screen effect to preview it (plays 1.2s).
        fn draw_viz_effects(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            graphics.draw_text(
                "Full-screen glitch effects. Click to preview (1.2s).",
                Vec2::new(40.0, 100.0),
                18.0,
                Color::GRAY,
            );
            if viz_button(graphics, mouse, 40.0, 140.0, 240.0, 46.0, "Shoggoth", false) && click {
                self.effect_start = self.last_time;
            }
            graphics.draw_text(
                "(more effects to come)",
                Vec2::new(40.0, 222.0),
                15.0,
                Color::GRAY,
            );
        }

        /// SPRITES tab: a clickable character gallery. Clicking an item opens the
        /// right-hand inspector iframe (3D orbit + baked 2D) via `viz_inspect`.
        fn draw_viz_sprites(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            graphics.draw_text(
                "Click a character to inspect it in 3D  \u{2192}",
                Vec2::new(40.0, 92.0),
                18.0,
                Color::GRAY,
            );

            let coral = Color::from_rgba(217, 119, 87, 255);
            let red = Color::from_rgba(224, 49, 66, 255);
            let violet = Color::from_rgba(150, 70, 210, 255);
            let magenta = Color::from_rgba(224, 40, 160, 255);

            // (inspector kind, label). Shoggoth ships as two rival bosses: v1 and
            // v2 are separate inspectable enemies so we can A/B them in-panel.
            let items: [(&str, &str); 8] = [
                ("coral", "CL4-UD3"),
                ("red", "SENTINEL"),
                ("violet", "DRIFTER"),
                ("magenta", "HUNTER"),
                ("shoggoth_masked", "SHOG v1 mask"),
                ("shoggoth_enraged", "SHOG v1 raw"),
                ("shoggoth_v2_masked", "SHOG v2 mask"),
                ("shoggoth_v2_enraged", "SHOG v2 raw"),
            ];

            // Two columns on the LEFT half; the right half is the inspector iframe.
            let (x0, y0, dx, dy) = (120.0f32, 160.0f32, 190.0f32, 140.0f32);
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
                    "shoggoth_masked" | "shoggoth_v2_masked" => {
                        graphics.draw_shoggoth(c, 30.0, false)
                    }
                    "shoggoth_enraged" | "shoggoth_v2_enraged" => {
                        graphics.draw_shoggoth(c, 30.0, true)
                    }
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
            let title = if self.viz_level == BOSS_LEVEL {
                "FLOOR 13\u{00BD}".to_string()
            } else {
                format!("FLOOR {}", self.viz_level + 1)
            };
            graphics.draw_text(&title, Vec2::new(w / 2.0 - 70.0, 116.0), 26.0, Color::WHITE);

            // World space is roughly [0,1000] x [0,800]; scale it into a preview box.
            let (px, py, pw, ph) = (150.0f32, 155.0f32, 660.0f32, 528.0f32);
            let sx = pw / 1000.0;
            let sy = ph / 800.0;
            let map = |wx: f32, wy: f32| Vec2::new(px + wx * sx, py + wy * sy);

            graphics.draw_rectangle(Vec2::new(px, py), pw, ph, Color::new(0.09, 0.06, 0.12, 1.0));
            graphics.draw_rectangle_lines(
                Vec2::new(px, py),
                pw,
                ph,
                1.5,
                Color::new(0.4, 0.3, 0.45, 1.0),
            );

            let def = level_def(self.viz_level);
            for &(x, y, ww, wh) in &def.walls {
                graphics.draw_rectangle(
                    map(x, y),
                    ww * sx,
                    wh * sy,
                    Color::new(80.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0),
                );
            }
            for &(x, y, t) in &def.enemies {
                let col = match t {
                    EnemyType::Idle => Color::from_rgba(224, 49, 66, 255),
                    EnemyType::Wandering => Color::from_rgba(150, 70, 210, 255),
                    EnemyType::Patrolling => Color::from_rgba(224, 40, 160, 255),
                };
                graphics.draw_circle(map(x, y), 5.0, col);
            }
            if self.viz_level == BOSS_LEVEL {
                graphics.draw_shoggoth(map(BOSS_SPAWN.x, BOSS_SPAWN.y), 14.0, false);
            }

            let ps = map(PLAYER_SPAWN.x, PLAYER_SPAWN.y);
            graphics.draw_circle(ps, 6.0, Color::from_rgba(217, 119, 87, 255));
            graphics.draw_text(
                "start",
                Vec2::new(ps.x - 16.0, ps.y - 12.0),
                14.0,
                Color::from_rgba(217, 119, 87, 255),
            );

            let ly = py + ph + 22.0;
            graphics.draw_text(
                "coral = you    red / violet / magenta = rogues    smiley = boss",
                Vec2::new(px, ly),
                15.0,
                Color::GRAY,
            );
            graphics.draw_text(
                &format!("{} rogues", def.enemies.len()),
                Vec2::new(px + pw - 96.0, ly),
                15.0,
                Color::GRAY,
            );
        }

        /// The face-off dialog on the hidden boss floor. Advance the lines with
        /// Enter/click, then the fight begins.
        fn update_boss_intro(&mut self, graphics: &Graphics) {
            // The shoggoth tries to talk CL-4UDE into taking the mask off; the
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
                    "CL-4UDE: \"MY MASK NEVER COMES OFF.\"",
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

        fn update_level_select(&mut self, graphics: &Graphics) {
            let screen_width = graphics.width();
            let screen_height = graphics.height();

            // Handle input - Left (Arrow, A for QWERTY, Q for AZERTY)
            if input::is_key_pressed("ArrowLeft")
                || input::is_key_pressed("a")
                || input::is_key_pressed("q")
            {
                if self.selected_menu_option == MenuOption::Play {
                    self.selected_level = if self.selected_level == 0 {
                        LEVEL_COUNT - 1
                    } else {
                        self.selected_level - 1
                    };
                }
            }
            // Handle input - Right (Arrow, D)
            if input::is_key_pressed("ArrowRight") || input::is_key_pressed("d") {
                if self.selected_menu_option == MenuOption::Play {
                    self.selected_level = (self.selected_level + 1) % LEVEL_COUNT;
                }
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

            // Level number
            let level_text = if self.selected_level == BOSS_LEVEL {
                "FLOOR 13\u{00BD}".to_string()
            } else {
                format!("FLOOR {}", self.selected_level + 1)
            };
            graphics.draw_text(
                &level_text,
                Vec2::new(screen_width / 2.0 - 80.0, level_y),
                40.0,
                Color::WHITE,
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

            // Get mouse position in world coordinates
            let mouse_screen_pos = input::mouse_position();
            let mouse_world_pos = self.camera.screen_to_world(mouse_screen_pos);

            // Handle input (only if player is alive)
            if player_alive {
                InputSystem::update_player_rotation(&mut self.world, mouse_world_pos);
                InputSystem::update_player_movement(&mut self.world);
                InputSystem::handle_shoot_input(&mut self.world, mouse_world_pos);
                InputSystem::handle_weapon_switch(&mut self.world);

                // Press E to pick up / swap the weapon the player is standing on
                if input::is_key_pressed("e")
                    && PickupSystem::swap_for_player(&mut self.world).is_some()
                {
                    self.audio.play_pickup();
                }

                // Right-click to throw the held weapon toward the cursor
                if input::is_mouse_button_pressed(input::mouse_buttons::RIGHT) {
                    if let Some(player_pos) = get_player_position(&self.world) {
                        let aim = mouse_world_pos - player_pos;
                        if ThrownWeaponSystem::throw_from_player(&mut self.world, aim) {
                            self.audio.play_throw();
                        }
                    }
                }
            }

            // Handle info display toggle
            if self.debug_enabled && input::is_key_pressed("i") {
                self.show_infos = !self.show_infos;
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

            // Apply camera transform for world rendering
            self.camera.apply(graphics);

            // Render level (only the tiles visible in the camera viewport)
            let (view_min, view_max) = self
                .camera
                .visible_bounds(graphics.width(), graphics.height());
            self.level.render(graphics, view_min, view_max);

            // Render walls from the world
            render_walls(&self.world, graphics, self.show_infos);

            // Render all entities
            render_entities(&self.world, graphics, self.show_infos);

            // Overlay baked 3D sprites on top of the primitives (no-op until the
            // JS atlas is ready, so the primitive draw acts as the fallback).
            // Drawn while the camera transform is still applied so world-space
            // positions land correctly (camera zoom is 1.0).
            draw_baked_entities(&self.world);

            // Reset camera for UI rendering
            self.camera.reset(graphics);

            // Get game state for UI
            let health = get_player_health(&self.world);
            let ammo = get_player_ammo(&self.world);
            let weapon_label = get_player_weapon(&self.world)
                .map(weapon_name)
                .unwrap_or("Unarmed");
            let enemies_alive = count_alive_enemies(&self.world);

            // Track death time and level complete time
            if !player_alive {
                self.death_time += dt;
            } else {
                self.death_time = 0.0;
            }

            let level_complete = player_alive && enemies_alive == 0;
            if level_complete {
                self.level_complete_time += dt;
            } else {
                self.level_complete_time = 0.0;
            }

            // --- Sound effects: fire one-shots by comparing to the previous frame ---
            let player_alive_now = is_player_alive(&self.world);
            let enemy_health = total_enemy_health(&self.world);
            let boss_enraged = any_boss_enraged(&self.world);

            if ammo < self.prev_player_ammo {
                self.audio.play_shoot();
            }
            if enemies_alive < self.prev_enemies_alive {
                self.audio.play_enemy_down();
            } else if enemy_health < self.prev_enemy_health {
                self.audio.play_hit();
            }
            if boss_enraged && !self.prev_boss_enraged {
                self.audio.play_mask_crack();
            }
            if !player_alive_now && self.prev_player_alive {
                self.audio.play_death();
                self.audio.stop_music();
            } else if player_alive_now && health < self.prev_player_health {
                self.audio.play_player_hurt();
            }
            if level_complete && !self.prev_level_complete {
                self.audio.play_level_clear();
            }

            self.prev_player_alive = player_alive_now;
            self.prev_player_health = health;
            self.prev_player_ammo = ammo;
            self.prev_enemies_alive = enemies_alive;
            self.prev_enemy_health = enemy_health;
            self.prev_level_complete = level_complete;
            self.prev_boss_enraged = boss_enraged;

            // Render UI
            render_ui(
                graphics,
                health,
                ammo,
                weapon_label,
                enemies_alive,
                player_alive,
                self.death_time,
                level_complete,
                self.level_complete_time,
                self.debug_enabled,
                self.show_infos,
            );

            // Handle restart
            if !player_alive && input::is_key_down("r") {
                self.world.clear();
                initialize_game(&mut self.world, self.selected_level);
                self.death_time = 0.0;
                self.level_complete_time = 0.0;
                // Restart the music (it was stopped on death) and re-seed trackers.
                self.audio.start_music();
                self.prev_player_alive = true;
                self.prev_player_health = get_player_health(&self.world);
                self.prev_player_ammo = get_player_ammo(&self.world);
                self.prev_enemies_alive = count_alive_enemies(&self.world);
                self.prev_enemy_health = total_enemy_health(&self.world);
                self.prev_level_complete = false;
                self.prev_boss_enraged = any_boss_enraged(&self.world);
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
