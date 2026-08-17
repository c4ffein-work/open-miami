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
    // Level index 1 = FLOOR 1 (4 rogues); index 0 is the passive cold open.
    let mut world = World::new();
    initialize_game(&mut world, 1);

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

    let mut ai_system = AISystem::default();
    // Run multiple frames for state transition (need > 0.3s)
    for _ in 0..30 {
        ai_system.run(&mut world, 0.016);
    }

    // Enemy should be in chase state
    let ai = world.get_component::<AI>(enemy).unwrap();
    assert_eq!(ai.state, AIState::SurePlayerSeen);

    // Enemy should have velocity toward player
    let velocity = world.get_component::<Velocity>(enemy).unwrap();
    assert!(velocity.x > 0.0); // Moving right toward player
}

#[test]
fn test_enemy_ai_attacks_when_close() {
    let mut world = World::new();

    let _player = spawn_player(&mut world, Vec2::new(30.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(0.0, 0.0));

    let mut ai_system = AISystem::default();
    // Run multiple frames for state transition
    for _ in 0..30 {
        ai_system.run(&mut world, 0.016);
    }

    // Enemy should be in attack state
    let ai = world.get_component::<AI>(enemy).unwrap();
    assert_eq!(ai.state, AIState::SurePlayerSeen);

    // Enemy should stop moving
    let velocity = world.get_component::<Velocity>(enemy).unwrap();
    assert_eq!(velocity.x, 0.0);
    assert_eq!(velocity.y, 0.0);
}

#[test]
fn test_enemy_ai_idle_when_far() {
    let mut world = World::new();

    let _player = spawn_player(&mut world, Vec2::new(1000.0, 0.0)); // Beyond 900 detection range
    let enemy = spawn_enemy(&mut world, Vec2::new(0.0, 0.0));

    let mut ai_system = AISystem::default();
    ai_system.run(&mut world, 0.016);

    // Enemy should be idle
    let ai = world.get_component::<AI>(enemy).unwrap();
    assert_eq!(ai.state, AIState::Unaware);
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
    world.get_component_mut::<AI>(enemy).unwrap().state = AIState::SurePlayerSeen;

    let mut combat_system = CombatSystem;
    combat_system.run(&mut world, 0.016);

    let health = world.get_component::<Health>(player).unwrap();
    assert_eq!(
        health.current,
        100 - open_miami::systems::combat::ENEMY_ATTACK_DAMAGE
    );
}

#[test]
fn test_complete_game_scenario_player_clears_room() {
    let mut world = World::new();
    initialize_game(&mut world, 1);

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
    initialize_game(&mut world, 0);

    let mut movement_system = MovementSystem;
    let mut ai_system = AISystem::default();
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
    initialize_game(&mut world, 1);

    assert_eq!(world.query::<Player>().len(), 1);
    assert_eq!(world.query::<Enemy>().len(), 4);

    world.clear();

    assert_eq!(world.query::<Player>().len(), 0);
    assert_eq!(world.query::<Enemy>().len(), 0);

    initialize_game(&mut world, 1);

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

#[test]
fn test_player_blocked_by_wall() {
    let mut world = World::new();

    // Player starts at (100, 100)
    let player = spawn_player(&mut world, Vec2::new(100.0, 100.0));

    // Add a vertical wall at x=150 that blocks right movement
    // Wall is at (150, 50) with width 20 and height 100
    // This creates a barrier from y=50 to y=150
    world.add_wall(150.0, 50.0, 20.0, 100.0);

    // Try to move right - player should hit the wall
    // Player radius is 15, so they should stop at x = wall_x - player_radius = 150 - 15 = 135
    world.get_component_mut::<Velocity>(player).unwrap().x = 100.0; // Move right
    world.get_component_mut::<Velocity>(player).unwrap().y = 0.0;

    let mut movement_system = MovementSystem;
    movement_system.run(&mut world, 0.5); // Move for 0.5 seconds

    // Player should be blocked by the wall, not at expected x=150
    let pos = world.get_component::<Position>(player).unwrap();
    assert!(
        pos.x < 150.0,
        "Player should be stopped by wall, but is at x={}",
        pos.x
    );
    assert!(
        pos.x >= 130.0,
        "Player should be close to wall at x={}",
        pos.x
    ); // Close to wall

    // Now move down to get below the wall (wall ends at y=150)
    world.get_component_mut::<Velocity>(player).unwrap().x = 0.0;
    world.get_component_mut::<Velocity>(player).unwrap().y = 100.0; // Move down

    movement_system.run(&mut world, 0.6); // Move down for 0.6 seconds (60 units)

    let pos = world.get_component::<Position>(player).unwrap();
    assert!(
        pos.y > 150.0,
        "Player should have moved past the wall vertically"
    );

    // Now we can move right again past where the wall was
    world.get_component_mut::<Velocity>(player).unwrap().x = 100.0; // Move right
    world.get_component_mut::<Velocity>(player).unwrap().y = 0.0;

    movement_system.run(&mut world, 0.5); // Move right for 0.5 seconds

    let pos = world.get_component::<Position>(player).unwrap();
    // Now player should be able to move past x=150 since they're below the wall
    assert!(
        pos.x > 150.0,
        "Player should be able to move right past the wall at x={}",
        pos.x
    );
}

// --- Hotline Miami weapon rules: one weapon in hand, the ammo lives on it ---

/// A downed rogue's weapon hits the floor with a partial magazine; the player
/// swaps their worn pistol for it in place; the pistol keeps its rounds.
#[test]
fn test_ammo_travels_with_the_weapon_through_drop_and_swap() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(100.0, 100.0));
    world.get_component_mut::<Weapon>(player).unwrap().ammo = 4;
    let rogue = spawn_enemy_with_type(&mut world, Vec2::new(105.0, 100.0), EnemyType::Patrolling);
    assert_eq!(
        world.get_component::<Weapon>(rogue).unwrap().weapon_type,
        WeaponType::Shotgun
    );
    world
        .get_component_mut::<Health>(rogue)
        .unwrap()
        .take_damage(999);

    let mut pickup = PickupSystem;
    pickup.run(&mut world, 1.0 / 60.0);
    let events = world.drain_events();
    assert_eq!(events, vec![GameEvent::EnemyDown]);

    let dropped = world.query::<WeaponPickup>();
    assert_eq!(dropped.len(), 1);
    let drop = *world.get_component::<WeaponPickup>(dropped[0]).unwrap();
    assert_eq!(drop.weapon_type, WeaponType::Shotgun);
    assert!(drop.ammo >= 1 && drop.ammo <= WeaponType::Shotgun.magazine());

    // E: take the shotgun with exactly its rounds; leave the 4-round pistol.
    assert_eq!(
        PickupSystem::swap_for_player(&mut world),
        Some(WeaponType::Shotgun)
    );
    let held = *world.get_component::<Weapon>(player).unwrap();
    assert_eq!(
        (held.weapon_type, held.ammo),
        (WeaponType::Shotgun, drop.ammo)
    );
    let floor = *world.get_component::<WeaponPickup>(dropped[0]).unwrap();
    assert_eq!((floor.weapon_type, floor.ammo), (WeaponType::Pistol, 4));
    assert_eq!(world.drain_events(), vec![GameEvent::Pickup]);
    // Nothing granted a second pickup or a refill.
    assert_eq!(world.query::<WeaponPickup>().len(), 1);
}

