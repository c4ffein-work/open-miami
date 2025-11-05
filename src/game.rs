// Game setup and entity spawning helpers
use crate::ecs::{World, Entity};
use crate::components::*;
use macroquad::prelude::Vec2;

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

/// Spawn an enemy entity
pub fn spawn_enemy(world: &mut World, position: Vec2) -> Entity {
    let entity = world.spawn();

    world.add_component(entity, Enemy);
    world.add_component(entity, Position::from_vec2(position));
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(100.0));
    world.add_component(entity, Health::new(50));
    world.add_component(entity, Radius::new(12.0));
    world.add_component(entity, AI::new());

    entity
}

/// Initialize a new game world with player and enemies
pub fn initialize_game(world: &mut World) {
    // Spawn player
    spawn_player(world, Vec2::new(400.0, 300.0));

    // Spawn enemies
    spawn_enemy(world, Vec2::new(600.0, 300.0));
    spawn_enemy(world, Vec2::new(800.0, 400.0));
    spawn_enemy(world, Vec2::new(300.0, 500.0));
    spawn_enemy(world, Vec2::new(700.0, 200.0));
}

/// Check if player is alive
pub fn is_player_alive(world: &World) -> bool {
    let players: Vec<Entity> = world.query::<Player>();
    players.first()
        .and_then(|&e| world.get_component::<Health>(e))
        .map(|h| h.is_alive())
        .unwrap_or(false)
}

/// Get player health for UI
pub fn get_player_health(world: &World) -> i32 {
    let players: Vec<Entity> = world.query::<Player>();
    players.first()
        .and_then(|&e| world.get_component::<Health>(e))
        .map(|h| h.current)
        .unwrap_or(0)
}

/// Get player ammo for UI
pub fn get_player_ammo(world: &World) -> i32 {
    let players: Vec<Entity> = world.query::<Player>();
    players.first()
        .and_then(|&e| world.get_component::<Weapon>(e))
        .map(|w| w.ammo)
        .unwrap_or(0)
}

/// Get player position
pub fn get_player_position(world: &World) -> Option<Vec2> {
    let players: Vec<Entity> = world.query::<Player>();
    players.first()
        .and_then(|&e| world.get_component::<Position>(e))
        .map(|p| p.to_vec2())
}

/// Count alive enemies
pub fn count_alive_enemies(world: &World) -> usize {
    let enemies: Vec<Entity> = world.query::<Enemy>();
    enemies.iter()
        .filter(|&&e| {
            world.get_component::<Health>(e)
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
        initialize_game(&mut world);

        assert_eq!(world.query::<Player>().len(), 1);
        assert_eq!(world.query::<Enemy>().len(), 4);
    }

    #[test]
    fn test_is_player_alive() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));

        assert!(is_player_alive(&world));

        // Kill player
        let player = world.query::<Player>()[0];
        world.get_component_mut::<Health>(player).unwrap().take_damage(100);

        assert!(!is_player_alive(&world));
    }

    #[test]
    fn test_get_player_health() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));

        assert_eq!(get_player_health(&world), 100);

        let player = world.query::<Player>()[0];
        world.get_component_mut::<Health>(player).unwrap().take_damage(30);

        assert_eq!(get_player_health(&world), 70);
    }

    #[test]
    fn test_count_alive_enemies() {
        let mut world = World::new();
        initialize_game(&mut world);

        assert_eq!(count_alive_enemies(&world), 4);

        // Kill one enemy
        let enemy = world.query::<Enemy>()[0];
        world.get_component_mut::<Health>(enemy).unwrap().take_damage(100);

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
