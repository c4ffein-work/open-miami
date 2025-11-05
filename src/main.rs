use macroquad::prelude::*;

// Import game modules
use open_miami::ecs::World;
use open_miami::systems::*;
use open_miami::game::*;
use open_miami::render::*;
use open_miami::level::Level;
use open_miami::camera::Camera;

#[macroquad::main("Open Miami - ECS Edition")]
async fn main() {
    // Initialize ECS world
    let mut world = World::new();
    initialize_game(&mut world);

    // Create systems
    let mut movement_system = MovementSystem;
    let mut weapon_system = WeaponUpdateSystem;
    let mut ai_system = AISystem;
    let mut combat_system = CombatSystem;

    // Non-ECS components
    let level = Level::new();
    let mut camera = Camera::new();

    loop {
        clear_background(Color::from_rgba(20, 12, 28, 255));

        let dt = get_frame_time();

        // Get player state for UI and camera
        let player_alive = is_player_alive(&world);
        let player_pos = get_player_position(&world);

        // Update camera to follow player
        if let Some(pos) = player_pos {
            camera.follow_player(pos);
        }

        // Get mouse position in world coordinates
        let mouse_screen_pos: Vec2 = mouse_position().into();
        let mouse_world_pos = camera.screen_to_world(mouse_screen_pos);

        // Handle input (only if player is alive)
        if player_alive {
            InputSystem::update_player_rotation(&mut world, mouse_world_pos);
            InputSystem::update_player_movement(&mut world);
            InputSystem::handle_shoot_input(&mut world, mouse_world_pos);
            InputSystem::handle_weapon_switch(&mut world);
        }

        // Run game systems
        weapon_system.run(&mut world, dt);
        ai_system.run(&mut world, dt);
        movement_system.run(&mut world, dt);
        combat_system.run(&mut world, dt);

        // Apply camera transform for world rendering
        camera.apply();

        // Render level
        level.render();

        // Render all entities
        render_entities(&world);

        // Reset camera for UI rendering
        camera.reset();

        // Render UI
        let health = get_player_health(&world);
        let ammo = get_player_ammo(&world);
        let enemies_alive = count_alive_enemies(&world);
        render_ui(health, ammo, enemies_alive, player_alive);

        // Handle restart
        if !player_alive && is_key_pressed(KeyCode::R) {
            world.clear();
            initialize_game(&mut world);
        }

        next_frame().await;
    }
}
