// Integration tests for the complete game system
use open_miami::components::*;
use open_miami::ecs::*;
use open_miami::game::*;
use open_miami::math::Vec2;
use open_miami::systems::*;

#[test]
fn test_player_spawning() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::new(100.0, 200.0));

    assert_eq!(world.query::<Player>().len(), 1);
    assert!(is_player_alive(&world));
    assert_eq!(get_player_health(&world), 100);
}

#[test]
fn test_enemy_spawning() {
    let mut world = World::new();
    spawn_enemy(&mut world, Vec2::new(50.0, 50.0));
    spawn_enemy(&mut world, Vec2::new(100.0, 100.0));

    assert_eq!(world.query::<Enemy>().len(), 2);
}

#[test]
fn test_full_game_initialization() {
    let mut world = World::new();
    initialize_game(&mut world);

    assert_eq!(world.query::<Player>().len(), 1);
    assert_eq!(world.query::<Enemy>().len(), 4);
    assert!(is_player_alive(&world));
    assert_eq!(count_alive_enemies(&world), 4);
}

#[test]
fn test_movement_system_moves_entities() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));

    // Set velocity
    world.get_component_mut::<Velocity>(player).unwrap().x = 100.0;

    let mut movement_system = MovementSystem;
    movement_system.run(&mut world, 1.0); // 1 second

    let pos = world.get_component::<Position>(player).unwrap();
    assert_eq!(pos.x, 100.0);
}

#[test]
fn test_player_takes_damage_and_dies() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));

    assert!(is_player_alive(&world));

    // Deal fatal damage
    world
        .get_component_mut::<Health>(player)
        .unwrap()
        .take_damage(100);

    assert!(!is_player_alive(&world));
    assert_eq!(get_player_health(&world), 0);
}

