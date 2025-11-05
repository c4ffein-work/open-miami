use crate::components::{AIState, Enemy, Player, Position, Speed, Velocity, AI};
use crate::ecs::{Entity, System, World};

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

    fn update_ai_state(ai: &mut AI, distance: f32) {
        ai.state = if distance < ai.attack_range {
            AIState::Attack
        } else if distance < ai.detection_range {
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

        // Query all enemies with AI, Position, Velocity, and Speed
        let enemies: Vec<Entity> = world.query::<Enemy>();

        for entity in enemies {
            let (enemy_pos, ai, speed) = match (
                world.get_component::<Position>(entity),
                world.get_component::<AI>(entity),
                world.get_component::<Speed>(entity),
            ) {
                (Some(pos), Some(ai), Some(spd)) => (*pos, *ai, *spd),
                _ => continue,
            };

            // Calculate distance to player
            let distance = enemy_pos.distance_to(&player_pos);

            // Update AI state
            if let Some(ai) = world.get_component_mut::<AI>(entity) {
                Self::update_ai_state(ai, distance);

                // Update attack timer
                if ai.attack_timer > 0.0 {
                    ai.attack_timer -= dt;
                }
            }

            // Get updated AI state
            let ai = world.get_component::<AI>(entity).copied().unwrap();

            // Update velocity based on state
            if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                match ai.state {
                    AIState::Chase => {
                        let (dir_x, dir_y) =
                            Self::calculate_move_direction(&enemy_pos, &player_pos);
                        velocity.x = dir_x * speed.value;
                        velocity.y = dir_y * speed.value;
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
        world.add_component(player, Position::new(500.0, 0.0)); // Far away

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));

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
        }

        let mut system = AISystem;
        system.run(&mut world, 0.016);

        // All enemies should have updated AI states
        let enemies: Vec<_> = world.query::<Enemy>();
        assert_eq!(enemies.len(), 3);
    }
}
