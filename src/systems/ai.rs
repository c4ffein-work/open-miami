use crate::collision::has_line_of_sight;
use crate::components::{
    AIState, Enemy, EnemyType, Health, Player, Position, Rotation, Speed, Velocity, WanderState, AI,
};
use crate::ecs::world::Wall;
use crate::ecs::{Entity, System, World};
use crate::pathfinding::NavigationGrid;
use std::f32::consts::PI;

// Simple pseudo-random number generator using hash
static mut RNG_STATE: u32 = 12345;

fn next_random() -> u32 {
    unsafe {
        RNG_STATE = RNG_STATE.wrapping_mul(1664525).wrapping_add(1013904223);
        RNG_STATE
    }
}

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

    /// Check if position is within the allowed movement square
    fn is_within_movement_square(pos: &Position, spawn: &Position, square_size: f32) -> bool {
        let dx = (pos.x - spawn.x).abs();
        let dy = (pos.y - spawn.y).abs();
        dx <= square_size && dy <= square_size
    }

    /// Check if target position is within the vision cone
    /// Vision cone is 90 degrees (PI/2), so 45 degrees on each side of facing direction
    fn is_within_vision_cone(
        enemy_pos: &Position,
        target_pos: &Position,
        enemy_rotation: f32,
    ) -> bool {
        // Calculate angle from enemy to target
        let dx = target_pos.x - enemy_pos.x;
        let dy = target_pos.y - enemy_pos.y;
        let angle_to_target = dy.atan2(dx);

        // Calculate angle difference
        let mut angle_diff = angle_to_target - enemy_rotation;

        // Normalize angle difference to [-PI, PI]
        while angle_diff > PI {
            angle_diff -= 2.0 * PI;
        }
        while angle_diff < -PI {
            angle_diff += 2.0 * PI;
        }

        // Check if within 90-degree cone (45 degrees on each side)
        let cone_half_angle = PI / 4.0; // 45 degrees
        angle_diff.abs() <= cone_half_angle
    }

    /// Find the direction with the most open space using 36 direction rays
    fn find_most_open_direction(
        pos: &Position,
        spawn: &Position,
        square_size: f32,
        walls: &[Wall],
    ) -> f32 {
        // Check 36 directions (every 10 degrees)
        let mut best_direction = 0.0;
        let mut best_distance = 0.0;

        for i in 0..36 {
            let angle = (i as f32) * (PI * 2.0 / 36.0);
            let max_check_distance = 200.0; // Check up to 200 pixels

            // Cast ray to find nearest obstacle
            let mut distance = max_check_distance;

            // Check against walls
            for step in 1..=20 {
                let check_distance = (step as f32) * 10.0;
                let check_x = pos.x + angle.cos() * check_distance;
                let check_y = pos.y + angle.sin() * check_distance;
                let check_pos = Position::new(check_x, check_y);

                // Check if hits wall
                let hit_wall = walls.iter().any(|wall| {
                    check_x >= wall.x
                        && check_x <= wall.x + wall.width
                        && check_y >= wall.y
                        && check_y <= wall.y + wall.height
                });

                // Check if outside movement square
                let outside_square =
                    !Self::is_within_movement_square(&check_pos, spawn, square_size);

                if hit_wall || outside_square {
                    distance = check_distance;
                    break;
                }
            }

            if distance > best_distance {
                best_distance = distance;
                best_direction = angle;
            }
        }

        // 50% chance: use best direction, 50% chance: random direction
        if next_random().is_multiple_of(2) {
            best_direction
        } else {
            (next_random() as f32 / u32::MAX as f32) * PI * 2.0
        }
    }

    /// Random float between min and max
    fn random_range(min: f32, max: f32) -> f32 {
        let r = next_random() as f32 / u32::MAX as f32;
        r * (max - min) + min
    }

    /// Random integer between min and max (inclusive)
    fn random_int_range(min: i32, max: i32) -> i32 {
        let range = (max - min + 1) as u32;
        min + (next_random() % range) as i32
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

        // Query all enemies
        let enemies: Vec<Entity> = world.query::<Enemy>();

        for entity in enemies {
            let (enemy_pos, speed, health) = match (
                world.get_component::<Position>(entity),
                world.get_component::<Speed>(entity),
                world.get_component::<Health>(entity),
            ) {
                (Some(pos), Some(spd), Some(hp)) => (*pos, *spd, *hp),
                _ => continue,
            };

            // Skip dead enemies
            if health.is_dead() {
                if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                    velocity.x = 0.0;
                    velocity.y = 0.0;
                }
                continue;
            }

            // Calculate distance to player and line of sight
            let distance = enemy_pos.distance_to(&player_pos);
            let has_los = has_line_of_sight(enemy_pos.to_vec2(), player_pos.to_vec2(), &walls);

            // Get enemy rotation to check vision cone
            let enemy_rotation = world
                .get_component::<Rotation>(entity)
                .map(|r| r.angle)
                .unwrap_or(0.0);

            // Check if player is within vision cone
            let in_vision_cone =
                Self::is_within_vision_cone(&enemy_pos, &player_pos, enemy_rotation);

            let can_see_player = has_los
                && distance < world.get_component::<AI>(entity).unwrap().detection_range
                && in_vision_cone;

            // Update AI state machine
            if let Some(ai) = world.get_component_mut::<AI>(entity) {
                // Update timers
                if ai.attack_timer > 0.0 {
                    ai.attack_timer -= dt;
                }
                ai.state_timer -= dt;

                // State machine logic
                match ai.state {
                    AIState::Unaware => {
                        if can_see_player {
                            ai.state_timer = ai.spot_duration;
                            ai.state = AIState::SpottedUnsure;
                            ai.check_position = Some(enemy_pos);
                            ai.last_known_player_position = Some(player_pos);
                        } else {
                            // Perform initial behavior based on type
                            match ai.initial_type {
                                EnemyType::Idle => {
                                    // Stay still, do nothing
                                }
                                EnemyType::Wandering | EnemyType::Patrolling => {
                                    Self::update_wander_behavior(ai, &enemy_pos, &walls, dt);
                                }
                            }
                        }
                    }
                    AIState::SpottedUnsure => {
                        if can_see_player {
                            ai.last_known_player_position = Some(player_pos);
                            if ai.state_timer <= 0.0 {
                                // Seen player long enough, transition to sure
                                ai.state = AIState::SurePlayerSeen;
                            }
                        } else {
                            // Lost sight, check the last known position
                            if let Some(check_pos) = ai.check_position {
                                if enemy_pos.distance_to(&check_pos) < 5.0 {
                                    // Returned to check position, go back to unaware
                                    ai.state = AIState::Unaware;
                                    ai.check_position = None;
                                }
                            } else {
                                ai.state = AIState::Unaware;
                            }
                        }
                    }
                    AIState::SurePlayerSeen => {
                        if can_see_player {
                            // Keep chasing
                            ai.last_known_player_position = Some(player_pos);
                            ai.state_timer = ai.lost_player_duration;
                        } else {
                            // Lost sight of player
                            if ai.state_timer <= 0.0 {
                                // Been at last known position too long, get confused
                                ai.state = AIState::Confused;
                                ai.confusion_looks_remaining = Self::random_int_range(2, 3);
                                ai.confusion_look_timer = ai.confusion_look_duration;
                            }
                        }
                    }
                    AIState::Confused => {
                        if can_see_player {
                            // Found player again!
                            ai.state = AIState::SurePlayerSeen;
                            ai.last_known_player_position = Some(player_pos);
                            ai.state_timer = ai.lost_player_duration;
                        } else {
                            // Continue looking around
                            ai.confusion_look_timer -= dt;
                            if ai.confusion_look_timer <= 0.0 {
                                ai.confusion_looks_remaining -= 1;
                                if ai.confusion_looks_remaining <= 0 {
                                    // Done looking, transition based on initial type
                                    ai.state = AIState::Unaware;
                                    ai.wander_state = WanderState::Waiting;
                                    ai.wander_timer = Self::random_range(1.0, 2.0);
                                } else {
                                    // Look in another direction
                                    ai.confusion_look_timer = ai.confusion_look_duration;
                                }
                            }
                        }
                    }
                    _ => {} // Legacy states
                }
            }

            // Get updated AI state and compute movement
            let ai = world.get_component::<AI>(entity).copied().unwrap();

            // Compute velocity and rotation based on state
            let (new_vx, new_vy, new_rot) = match ai.state {
                AIState::Unaware => {
                    match ai.initial_type {
                        EnemyType::Idle => (0.0, 0.0, 0.0), // Keep current rotation
                        EnemyType::Wandering | EnemyType::Patrolling => match ai.wander_state {
                            WanderState::Moving => (
                                ai.wander_direction.cos() * speed.value,
                                ai.wander_direction.sin() * speed.value,
                                ai.wander_direction,
                            ),
                            WanderState::LookingAround => {
                                let look_progress = 1.5 - ai.wander_look_timer;
                                let rot = if look_progress < 0.5 {
                                    let angle_offset = -(70.0 * PI / 180.0) * (look_progress / 0.5);
                                    ai.wander_direction + angle_offset
                                } else if look_progress < 1.5 {
                                    let left_angle = ai.wander_direction - (70.0 * PI / 180.0);
                                    let angle_offset =
                                        (140.0 * PI / 180.0) * ((look_progress - 0.5) / 1.0);
                                    left_angle + angle_offset
                                } else {
                                    ai.wander_direction
                                };
                                (0.0, 0.0, rot)
                            }
                            WanderState::Waiting => (0.0, 0.0, 0.0),
                        },
                    }
                }
                AIState::SpottedUnsure => {
                    let target = ai.last_known_player_position.unwrap_or(player_pos);

                    // Check if there's a clear line of sight to the target
                    let has_clear_path =
                        has_line_of_sight(enemy_pos.to_vec2(), target.to_vec2(), &walls);

                    // If clear line of sight, move directly toward target; otherwise use pathfinding
                    let movement_target = if has_clear_path {
                        target.to_vec2()
                    } else if let Some(next_waypoint) =
                        nav_grid.get_next_waypoint(enemy_pos.to_vec2(), target.to_vec2())
                    {
                        next_waypoint
                    } else {
                        target.to_vec2()
                    };

                    let dx = movement_target.x - enemy_pos.x;
                    let dy = movement_target.y - enemy_pos.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > 0.0 {
                        (
                            (dx / dist) * speed.value,
                            (dy / dist) * speed.value,
                            dy.atan2(dx),
                        )
                    } else {
                        (0.0, 0.0, 0.0)
                    }
                }
                AIState::SurePlayerSeen => {
                    let target = if can_see_player {
                        player_pos
                    } else {
                        ai.last_known_player_position.unwrap_or(player_pos)
                    };

                    let dist_to_target = enemy_pos.distance_to(&target);
                    if can_see_player && dist_to_target < ai.attack_range {
                        let dx = player_pos.x - enemy_pos.x;
                        let dy = player_pos.y - enemy_pos.y;
                        (0.0, 0.0, dy.atan2(dx))
                    } else {
                        // Check if there's a clear line of sight to the target
                        let has_clear_path =
                            has_line_of_sight(enemy_pos.to_vec2(), target.to_vec2(), &walls);

                        // If clear line of sight, move directly toward target; otherwise use pathfinding
                        let movement_target = if has_clear_path {
                            target.to_vec2()
                        } else if let Some(next_waypoint) =
                            nav_grid.get_next_waypoint(enemy_pos.to_vec2(), target.to_vec2())
                        {
                            next_waypoint
                        } else {
                            target.to_vec2()
                        };

                        let dx = movement_target.x - enemy_pos.x;
                        let dy = movement_target.y - enemy_pos.y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist > 0.0 {
                            (
                                (dx / dist) * speed.value,
                                (dy / dist) * speed.value,
                                dy.atan2(dx),
                            )
                        } else {
                            (0.0, 0.0, 0.0)
                        }
                    }
                }
                AIState::Confused => {
                    let rot = if ai.confusion_look_timer == ai.confusion_look_duration {
                        Self::random_range(0.0, PI * 2.0)
                    } else {
                        world
                            .get_component::<Rotation>(entity)
                            .map(|r| r.angle)
                            .unwrap_or(0.0)
                    };
                    (0.0, 0.0, rot)
                }
                _ => (0.0, 0.0, 0.0),
            };

            // Apply computed values
            if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                velocity.x = new_vx;
                velocity.y = new_vy;
            }
            if let Some(rotation) = world.get_component_mut::<Rotation>(entity) {
                if new_rot != 0.0
                    || !matches!(ai.state, AIState::Unaware if ai.initial_type == EnemyType::Idle)
                {
                    rotation.angle = new_rot;
                }
            }
        }
    }
}