/// Throwing carries the ammo along: the weapon lands as a pickup with the same
/// rounds, and picking it back up restores exactly that weapon.
#[test]
fn test_thrown_weapon_keeps_its_ammo_and_can_be_retrieved() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
    // Fire twice, resetting the cooldown in between: 12 -> 10.
    fire_player_weapon(&mut world, Vec2::new(500.0, 0.0));
    world
        .get_component_mut::<Weapon>(player)
        .unwrap()
        .fire_timer = 0.0;
    fire_player_weapon(&mut world, Vec2::new(500.0, 0.0));
    assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 10);
    assert_eq!(
        world.drain_events(),
        vec![
            GameEvent::PlayerFired(WeaponType::Pistol),
            GameEvent::PlayerFired(WeaponType::Pistol)
        ]
    );

    assert!(ThrownWeaponSystem::throw_from_player(
        &mut world,
        Vec2::new(1.0, 0.0)
    ));
    assert!(world.get_component::<Weapon>(player).is_none()); // unarmed
    assert_eq!(get_player_weapon(&world), None);
    // Unarmed: the trigger does nothing at all (no event either).
    assert!(!fire_player_weapon(&mut world, Vec2::new(500.0, 0.0)));
    assert_eq!(world.drain_events(), vec![GameEvent::Throw]);

    let mut thrown = ThrownWeaponSystem;
    for _ in 0..300 {
        thrown.run(&mut world, 1.0 / 60.0);
        if world.query::<ThrownWeapon>().is_empty() {
            break;
        }
    }
    let pickups = world.query::<WeaponPickup>();
    assert_eq!(pickups.len(), 1);
    let landed = *world.get_component::<WeaponPickup>(pickups[0]).unwrap();
    assert_eq!((landed.weapon_type, landed.ammo), (WeaponType::Pistol, 10));

    // Walk over it and pick it up: same pistol, still 10 rounds.
    let where_it_lies = *world.get_component::<Position>(pickups[0]).unwrap();
    *world.get_component_mut::<Position>(player).unwrap() = where_it_lies;
    assert_eq!(
        PickupSystem::swap_for_player(&mut world),
        Some(WeaponType::Pistol)
    );
    let held = *world.get_component::<Weapon>(player).unwrap();
    assert_eq!((held.weapon_type, held.ammo), (WeaponType::Pistol, 10));
    assert!(world.query::<WeaponPickup>().is_empty()); // consumed (was unarmed)
}

