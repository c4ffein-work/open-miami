// Core modules
pub mod math;

// WASM-only modules for browser integration
#[cfg(target_arch = "wasm32")]
pub mod graphics;
#[cfg(target_arch = "wasm32")]
pub mod input;

// Library module for game logic (enables testing)
pub mod collision;
pub mod components;
pub mod ecs;
pub mod game;
pub mod pathfinding;
#[cfg(target_arch = "wasm32")]
pub mod render;
pub mod systems;

// Keep old modules for camera and level (WASM-only)
#[cfg(target_arch = "wasm32")]
pub mod camera;
#[cfg(target_arch = "wasm32")]
pub mod level;

// Old modules (deprecated but kept for reference)
// These are not exported to avoid dead code warnings
// Only compile for WASM as they use macroquad
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod enemy;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod player;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod weapon;

// WASM entry point - browser game initialization and main loop
#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    // Import game modules
    use crate::camera::Camera;
    use crate::ecs::{System, World};
    use crate::game::*;
    use crate::graphics::Graphics;
    use crate::input;
    use crate::level::Level;
    use crate::math::{Color, Vec2};
    use crate::render::*;
    use crate::systems::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GameScreen {
        LevelSelect,
        InGame,
        Paused,
        Settings,
        About,
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
        level: Level,
        camera: Camera,
        last_time: f64,
        death_time: f32,
        level_complete_time: f32,
    }

    impl GameState {
        fn new() -> Self {
            GameState {
                screen: GameScreen::LevelSelect,
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
                level: Level::new(),
                camera: Camera::new(),
                last_time: 0.0,
                death_time: 0.0,
                level_complete_time: 0.0,
            }
        }

        fn start_game(&mut self) {
            self.world.clear();
            initialize_game(&mut self.world, self.selected_level);
            self.screen = GameScreen::InGame;
            self.death_time = 0.0;
            self.level_complete_time = 0.0;
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
            }

            // Update input state for next frame
            input::end_frame();
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
                        11
                    } else {
                        self.selected_level - 1
                    };
                }
            }
            // Handle input - Right (Arrow, D)
            if input::is_key_pressed("ArrowRight") || input::is_key_pressed("d") {
                if self.selected_menu_option == MenuOption::Play {
                    self.selected_level = (self.selected_level + 1) % 12;
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
            let level_text = format!("LEVEL {}", self.selected_level + 1);
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
                "This is Open Miami,",
                Vec2::new(screen_width / 2.0 - 140.0, screen_height / 2.0 - 40.0),
                30.0,
                Color::WHITE,
            );
            graphics.draw_text(
                "a game heavily inspired by Hotline Miami",
                Vec2::new(screen_width / 2.0 - 260.0, screen_height / 2.0),
                30.0,
                Color::WHITE,
            );
            graphics.draw_text(
                "and vibe coded with Claude.",
                Vec2::new(screen_width / 2.0 - 200.0, screen_height / 2.0 + 40.0),
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
            }

            // Run game systems
            self.weapon_system.run(&mut self.world, dt);
            self.ai_system.run(&mut self.world, dt);
            self.movement_system.run(&mut self.world, dt);
            self.combat_system.run(&mut self.world, dt);
            self.bullet_system.run(&mut self.world, dt);
            self.projectile_system.run(&mut self.world, dt);

            // Apply camera transform for world rendering
            self.camera.apply(graphics);

            // Render level
            self.level.render(graphics);

            // Render walls from the world
            render_walls(&self.world, graphics);

            // Render all entities
            render_entities(&self.world, graphics);

            // Reset camera for UI rendering
            self.camera.reset(graphics);

            // Get game state for UI
            let health = get_player_health(&self.world);
            let ammo = get_player_ammo(&self.world);
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

            // Render UI
            render_ui(
                graphics,
                health,
                ammo,
                enemies_alive,
                player_alive,
                self.death_time,
                level_complete,
                self.level_complete_time,
            );

            // Handle restart
            if !player_alive && input::is_key_down("r") {
                self.world.clear();
                initialize_game(&mut self.world, self.selected_level);
                self.death_time = 0.0;
                self.level_complete_time = 0.0;
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
