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
    use crate::math::Color;
    use crate::render::*;
    use crate::systems::*;

    struct GameState {
        world: World,
        movement_system: MovementSystem,
        weapon_system: WeaponUpdateSystem,
        ai_system: AISystem,
        combat_system: CombatSystem,
        projectile_system: ProjectileTrailSystem,
        level: Level,
        camera: Camera,
        last_time: f64,
    }

    impl GameState {
        fn new() -> Self {
            let mut world = World::new();
            initialize_game(&mut world);

            GameState {
                world,
                movement_system: MovementSystem,
                weapon_system: WeaponUpdateSystem,
                ai_system: AISystem,
                combat_system: CombatSystem,
                projectile_system: ProjectileTrailSystem,
                level: Level::new(),
                camera: Camera::new(),
                last_time: 0.0,
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
            self.projectile_system.run(&mut self.world, dt);

            // Apply camera transform for world rendering
            self.camera.apply();

            // Render level
            self.level.render(graphics);

            // Render all entities
            render_entities(&self.world, graphics);

            // Reset camera for UI rendering
            self.camera.reset();

            // Render UI
            let health = get_player_health(&self.world);
            let ammo = get_player_ammo(&self.world);
            let enemies_alive = count_alive_enemies(&self.world);
            render_ui(graphics, health, ammo, enemies_alive, player_alive);

            // Handle restart
            if !player_alive && input::is_key_down("r") {
                self.world.clear();
                initialize_game(&mut self.world);
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
