use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use std::cell::RefCell;
use std::rc::Rc;

// Import game modules
use open_miami::camera::Camera;
use open_miami::ecs::World;
use open_miami::game::*;
use open_miami::graphics::Graphics;
use open_miami::input;
use open_miami::level::Level;
use open_miami::math::Color;
use open_miami::render::*;
use open_miami::systems::*;

struct GameState {
    world: World,
    movement_system: MovementSystem,
    weapon_system: WeaponUpdateSystem,
    ai_system: AISystem,
    combat_system: CombatSystem,
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
        graphics.clear(Color::new(20.0/255.0, 12.0/255.0, 28.0/255.0, 1.0));

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

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // Set panic hook for better error messages
    console_error_panic_hook::set_once();

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