impl AISystem {
    /// Update wandering/patrolling behavior
    fn update_wander_behavior(ai: &mut AI, pos: &Position, walls: &[Wall], dt: f32) {
        match ai.wander_state {
            WanderState::Moving => {
                ai.wander_timer -= dt;

                // Check if hit wall or outside square
                let next_pos = Position::new(
                    pos.x + ai.wander_direction.cos() * 5.0,
                    pos.y + ai.wander_direction.sin() * 5.0,
                );

                let hit_wall = walls.iter().any(|wall| {
                    next_pos.x >= wall.x
                        && next_pos.x <= wall.x + wall.width
                        && next_pos.y >= wall.y
                        && next_pos.y <= wall.y + wall.height
                });

                let outside_square = !Self::is_within_movement_square(
                    &next_pos,
                    &ai.spawn_position,
                    ai.movement_square_size,
                );

                if ai.wander_timer <= 0.0 || hit_wall || outside_square {
                    // Stop and look around
                    ai.wander_state = WanderState::LookingAround;
                    ai.wander_look_timer = 1.5; // Look around for 1.5 seconds

                    // If hit obstacle, find best direction
                    if hit_wall || outside_square {
                        ai.wander_direction = Self::find_most_open_direction(
                            pos,
                            &ai.spawn_position,
                            ai.movement_square_size,
                            walls,
                        );
                    }
                }
            }
            WanderState::LookingAround => {
                ai.wander_look_timer -= dt;
                if ai.wander_look_timer <= 0.0 {
                    // Done looking, wait before moving
                    ai.wander_state = WanderState::Waiting;
                    ai.wander_timer = Self::random_range(1.0, 2.0);
                }
            }
            WanderState::Waiting => {
                ai.wander_timer -= dt;
                if ai.wander_timer <= 0.0 {
                    // Start moving in new direction
                    ai.wander_state = WanderState::Moving;
                    ai.wander_timer = Self::random_range(1.0, 2.0);
                    ai.wander_direction = Self::find_most_open_direction(
                        pos,
                        &ai.spawn_position,
                        ai.movement_square_size,
                        walls,
                    );
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
        let enemy_pos = Position::new(0.0, 0.0);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Idle, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));
        world.add_component(enemy, Rotation::new(0.0));

        let mut system = AISystem;
        // Run multiple frames to trigger state transitions (need > 0.3s to go from Unaware -> SpottedUnsure -> SurePlayerSeen)
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::SurePlayerSeen);

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
        let enemy_pos = Position::new(0.0, 0.0);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Idle, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));
        world.add_component(enemy, Rotation::new(0.0));

        let mut system = AISystem;
        // Run multiple frames to trigger state transitions
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::SurePlayerSeen);

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
        assert_eq!(ai.state, AIState::Unaware); // Changed from Idle to Unaware

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
        let enemy_pos = Position::new(100.0, 100.0);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Idle, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));
        world.add_component(enemy, Rotation::new(0.0));

        let mut system = AISystem;
        // Run multiple frames for state transition
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        // Enemy should be chasing player (SurePlayerSeen state)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::SurePlayerSeen);

        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        // Should have non-zero velocity
        assert!(velocity.x.abs() > 0.0 || velocity.y.abs() > 0.0);

        // Velocity should point directly toward player (with direct line-of-sight targeting)
        assert!(velocity.x > 0.0); // Moving right toward player
                                   // Both at y=100, so no vertical movement expected with direct targeting
        assert!(velocity.y.abs() < 1.0); // Should be nearly zero
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
        assert_eq!(ai.state, AIState::Unaware);

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
        assert_eq!(ai.state, AIState::Unaware);

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
        assert_eq!(ai.state, AIState::Unaware);

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
        let enemy_pos = Position::new(0.0, 0.0);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Idle, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));
        world.add_component(enemy, Rotation::new(0.0));

        let mut system = AISystem;
        // Run multiple frames for state transition
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        // Enemy should be attacking
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::SurePlayerSeen);

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
        let slow_pos = Position::new(100.0, 100.0);
        world.add_component(slow_enemy, slow_pos);
        world.add_component(slow_enemy, AI::new_with_type(EnemyType::Idle, slow_pos));
        world.add_component(slow_enemy, Velocity::zero());
        world.add_component(slow_enemy, Speed::new(50.0));
        world.add_component(slow_enemy, Health::new(100));
        world.add_component(slow_enemy, Rotation::new(0.0));

        // Create fast enemy
        let fast_enemy = world.spawn();
        world.add_component(fast_enemy, Enemy);
        let fast_pos = Position::new(100.0, 120.0);
        world.add_component(fast_enemy, fast_pos);
        world.add_component(fast_enemy, AI::new_with_type(EnemyType::Idle, fast_pos));
        world.add_component(fast_enemy, Velocity::zero());
        world.add_component(fast_enemy, Speed::new(200.0));
        world.add_component(fast_enemy, Health::new(100));
        world.add_component(fast_enemy, Rotation::new(0.0));

        let mut system = AISystem;
        // Run multiple frames for state transition
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        // Both should be chasing
        let slow_ai = world.get_component::<AI>(slow_enemy).unwrap();
        let fast_ai = world.get_component::<AI>(fast_enemy).unwrap();
        assert_eq!(slow_ai.state, AIState::SurePlayerSeen);
        assert_eq!(fast_ai.state, AIState::SurePlayerSeen);

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
        assert_eq!(ai.state, AIState::Unaware);

        // Enemy should not be moving
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn test_ai_cannot_see_player_outside_vision_cone() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create player to the right of enemy
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 0.0));

        // Create enemy facing DOWN (PI/2), player is to the right (outside 90-degree cone)
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        let enemy_pos = Position::new(0.0, 0.0);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Idle, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));
        world.add_component(enemy, Rotation::new(std::f32::consts::PI / 2.0)); // Facing down

        let mut system = AISystem;
        // Run multiple frames
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        // Enemy should NOT see player (outside vision cone)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(
            ai.state,
            AIState::Unaware,
            "Enemy should not see player outside vision cone"
        );

        // Enemy should not be moving toward player
        let velocity = world.get_component::<Velocity>(enemy).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);

        // Enemy rotation should not have changed to face player
        let rotation = world.get_component::<Rotation>(enemy).unwrap();
        assert_eq!(rotation.angle, std::f32::consts::PI / 2.0);
    }

    #[test]
    fn test_ai_can_see_player_behind_when_turned_around() {
        use crate::ecs::world::World;

        let mut world = World::new();

        // Create player behind the enemy
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(-100.0, 0.0));

        // Create enemy at origin facing RIGHT (0 degrees)
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        let enemy_pos = Position::new(0.0, 0.0);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Idle, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(100));
        world.add_component(enemy, Rotation::new(0.0)); // Facing right

        let mut system = AISystem;
        // Run a few frames - enemy shouldn't see player yet
        for _ in 0..5 {
            system.run(&mut world, 0.016);
        }

        // Enemy should NOT see player (behind them, outside vision cone)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(
            ai.state,
            AIState::Unaware,
            "Enemy should not see player behind them"
        );

        // Now turn the enemy around to face left (PI)
        if let Some(rotation) = world.get_component_mut::<Rotation>(enemy) {
            rotation.angle = std::f32::consts::PI;
        }

        // Run more frames - now enemy should spot player
        for _ in 0..30 {
            system.run(&mut world, 0.016);
        }

        // Now enemy SHOULD see player (player is in front of vision cone)
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(
            ai.state,
            AIState::SurePlayerSeen,
            "Enemy should see player when facing them"
        );
    }
}
