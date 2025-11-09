use crate::collision::has_line_of_sight;
use crate::components::{AIState, Enemy, Health, Player, Position, Rotation, Speed, Velocity, AI};
use crate::ecs::world::Wall;
use crate::ecs::{Entity, System, World};
use crate::math::Vec2;
use crate::pathfinding::NavigationGrid;

/// System that handles enemy AI behavior
pub struct AISystem;

impl AISystem {
    fn find_player_position(world: &World) -> Option<Position> {
        let players: Vec<Entity> = world.query::<Player>();
        players
            .first()
            .and_then(|&entity| world.get_component::<Position>(entity))
            .copied()
    }

    fn update_ai_state(
        ai: &mut AI,
        distance: f32,
        enemy_pos: Vec2,
        player_pos: Vec2,
        walls: &[Wall],
    ) {
        // Only detect player if there's line of sight (no walls blocking)
        let has_los = has_line_of_sight(enemy_pos, player_pos, walls);

        ai.state = if has_los && distance < ai.attack_range {
            AIState::Attack
        } else if has_los && distance < ai.detection_range {
            AIState::Chase
        } else {
            AIState::Idle
        };
    }

    fn calculate_move_direction(enemy_pos: &Position, player_pos: &Position) -> (f32, f32) {
        let dx = player_pos.x - enemy_pos.x;
        let dy = player_pos.y - enemy_pos.y;
        let distance = (dx * dx + dy * dy).sqrt();

        if distance > 0.0 {
            (dx / distance, dy / distance)
        } else {
            (0.0, 0.0)
        }
    }
}