/// Empty gun: dry-fires, never refills; the intended loop is throw + take.
#[test]
fn test_empty_gun_dry_fires_until_swapped() {
    let mut world = World::new();
    let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
    *world.get_component_mut::<Weapon>(player).unwrap() = Weapon::with_ammo(WeaponType::Pistol, 1);
    fire_player_weapon(&mut world, Vec2::new(100.0, 0.0));
    world
        .get_component_mut::<Weapon>(player)
        .unwrap()
        .fire_timer = 0.0;
    assert!(!fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
    assert_eq!(
        world.drain_events(),
        vec![
            GameEvent::PlayerFired(WeaponType::Pistol),
            GameEvent::DryFire
        ]
    );
    assert_eq!(world.query::<Bullet>().len(), 1);

    // A machine gun lies here: swap and we're firing again.
    spawn_pickup_with_ammo(&mut world, Vec2::new(0.0, 0.0), WeaponType::MachineGun, 9);
    assert_eq!(
        PickupSystem::swap_for_player(&mut world),
        Some(WeaponType::MachineGun)
    );
    world.drain_events();
    fire_player_weapon(&mut world, Vec2::new(100.0, 0.0));
    assert_eq!(
        world.drain_events(),
        vec![GameEvent::PlayerFired(WeaponType::MachineGun)]
    );
    assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 8);
    // The empty pistol lies where the MG was, still empty.
    let floor = world.query::<WeaponPickup>();
    assert_eq!(floor.len(), 1);
    let p = world.get_component::<WeaponPickup>(floor[0]).unwrap();
    assert_eq!((p.weapon_type, p.ammo), (WeaponType::Pistol, 0));
}

/// The full event flow through the headless simulation: fire -> bullet hit ->
/// enemy down, each announced once, in order, with the right weapon.
#[test]
fn test_sim_events_fire_hit_down() {
    use open_miami::sim::Simulation;
    let mut world = World::new();
    spawn_player(&mut world, Vec2::new(0.0, 0.0));
    let enemy = spawn_enemy(&mut world, Vec2::new(0.0, -80.0));
    world.get_component_mut::<Health>(enemy).unwrap().current = 50;
    let mut sim = Simulation::from_world(world);

    let mut seen: Vec<GameEvent> = Vec::new();
    for _ in 0..120 {
        if sim.enemies_alive() == 0 {
            break;
        }
        sim.player_fire(Vec2::new(0.0, -80.0));
        sim.step(1.0 / 60.0);
        seen.extend(sim.world.drain_events());
    }
    assert_eq!(sim.enemies_alive(), 0);
    let fired = seen
        .iter()
        .filter(|e| matches!(e, GameEvent::PlayerFired(WeaponType::Pistol)))
        .count();
    let hits = seen
        .iter()
        .filter(|e| {
            matches!(
                e,
                GameEvent::EnemyHit {
                    by: WeaponType::Pistol
                }
            )
        })
        .count();
    let downs = seen
        .iter()
        .filter(|e| matches!(e, GameEvent::EnemyDown))
        .count();
    assert!(fired >= 1, "{seen:?}");
    assert_eq!(hits, 1, "50 hp / 50 dmg = one hit: {seen:?}");
    assert_eq!(downs, 1, "one kill announced once: {seen:?}");
    // The hit is announced before the down.
    let hit_at = seen
        .iter()
        .position(|e| matches!(e, GameEvent::EnemyHit { .. }))
        .unwrap();
    let down_at = seen
        .iter()
        .position(|e| matches!(e, GameEvent::EnemyDown))
        .unwrap();
    assert!(hit_at < down_at);
    // Nothing else leaked (no pickup/throw/hurt from a static enemy).
    assert!(seen.iter().all(|e| !matches!(
        e,
        GameEvent::Pickup | GameEvent::Throw | GameEvent::PlayerHurt
    )));
}