#[test]
fn test_enemy_ai_chases_player() {
    let mut world = World::new();

    let _player = spawn_player(&mut world, Vec2::new(200.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(0.0, 0.0));

    let mut ai_system = AISystem;
    ai_system.run(&mut world, 0.016);

    // Enemy should be in chase state
    let ai = world.get_component::<AI>(enemy).unwrap();
    assert_eq!(ai.state, AIState::Chase);

    // Enemy should have velocity toward player
    let velocity = world.get_component::<Velocity>(enemy).unwrap();
    assert!(velocity.x > 0.0); // Moving right toward player
}

#[test]
fn test_enemy_ai_attacks_when_close() {
    let mut world = World::new();

    let _player = spawn_player(&mut world, Vec2::new(30.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(0.0, 0.0));

    let mut ai_system = AISystem;
    ai_system.run(&mut world, 0.016);

    // Enemy should be in attack state
    let ai = world.get_component::<AI>(enemy).unwrap();
    assert_eq!(ai.state, AIState::Attack);

    // Enemy should stop moving
    let velocity = world.get_component::<Velocity>(enemy).unwrap();
    assert_eq!(velocity.x, 0.0);
    assert_eq!(velocity.y, 0.0);
}

#[test]
fn test_enemy_ai_idle_when_far() {
    let mut world = World::new();

    let _player = spawn_player(&mut world, Vec2::new(500.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(0.0, 0.0));

    let mut ai_system = AISystem;
    ai_system.run(&mut world, 0.016);

    // Enemy should be idle
    let ai = world.get_component::<AI>(enemy).unwrap();
    assert_eq!(ai.state, AIState::Idle);
}

#[test]
fn test_shooting_hits_enemy() {
    let mut world = World::new();

    spawn_player(&mut world, Vec2::new(0.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(50.0, 0.0));

    let shooter_pos = Position::new(0.0, 0.0);
    let target_pos = Position::new(100.0, 0.0);

    let hit = CombatSystem::process_shoot(&mut world, shooter_pos, target_pos, 30);

    assert!(hit);
    let health = world.get_component::<Health>(enemy).unwrap();
    assert_eq!(health.current, 20); // 50 - 30 = 20
}

#[test]
fn test_shooting_kills_enemy() {
    let mut world = World::new();

    spawn_player(&mut world, Vec2::new(0.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(50.0, 0.0));

    let shooter_pos = Position::new(0.0, 0.0);
    let target_pos = Position::new(100.0, 0.0);

    // Shoot with high damage
    CombatSystem::process_shoot(&mut world, shooter_pos, target_pos, 100);

    let health = world.get_component::<Health>(enemy).unwrap();
    assert!(health.is_dead());
}

#[test]
fn test_enemy_attacks_player() {
    let mut world = World::new();

    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(30.0, 0.0));

    // Set enemy to attack state
    world.get_component_mut::<AI>(enemy).unwrap().state = AIState::Attack;

    let mut combat_system = CombatSystem;
    combat_system.run(&mut world, 0.016);

    let health = world.get_component::<Health>(player).unwrap();
    assert_eq!(health.current, 90); // 100 - 10 = 90
}

#[test]
fn test_complete_game_scenario_player_clears_room() {
    let mut world = World::new();
    initialize_game(&mut world);

    assert_eq!(count_alive_enemies(&world), 4);

    // Get all enemy positions
    let enemy_positions: Vec<Position> = world
        .query::<Enemy>()
        .iter()
        .filter_map(|&e| world.get_component::<Position>(e).copied())
        .collect();

    let player_pos = get_player_position(&world).unwrap();
    let shooter_pos = Position::from_vec2(player_pos);

    // Shoot all enemies
    for enemy_pos in enemy_positions {
        CombatSystem::process_shoot(&mut world, shooter_pos, enemy_pos, 100);
    }

    assert_eq!(count_alive_enemies(&world), 0);
    assert!(is_player_alive(&world));
}

#[test]
fn test_weapon_system_updates_cooldown() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));

    // Fire weapon
    world.get_component_mut::<Weapon>(player).unwrap().fire();

    let mut weapon_system = WeaponUpdateSystem;
    weapon_system.run(&mut world, 0.3);

    let weapon = world.get_component::<Weapon>(player).unwrap();
    assert!((weapon.fire_timer - 0.2).abs() < 0.001);
}

#[test]
fn test_melee_attack_hits_in_range() {
    let mut world = World::new();

    spawn_player(&mut world, Vec2::new(0.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(30.0, 0.0));

    let attacker_pos = Position::new(0.0, 0.0);
    let target_pos = Position::new(100.0, 0.0);

    let hit = CombatSystem::process_melee(&mut world, attacker_pos, target_pos, 50, 50.0);

    assert!(hit);
    let health = world.get_component::<Health>(enemy).unwrap();
    assert_eq!(health.current, 0); // 50 - 50 = 0
}

#[test]
fn test_multiple_systems_integration() {
    let mut world = World::new();
    initialize_game(&mut world);

    let mut movement_system = MovementSystem;
    let mut ai_system = AISystem;
    let mut weapon_system = WeaponUpdateSystem;
    let mut combat_system = CombatSystem;

    // Run systems for several frames
    for _ in 0..60 {
        weapon_system.run(&mut world, 0.016);
        ai_system.run(&mut world, 0.016);
        movement_system.run(&mut world, 0.016);
        combat_system.run(&mut world, 0.016);
    }

    // Game should still be in a valid state
    assert!(!world.query::<Player>().is_empty());
    assert!(!world.query::<Enemy>().is_empty());
}

#[test]
fn test_world_clear_and_reinitialize() {
    let mut world = World::new();
    initialize_game(&mut world);

    assert_eq!(world.query::<Player>().len(), 1);
    assert_eq!(world.query::<Enemy>().len(), 4);

    world.clear();

    assert_eq!(world.query::<Player>().len(), 0);
    assert_eq!(world.query::<Enemy>().len(), 0);

    initialize_game(&mut world);

    assert_eq!(world.query::<Player>().len(), 1);
    assert_eq!(world.query::<Enemy>().len(), 4);
    assert!(is_player_alive(&world));
}

#[test]
fn test_replay_simulation() {
    // Simulate a deterministic sequence of actions
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(100.0, 100.0));
    spawn_enemy(&mut world, Vec2::new(200.0, 100.0));

    // Record initial state
    let initial_health = get_player_health(&world);
    assert_eq!(initial_health, 100);

    // Simulate 1 second of movement to the right
    world.get_component_mut::<Velocity>(player).unwrap().x = 100.0;

    let mut movement_system = MovementSystem;
    for _ in 0..60 {
        movement_system.run(&mut world, 1.0 / 60.0);
    }

    let final_pos = world.get_component::<Position>(player).unwrap();
    assert!((final_pos.x - 200.0).abs() < 1.0); // Should be around 200.0
}

#[test]
fn test_weapon_ammo_depletion() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));

    let initial_ammo = world.get_component::<Weapon>(player).unwrap().ammo;

    // Fire until out of ammo
    for _ in 0..initial_ammo {
        world.get_component_mut::<Weapon>(player).unwrap().fire();
        world
            .get_component_mut::<Weapon>(player)
            .unwrap()
            .fire_timer = 0.0; // Reset cooldown
    }

    let weapon = world.get_component::<Weapon>(player).unwrap();
    assert_eq!(weapon.ammo, 0);
    assert!(!weapon.can_fire());
}
