// Game setup and entity spawning helpers
use crate::components::*;
use crate::ecs::{Entity, World};
use crate::levels::{level_def, BOSS_LEVEL, PLAYER_SPAWN};
use crate::math::Vec2;
use crate::systems::boss::{BOSS_ATTACK_RANGE, BOSS_MASK_SPEED, BOSS_MAX_HEALTH, BOSS_RADIUS};
use crate::systems::combat::CombatSystem;

/// Where the shoggoth boss stands at the start of its floor.
pub const BOSS_SPAWN: Vec2 = Vec2::new(400.0, 560.0);

/// Spawn the shoggoth boss (a big, tanky, masked enemy) at `position`.
pub fn spawn_boss(world: &mut World, position: Vec2) -> Entity {
    let entity = world.spawn();
    let pos = Position::from_vec2(position);

    world.add_component(entity, Enemy);
    world.add_component(entity, Boss::new());
    world.add_component(entity, pos);
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(BOSS_MASK_SPEED));
    world.add_component(entity, Health::new(BOSS_MAX_HEALTH));
    world.add_component(entity, Radius::new(BOSS_RADIUS));
    world.add_component(entity, Rotation::new(0.0));

    let mut ai = AI::new();
    ai.attack_range = BOSS_ATTACK_RANGE;
    ai.detection_range = 5000.0;
    world.add_component(entity, ai);

    entity
}

/// Spawn a player entity
pub fn spawn_player(world: &mut World, position: Vec2) -> Entity {
    let entity = world.spawn();

    world.add_component(entity, Player);
    world.add_component(entity, Position::from_vec2(position));
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(200.0));
    world.add_component(entity, Health::new(100));
    world.add_component(entity, Rotation::new(0.0));
    world.add_component(entity, Radius::new(15.0));
    world.add_component(entity, Weapon::new(WeaponType::Pistol));

    entity
}

/// The weapon an enemy of a given type carries (and drops when killed).
/// All enemy weapons are firearms so a dropped pickup is always an upgrade in
/// ammo rather than a downgrade to fists.
pub fn weapon_for_enemy(enemy_type: EnemyType) -> WeaponType {
    match enemy_type {
        EnemyType::Idle => WeaponType::Pistol,
        EnemyType::Wandering => WeaponType::MachineGun,
        EnemyType::Patrolling => WeaponType::Shotgun,
    }
}

/// Spawn an enemy entity with a specific type
pub fn spawn_enemy_with_type(world: &mut World, position: Vec2, enemy_type: EnemyType) -> Entity {
    let entity = world.spawn();
    let pos = Position::from_vec2(position);

    world.add_component(entity, Enemy);
    world.add_component(entity, pos);
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(100.0));
    world.add_component(entity, Health::new(50));
    world.add_component(entity, Radius::new(12.0));
    world.add_component(entity, Rotation::new(0.0));
    world.add_component(entity, AI::new_with_type(enemy_type, pos));
    world.add_component(entity, Weapon::new(weapon_for_enemy(enemy_type)));

    entity
}

/// Spawn an enemy entity (default to Idle type for backwards compatibility)
pub fn spawn_enemy(world: &mut World, position: Vec2) -> Entity {
    spawn_enemy_with_type(world, position, EnemyType::Idle)
}

/// Initialize a new game world with the player and a level's designed layout.
pub fn initialize_game(world: &mut World, level: usize) {
    // Spawn player (same position for all levels)
    spawn_player(world, PLAYER_SPAWN);

    // Build the level from its data-driven definition
    let def = level_def(level);

    for (x, y, width, height) in def.walls {
        world.add_wall(x, y, width, height);
    }

    for (x, y, enemy_type) in def.enemies {
        spawn_enemy_with_type(world, Vec2::new(x, y), enemy_type);
    }

    // The hidden final floor: the shoggoth waits below.
    if level == BOSS_LEVEL {
        spawn_boss(world, BOSS_SPAWN);
    }
}

/// Check if player is alive
pub fn is_player_alive(world: &World) -> bool {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Health>(e))
        .map(|h| h.is_alive())
        .unwrap_or(false)
}

/// Get player health for UI
pub fn get_player_health(world: &World) -> i32 {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Health>(e))
        .map(|h| h.current)
        .unwrap_or(0)
}

/// Get player ammo for UI
pub fn get_player_ammo(world: &World) -> i32 {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Weapon>(e))
        .map(|w| w.ammo)
        .unwrap_or(0)
}