impl System for AISystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        // Find player position first
        let player_pos = match Self::find_player_position(world) {
            Some(pos) => pos,
            None => return, // No player, nothing to do
        };

        // Get walls before any mutable borrows (clone to avoid borrow conflicts)
        let walls: Vec<Wall> = world.walls().to_vec();

        // Create navigation grid from world walls
        let nav_grid = NavigationGrid::new(&walls);

        // Query all enemies with AI, Position, Velocity, and Speed
        let enemies: Vec<Entity> = world.query::<Enemy>();

        for entity in enemies {
            let (enemy_pos, _ai, speed, health) = match (
                world.get_component::<Position>(entity),
                world.get_component::<AI>(entity),
                world.get_component::<Speed>(entity),
                world.get_component::<Health>(entity),
            ) {
                (Some(pos), Some(ai), Some(spd), Some(hp)) => (*pos, *ai, *spd, *hp),
                _ => continue,
            };

            // Skip dead enemies
            if health.is_dead() {
                // Set velocity to zero for dead enemies
                if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                    velocity.x = 0.0;
                    velocity.y = 0.0;
                }
                continue;
            }

            // Calculate distance to player
            let distance = enemy_pos.distance_to(&player_pos);

            // Update AI state with line-of-sight check
            if let Some(ai) = world.get_component_mut::<AI>(entity) {
                Self::update_ai_state(
                    ai,
                    distance,
                    enemy_pos.to_vec2(),
                    player_pos.to_vec2(),
                    &walls,
                );

                // Update attack timer
                if ai.attack_timer > 0.0 {
                    ai.attack_timer -= dt;
                }
            }

            // Get updated AI state
            let ai = world.get_component::<AI>(entity).copied().unwrap();

            // Update rotation to face player (for Chase and Attack states)
            if matches!(ai.state, AIState::Chase | AIState::Attack) {
                let dx = player_pos.x - enemy_pos.x;
                let dy = player_pos.y - enemy_pos.y;
                let angle = dy.atan2(dx);

                if let Some(rotation) = world.get_component_mut::<Rotation>(entity) {
                    rotation.angle = angle;
                }
            }

            // Update velocity based on state
            if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                match ai.state {
                    AIState::Chase => {
                        // Use pathfinding to get next waypoint
                        let enemy_world_pos = enemy_pos.to_vec2();
                        let player_world_pos = player_pos.to_vec2();

                        if let Some(next_waypoint) =
                            nav_grid.get_next_waypoint(enemy_world_pos, player_world_pos)
                        {
                            // Calculate direction to waypoint
                            let dx = next_waypoint.x - enemy_pos.x;
                            let dy = next_waypoint.y - enemy_pos.y;
                            let distance = (dx * dx + dy * dy).sqrt();

                            if distance > 0.0 {
                                velocity.x = (dx / distance) * speed.value;
                                velocity.y = (dy / distance) * speed.value;
                            } else {
                                velocity.x = 0.0;
                                velocity.y = 0.0;
                            }
                        } else {
                            // No path found, fall back to direct movement
                            let (dir_x, dir_y) =
                                Self::calculate_move_direction(&enemy_pos, &player_pos);
                            velocity.x = dir_x * speed.value;
                            velocity.y = dir_y * speed.value;
                        }
                    }
                    AIState::Attack => {
                        // Stop moving when attacking
                        velocity.x = 0.0;
                        velocity.y = 0.0;
                    }
                    AIState::Idle | AIState::Patrol => {
                        velocity.x = 0.0;
                        velocity.y = 0.0;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_system_chase_when_player_in_range() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(200.0, 0.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Chase);

        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert!(velocity.x > 0.0); // Moving toward player
    }

    #[test]
    fn test_ai_system_attack_when_player_close() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(30.0, 0.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Attack);

        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0); // Stopped to attack
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_system_idle_when_player_far() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(1000.0, 0.0)); // Far away (beyond 900 detection range)

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Idle);

        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_system_updates_attack_timer() {
        let mut world = World::new();

        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));

        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        let mut ai = AI::new();
        ai.reset_attack_timer();
        world.add_component(enemy, ai);
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.5);

        let ai = world.get_component::<AI>(enemy).unwrap();
        assert!((ai.attack_timer - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_ai_system_multiple_enemies() {
        let mut world = World::new();

        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 0.0));

        // Create 3 enemies at different distances
        for i in 0..3 {
            let enemy = world.spawn();
            world.add_component(enemy, Enemy);
            world.add_component(enemy, Position::new(i as f32 * 50.0, 0.0));
            world.add_component(enemy, AI::new());
            world.add_component(enemy, Velocity::zero());
            world.add_component(enemy, Speed::new(100.0));
            world.add_component(enemy, Health::new(100));
        }

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // All enemies should have updated AI states
        let enemies: Vec<_> = world.query::<Enemy>();
        assert_eq!(enemies.len(), 3);
    }

    #[test]
    fn test_ai_pathfinding_no_obstacles() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create player (within detection range)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(300.0, 100.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 100.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Enemy should be in Chase state and moving toward player
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Chase);

        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        // Should have non-zero velocity
        assert!(velocity.x.abs() > 0.0 || velocity.y.abs() > 0.0);

        // Velocity should point generally toward player
        assert!(velocity.x > 0.0); // Moving right
        assert!(velocity.y > 0.0); // Moving down
    }

    #[test]
    fn test_ai_pathfinding_with_wall_obstacle() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Add a vertical wall between enemy and player
        world.add_wall(250.0, 0.0, 20.0, 300.0);

        // Create player on right side of wall (within detection range but no line of sight)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(350.0, 150.0));

        // Create enemy on left side of wall
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 150.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Enemy should NOT detect player (line of sight blocked by wall)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Idle);

        // Enemy should not be moving
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_pathfinding_around_corner() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create L-shaped wall
        world.add_wall(200.0, 200.0, 400.0, 20.0); // Horizontal wall
        world.add_wall(200.0, 200.0, 20.0, 200.0); // Vertical wall

        // Create player in the corner (line of sight blocked by walls)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 300.0));

        // Create enemy outside the corner (within detection range but no line of sight)
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(300.0, 100.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Enemy should NOT detect player (line of sight blocked by walls)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Idle);

        // Enemy should not be moving
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_pathfinding_multiple_walls() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create a maze of walls
        world.add_wall(200.0, 0.0, 20.0, 300.0);
        world.add_wall(400.0, 200.0, 20.0, 400.0);

        // Create player (within detection range but line of sight blocked by walls)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(350.0, 250.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 100.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Enemy should NOT detect player (line of sight blocked by walls)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Idle);

        // Enemy should not be moving
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_pathfinding_enemy_follows_over_time() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create player (within detection range)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(300.0, 100.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 100.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        // Get initial distance
        let initial_pos = *world.get_component::<Position>(enemy).unwrap();
        let player_pos_clone = *world.get_component::<Position>(player).unwrap();
        let initial_distance = initial_pos.distance_to(&player_pos_clone);

        let mut system = AISystem;

        // Simulate multiple frames
        for _ in 0..10 {
            system.run(&mut world, 0.016);

            // Apply velocity to position (simulate movement system)
            let enemies: Vec<_> = world.query::<Enemy>();
            for entity in enemies {
                // Get velocity first, then update position
                let vel = *world.get_component::<Velocity>(entity).unwrap();
                if let Some(pos) = world.get_component_mut::<Position>(entity) {
                    pos.x += vel.x * 0.016;
                    pos.y += vel.y * 0.016;
                }
            }
        }

        // Enemy should be closer to player
        let final_pos = world.get_component::<Position>(enemy).unwrap();
        let final_distance = final_pos.distance_to(&player_pos_clone);

        assert!(final_distance < initial_distance);
    }

    #[test]
    fn test_ai_pathfinding_stops_when_attacking() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create player very close to enemy (within attack range)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(30.0, 0.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Enemy should be attacking
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Attack);

        // Enemy should stop moving when attacking
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_pathfinding_respects_speed() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create player (within detection range of both enemies)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(250.0, 100.0));

        // Create slow enemy
        let slow_enemy = world.spawn();
        world.add_component(slow_enemy, Enemy);
        world.add_component(slow_enemy, Position::new(100.0, 100.0));
        world.add_component(slow_enemy, AI::new());
        world.add_component(slow_enemy, Velocity::zero());
        world.add_component(slow_enemy, Speed::new(50.0));
        world.add_component(slow_enemy, Health::new(100));

        // Create fast enemy
        let fast_enemy = world.spawn();
        world.add_component(fast_enemy, Enemy);
        world.add_component(fast_enemy, Position::new(100.0, 120.0));
        world.add_component(fast_enemy, AI::new());
        world.add_component(fast_enemy, Velocity::zero());
        world.add_component(fast_enemy, Speed::new(200.0));
        world.add_component(fast_enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Both should be chasing
        let slow_ai = world.get_component::<AI>(slow_enemy).unwrap();
        let fast_ai = world.get_component::<AI>(fast_enemy).unwrap();
        assert_eq!(slow_ai.state, AIState::Chase);
        assert_eq!(fast_ai.state, AIState::Chase);

        // Fast enemy should have higher velocity magnitude
        let slow_vel = world.get_component::<Velocity>(slow_enemy).unwrap();
        let fast_vel = world.get_component::<Velocity>(fast_enemy).unwrap();

        let slow_mag = (slow_vel.x * slow_vel.x + slow_vel.y * slow_vel.y).sqrt();
        let fast_mag = (fast_vel.x * fast_vel.x + fast_vel.y * fast_vel.y).sqrt();

        assert!(fast_mag > slow_mag);
        assert!((slow_mag - 50.0).abs() < 1.0); // Should be approximately 50.0
        assert!((fast_mag - 200.0).abs() < 1.0); // Should be approximately 200.0
    }

    #[test]
    fn test_ai_pathfinding_blocked_by_walls() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Add a wall between enemy and player
        world.add_wall(250.0, 0.0, 20.0, 500.0);

        // Create player on other side of wall (within detection range but no line of sight)
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(350.0, 250.0));

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 250.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // Enemy should NOT detect player through wall (line of sight blocked)
        // and remain Idle
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::Idle);

        // Enemy should not be moving
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }
}
