// Game setup and entity spawning helpers
use crate::components::*;
use crate::ecs::{Entity, World};
use crate::math::Vec2;

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
    world.add_component(entity, Rotation::new(0.0));
    world.add_component(entity, AI::new());

    entity
}

/// Initialize a new game world with player and enemies
pub fn initialize_game(world: &mut World, level: usize) {
    // Spawn player (same position for all levels)
    spawn_player(world, Vec2::new(400.0, 300.0));

    // Level 1 (level 0): Original layout with 4 enemies
    if level == 0 {
        // Original walls
        world.add_wall(200.0, 200.0, 400.0, 20.0); // Horizontal wall
        world.add_wall(200.0, 200.0, 20.0, 200.0); // Vertical wall (L-shape)
        world.add_wall(800.0, 300.0, 20.0, 300.0); // Vertical wall
        world.add_wall(400.0, 600.0, 300.0, 20.0); // Horizontal wall

        // Original 4 enemies
        spawn_enemy(world, Vec2::new(600.0, 300.0));
        spawn_enemy(world, Vec2::new(800.0, 400.0));
        spawn_enemy(world, Vec2::new(300.0, 500.0));
        spawn_enemy(world, Vec2::new(700.0, 200.0));
    } else {
        // Levels 2-12: 12 enemies (3x more) with varied wall layouts
        let enemy_count = 12;

        // Generate level-specific wall patterns using level as seed
        match level {
            1 => {
                // Level 2: Cross pattern
                world.add_wall(400.0, 100.0, 20.0, 400.0); // Vertical center
                world.add_wall(200.0, 300.0, 400.0, 20.0); // Horizontal center
                world.add_wall(700.0, 400.0, 20.0, 300.0); // Right vertical
                world.add_wall(150.0, 600.0, 300.0, 20.0); // Bottom horizontal
            }
            2 => {
                // Level 3: Diagonal corridors
                world.add_wall(300.0, 150.0, 300.0, 20.0); // Top diagonal
                world.add_wall(600.0, 150.0, 20.0, 300.0); // Right diagonal
                world.add_wall(300.0, 450.0, 300.0, 20.0); // Bottom diagonal
                world.add_wall(300.0, 150.0, 20.0, 300.0); // Left diagonal
            }
            3 => {
                // Level 4: Room clusters
                world.add_wall(250.0, 250.0, 150.0, 20.0); // Top left room
                world.add_wall(250.0, 250.0, 20.0, 150.0);
                world.add_wall(700.0, 350.0, 150.0, 20.0); // Right room
                world.add_wall(700.0, 350.0, 20.0, 200.0);
            }
            4 => {
                // Level 5: Maze-like
                world.add_wall(200.0, 150.0, 20.0, 250.0); // Left maze wall
                world.add_wall(400.0, 200.0, 20.0, 300.0); // Center maze wall
                world.add_wall(600.0, 150.0, 20.0, 250.0); // Right maze wall
                world.add_wall(300.0, 500.0, 400.0, 20.0); // Bottom wall
            }
            5 => {
                // Level 6: Pillars
                world.add_wall(300.0, 250.0, 60.0, 60.0); // Top left pillar
                world.add_wall(600.0, 250.0, 60.0, 60.0); // Top right pillar
                world.add_wall(300.0, 500.0, 60.0, 60.0); // Bottom left pillar
                world.add_wall(600.0, 500.0, 60.0, 60.0); // Bottom right pillar
            }
            6 => {
                // Level 7: T-junctions
                world.add_wall(250.0, 300.0, 300.0, 20.0); // Top horizontal
                world.add_wall(450.0, 300.0, 20.0, 200.0); // Center vertical
                world.add_wall(700.0, 200.0, 20.0, 300.0); // Right vertical
                world.add_wall(250.0, 550.0, 200.0, 20.0); // Bottom horizontal
            }
            7 => {
                // Level 8: Spiral
                world.add_wall(300.0, 200.0, 400.0, 20.0); // Top
                world.add_wall(680.0, 200.0, 20.0, 300.0); // Right
                world.add_wall(300.0, 480.0, 400.0, 20.0); // Bottom
                world.add_wall(300.0, 280.0, 20.0, 200.0); // Left inner
            }
            8 => {
                // Level 9: Grid
                world.add_wall(350.0, 200.0, 20.0, 400.0); // Left vertical
                world.add_wall(550.0, 200.0, 20.0, 400.0); // Right vertical
                world.add_wall(200.0, 350.0, 500.0, 20.0); // Top horizontal
                world.add_wall(200.0, 500.0, 500.0, 20.0); // Bottom horizontal
            }
            9 => {
                // Level 10: U-shapes
                world.add_wall(250.0, 200.0, 20.0, 250.0); // Left U
                world.add_wall(250.0, 430.0, 150.0, 20.0);
                world.add_wall(650.0, 300.0, 20.0, 250.0); // Right U
                world.add_wall(500.0, 300.0, 150.0, 20.0);
            }
            10 => {
                // Level 11: Zigzag
                world.add_wall(200.0, 250.0, 250.0, 20.0); // Top left
                world.add_wall(450.0, 250.0, 20.0, 150.0); // Down
                world.add_wall(450.0, 400.0, 250.0, 20.0); // Bottom right
                world.add_wall(700.0, 400.0, 20.0, 150.0); // Down right
            }
            _ => {
                // Level 12: Arena (minimal walls)
                world.add_wall(400.0, 250.0, 200.0, 20.0); // Top center
                world.add_wall(400.0, 500.0, 200.0, 20.0); // Bottom center
                world.add_wall(300.0, 350.0, 20.0, 100.0); // Left small
                world.add_wall(680.0, 350.0, 20.0, 100.0); // Right small
            }
        }

        // Spawn 12 enemies in level-specific formations
        // Base positions scaled and varied by level
        let offset = (level as f32 * 13.7) % 100.0; // Pseudo-random offset per level

        for i in 0..enemy_count {
            let angle = (i as f32) * 2.0 * std::f32::consts::PI / (enemy_count as f32);
            let radius = 250.0 + offset;
            let x = 500.0 + radius * angle.cos();
            let y = 400.0 + radius * angle.sin();

            // Add some variation to prevent perfect circles
            let variation_x = ((i * 17 + level * 23) % 100) as f32 - 50.0;
            let variation_y = ((i * 31 + level * 19) % 100) as f32 - 50.0;

            spawn_enemy(world, Vec2::new(x + variation_x, y + variation_y));
        }
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

/// Get player position
pub fn get_player_position(world: &World) -> Option<Vec2> {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Position>(e))
        .map(|p| p.to_vec2())
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