/// Fire the player's currently held weapon toward a world position. Melee hits
/// instantly (returns whether it connected); ranged weapons spawn a bullet
/// entity (returns `false`, since bullets resolve asynchronously). Does nothing
/// if the weapon is on cooldown or out of ammo.
///
/// This is the input-independent core of shooting so it can be driven both by
/// the browser input layer and by the headless [`crate::sim::Simulation`].
pub fn fire_player_weapon(world: &mut World, target_world_pos: Vec2) -> bool {
    let player = match world.query::<Player>().first() {
        Some(&e) => e,
        None => return false,
    };
    let player_pos = match world.get_component::<Position>(player) {
        Some(p) => *p,
        None => return false,
    };
    let (damage, is_melee, can_fire) = match world.get_component::<Weapon>(player) {
        Some(w) => (w.damage, w.weapon_type == WeaponType::Melee, w.can_fire()),
        None => return false, // unarmed
    };

    if !can_fire {
        return false;
    }

    if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
        weapon.fire();
    }

    let target_pos = Position::from_vec2(target_world_pos);

    if is_melee {
        CombatSystem::process_melee(world, player_pos, target_pos, damage, 50.0)
    } else {
        let dx = target_pos.x - player_pos.x;
        let dy = target_pos.y - player_pos.y;
        let length = (dx * dx + dy * dy).sqrt();

        let bullet = Bullet::new(damage);
        let bullet_speed = bullet.speed;
        let (vel_x, vel_y) = if length > 0.0 {
            (dx / length * bullet_speed, dy / length * bullet_speed)
        } else {
            (0.0, 0.0)
        };

        let bullet_entity = world.spawn();
        world.add_component(bullet_entity, bullet);
        world.add_component(bullet_entity, player_pos);
        world.add_component(bullet_entity, Velocity::new(vel_x, vel_y));
        world.add_component(bullet_entity, Radius::new(2.0));

        false
    }
}

/// Get the player's current weapon type (for the HUD)
pub fn get_player_weapon(world: &World) -> Option<WeaponType> {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Weapon>(e))
        .map(|w| w.weapon_type)
}

/// Human-readable name for a weapon type
pub fn weapon_name(weapon_type: WeaponType) -> &'static str {
    match weapon_type {
        WeaponType::Pistol => "Pistol",
        WeaponType::Shotgun => "Shotgun",
        WeaponType::MachineGun => "Machine Gun",
        WeaponType::Melee => "Melee",
    }
}

/// Get player position
pub fn get_player_position(world: &World) -> Option<Vec2> {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Position>(e))
        .map(|p| p.to_vec2())
}

/// Sum of current health across all enemies (used to detect a hit landing).
pub fn total_enemy_health(world: &World) -> i32 {
    world
        .query::<Enemy>()
        .iter()
        .filter_map(|&e| world.get_component::<Health>(e))
        .map(|h| h.current.max(0))
        .sum()
}

/// Count alive enemies
pub fn count_alive_enemies(world: &World) -> usize {
    let enemies: Vec<Entity> = world.query::<Enemy>();
    enemies
        .iter()
        .filter(|&&e| {
            world
                .get_component::<Health>(e)
                .map(|h| h.is_alive())
                .unwrap_or(false)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_player() {
        let mut world = World::new();
        let player = spawn_player(&mut world, Vec2::new(100.0, 200.0));

        assert!(world.has_component::<Player>(player));
        assert!(world.has_component::<Position>(player));
        assert!(world.has_component::<Health>(player));

        let pos = world.get_component::<Position>(player).unwrap();
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 200.0);

        let health = world.get_component::<Health>(player).unwrap();
        assert_eq!(health.current, 100);
    }

    #[test]
    fn test_spawn_enemy() {
        let mut world = World::new();
        let enemy = spawn_enemy(&mut world, Vec2::new(50.0, 75.0));

        assert!(world.has_component::<Enemy>(enemy));
        assert!(world.has_component::<AI>(enemy));
        assert!(world.has_component::<Position>(enemy));

        let pos = world.get_component::<Position>(enemy).unwrap();
        assert_eq!(pos.x, 50.0);
        assert_eq!(pos.y, 75.0);
    }

    #[test]
    fn test_initialize_game() {
        let mut world = World::new();
        initialize_game(&mut world, 0);

        assert_eq!(world.query::<Player>().len(), 1);
        assert_eq!(world.query::<Enemy>().len(), 4);

        // Test level 2 has 12 enemies
        let mut world2 = World::new();
        initialize_game(&mut world2, 1);
        assert_eq!(world2.query::<Player>().len(), 1);
        assert_eq!(world2.query::<Enemy>().len(), 12);
    }

    #[test]
    fn test_is_player_alive() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));

        assert!(is_player_alive(&world));

        // Kill player
        let player = world.query::<Player>()[0];
        world
            .get_component_mut::<Health>(player)
            .unwrap()
            .take_damage(100);

        assert!(!is_player_alive(&world));
    }

    #[test]
    fn test_get_player_health() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));

        assert_eq!(get_player_health(&world), 100);

        let player = world.query::<Player>()[0];
        world
            .get_component_mut::<Health>(player)
            .unwrap()
            .take_damage(30);

        assert_eq!(get_player_health(&world), 70);
    }

    #[test]
    fn test_count_alive_enemies() {
        let mut world = World::new();
        initialize_game(&mut world, 0);

        assert_eq!(count_alive_enemies(&world), 4);

        // Kill one enemy
        let enemy = world.query::<Enemy>()[0];
        world
            .get_component_mut::<Health>(enemy)
            .unwrap()
            .take_damage(100);

        assert_eq!(count_alive_enemies(&world), 3);
    }

    #[test]
    fn test_get_player_position() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(123.0, 456.0));

        let pos = get_player_position(&world).unwrap();
        assert_eq!(pos.x, 123.0);
        assert_eq!(pos.y, 456.0);
    }
}
