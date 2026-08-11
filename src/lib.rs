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

    /// Full-screen "shoggoth" glitch: dark-grey cells with yellow eyes scattered
    /// across the screen, the pattern jittering to nearby spots every 0.2s, over
    /// a 1.2s fade-in/out. `elapsed_ms` is time since the effect started.
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
            Color::new(0.05, 0.05, 0.06, 0.82 * env),
        );

        // Regenerate the cell layout every 0.2s, jittering each cell near its anchor.
        let tick = (elapsed_ms / 200.0) as u32;
        for i in 0..64u32 {
            let bx = rand01(i, 11) * w;
            let by = rand01(i, 23) * h;
            let jx = (rand01(i, tick * 3 + 1) - 0.5) * 34.0;
            let jy = (rand01(i, tick * 3 + 2) - 0.5) * 34.0;
            let (x, y) = (bx + jx, by + jy);
            let sz = 26.0 + rand01(i, 7) * 34.0;
            g.draw_rectangle(
                Vec2::new(x - sz / 2.0, y - sz / 2.0),
                sz,
                sz,
                Color::new(0.22, 0.22, 0.24, 0.9 * env),
            );
            if rand01(i, tick + 5) > 0.35 {
                let eye = Color::new(1.0, 0.86, 0.12, env);
                let eo = sz * 0.18;
                g.draw_circle(Vec2::new(x - eo, y), sz * 0.09, eye);
                g.draw_circle(Vec2::new(x + eo, y), sz * 0.09, eye);
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
                ai_system: AISystem,
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

            match self.viz_tab {
                VizTab::Sprites => self.draw_viz_sprites(graphics),
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

        /// SPRITES tab: every sprite/asset drawn once and labelled.
        fn draw_viz_sprites(&self, graphics: &Graphics) {
            let coral = Color::from_rgba(217, 119, 87, 255);
            let red = Color::from_rgba(224, 49, 66, 255);
            let violet = Color::from_rgba(150, 70, 210, 255);
            let magenta = Color::from_rgba(224, 40, 160, 255);

            let (x0, y0, dx, dy) = (130.0f32, 180.0f32, 175.0f32, 165.0f32);
            let cell = |i: usize| Vec2::new(x0 + (i % 5) as f32 * dx, y0 + (i / 5) as f32 * dy);
            let label = |c: Vec2, t: &str| {
                graphics.draw_text(t, Vec2::new(c.x - 60.0, c.y + 60.0), 16.0, Color::WHITE);
            };

            // Actors (drawn facing "up" at rotation 0).
            let sprites = [
                (coral, false, "CL-4UDE"),
                (coral, true, "CL-4UDE down"),
                (red, false, "SENTINEL"),
                (violet, false, "DRIFTER"),
                (magenta, false, "HUNTER"),
                (red, true, "rogue down"),
            ];
            let mut i = 0;
            for (color, dead, name) in sprites {
                let c = cell(i);
                graphics.draw_pixelated_sprite(c, 0.0, color, dead);
                label(c, name);
                i += 1;
            }

            // Weapon pickups (same markers as on the floor).
            let pickups = [
                ("Pistol", Color::new(0.9, 0.9, 0.9, 1.0)),
                ("Shotgun", Color::new(1.0, 0.55, 0.1, 1.0)),
                ("MachineGun", Color::new(0.2, 0.8, 1.0, 1.0)),
                ("Melee", Color::new(0.7, 0.7, 0.75, 1.0)),
            ];
            for (name, col) in pickups {
                let c = cell(i);
                graphics.draw_rectangle(
                    Vec2::new(c.x - 11.0, c.y - 7.0),
                    22.0,
                    14.0,
                    Color::new(0.0, 0.0, 0.0, 0.35),
                );
                graphics.draw_rectangle(Vec2::new(c.x - 8.0, c.y - 2.0), 16.0, 4.0, col);
                graphics.draw_rectangle(Vec2::new(c.x - 6.0, c.y + 2.0), 4.0, 4.0, col);
                graphics.draw_rectangle_lines(
                    Vec2::new(c.x - 11.0, c.y - 7.0),
                    22.0,
                    14.0,
                    1.0,
                    col,
                );
                label(c, name);
                i += 1;
            }

            // Bullet.
            let c = cell(i);
            graphics.draw_circle(c, 3.0, Color::new(1.0, 0.9, 0.3, 1.0));
            label(c, "Bullet");
            i += 1;

            // Wall.
            let c = cell(i);
            graphics.draw_rectangle(
                Vec2::new(c.x - 24.0, c.y - 16.0),
                48.0,
                32.0,
                Color::new(80.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0),
            );
            graphics.draw_rectangle_lines(
                Vec2::new(c.x - 24.0, c.y - 16.0),
                48.0,
                32.0,
                2.0,
                Color::new(100.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0),
            );
            label(c, "Wall");
            i += 1;

            // Floor tiles (the two checker shades).
            let c = cell(i);
            let a = Color::new(40.0 / 255.0, 35.0 / 255.0, 45.0 / 255.0, 1.0);
            let b = Color::new(35.0 / 255.0, 30.0 / 255.0, 40.0 / 255.0, 1.0);
            graphics.draw_rectangle(Vec2::new(c.x - 20.0, c.y - 20.0), 20.0, 20.0, a);
            graphics.draw_rectangle(Vec2::new(c.x, c.y - 20.0), 20.0, 20.0, b);
            graphics.draw_rectangle(Vec2::new(c.x - 20.0, c.y), 20.0, 20.0, b);
            graphics.draw_rectangle(Vec2::new(c.x, c.y), 20.0, 20.0, a);
            label(c, "Floor");
            i += 1;

            // The boss, both phases.
            let c = cell(i);
            graphics.draw_shoggoth(c, 32.0, false);
            label(c, "SHOGGOTH mask");
            i += 1;
            let c = cell(i);
            graphics.draw_shoggoth(c, 32.0, true);
            label(c, "SHOGGOTH raw");
        }

        /// MUSICS tab: a button per sound + the music loop. Clicking plays it.
        fn draw_viz_musics(&mut self, graphics: &Graphics, mouse: Vec2, click: bool) {
            graphics.draw_text(
                "Click a button to play it. (audio unlocks on the first click)",
                Vec2::new(40.0, 100.0),
                18.0,
                Color::GRAY,
            );
            let items = [
                "Music: START",
                "Music: STOP",
                "Shoot",
                "Hit",
                "Rogue down",
                "Pickup",
                "Throw",
                "Player hurt",
                "Death",
                "Level clear",
                "Mask crack",
            ];
            for (i, &name) in items.iter().enumerate() {
                let x = 40.0 + (i % 2) as f32 * 270.0;
                let y = 140.0 + (i / 2) as f32 * 62.0;
                if viz_button(graphics, mouse, x, y, 240.0, 46.0, name, false) && click {
                    self.audio.resume();
                    match i {
                        0 => self.audio.start_music(),
                        1 => self.audio.stop_music(),
                        2 => self.audio.play_shoot(),
                        3 => self.audio.play_hit(),
                        4 => self.audio.play_enemy_down(),
                        5 => self.audio.play_pickup(),
                        6 => self.audio.play_throw(),
                        7 => self.audio.play_player_hurt(),
                        8 => self.audio.play_death(),
                        9 => self.audio.play_level_clear(),
                        _ => self.audio.play_mask_crack(),
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
            render_walls(&self.world, graphics);

            // Render all entities
            render_entities(&self.world, graphics, self.show_infos);

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
