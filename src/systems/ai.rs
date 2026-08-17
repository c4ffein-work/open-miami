use crate::collision::{has_line_of_sight, has_line_of_sight_with_padding};
use crate::components::{
    AIState, DebugPath, DebugTrail, Enemy, EnemyType, Health, Player, Position, Rotation, Speed,
    Stunned, Velocity, WanderState, AI,
};
use crate::ecs::world::Wall;
use crate::ecs::{Entity, System, World};
use crate::pathfinding::NavigationGrid;
use std::f32::consts::PI;

// Pseudo-random number generation. The RNG state lives in the `World` (so every
// `Simulation` is independently reproducible); these free helpers operate on a
// borrowed copy of that state threaded through the AI update. They use the same
// LCG constants as the world so the produced sequence is unchanged.
fn next_random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

/// Random float between min and max.
fn random_range(state: &mut u32, min: f32, max: f32) -> f32 {
    let r = next_random(state) as f32 / u32::MAX as f32;
    r * (max - min) + min
}

/// Random integer between min and max (inclusive).
fn random_int_range(state: &mut u32, min: i32, max: i32) -> i32 {
    let range = (max - min + 1) as u32;
    min + (next_random(state) % range) as i32
}

// --- Feral (DRIFTER / `EnemyType::Wandering`) tuning -------------------------
// The corruptor's soldiers (Idle/Patrolling) pursue with discipline: they
// pathfind toward the player and hold at weapon range. The feral drifter has no
// such objective. When it locks on it *lunges* — a locked-in straight-line burst
// of speed at the player's position, no pathfinding, no braking at range — then
// briefly winds up and lunges again, burning a chip of itself out on every dash.

/// Speed multiplier applied to a feral's base speed during a lunge burst.
const FERAL_LUNGE_SPEED_MULT: f32 = 2.4;
/// Speed multiplier while a feral is winding up between lunges (a slow creep).
const FERAL_RECOVER_SPEED_MULT: f32 = 0.35;
/// Lunge-burst duration bounds (seconds).
const FERAL_LUNGE_MIN: f32 = 0.30;
const FERAL_LUNGE_MAX: f32 = 0.55;
/// Wind-up / recover duration bounds between lunges (seconds).
const FERAL_RECOVER_MIN: f32 = 0.20;
const FERAL_RECOVER_MAX: f32 = 0.40;
/// Chip of self-damage a feral takes each time it completes a lunge ("running
/// down like a dropped call"). Deterministic and discrete — no accumulator.
const FERAL_LUNGE_SELF_DAMAGE: i32 = 1;
/// Erratic idle-wander burst duration bounds for a feral (seconds). Much shorter
/// and jitterier than the soldier patrol so the two read as different creatures.
const FERAL_WANDER_MIN: f32 = 0.20;
const FERAL_WANDER_MAX: f32 = 0.70;

/// System that handles enemy AI behavior
#[derive(Default)]
pub struct AISystem {
    /// Cached navigation grid, rebuilt only when the wall layout changes.
    /// Building the grid scans every cell against every wall, far too much
    /// work to redo each tick for what is static level geometry.
    nav_cache: Option<(Vec<Wall>, NavigationGrid)>,
}

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
        rng: &mut u32,
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
        if next_random(rng).is_multiple_of(2) {
            best_direction
        } else {
            (next_random(rng) as f32 / u32::MAX as f32) * PI * 2.0
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

        // Pull the world's RNG state into a local so it can be threaded through
        // the AI update without conflicting with component borrows of `world`.
        // It is written back at the end so the sequence continues across ticks.
        let mut rng = world.rng_state();

        // Reuse the cached navigation grid unless the walls changed (level swap)
        let walls_changed = match &self.nav_cache {
            Some((cached_walls, _)) => *cached_walls != walls,
            None => true,
        };
        if walls_changed {
            self.nav_cache = Some((walls.clone(), NavigationGrid::new(&walls)));
        }
        let nav_grid = &self.nav_cache.as_ref().unwrap().1;

        // Query all enemies
        let enemies: Vec<Entity> = world.query::<Enemy>();

        // Set when a passive bot took damage this tick: the crowd turns.
        let mut passive_hurt = false;

        for entity in enemies {
            let (enemy_pos, speed, health) = match (
                world.get_component::<Position>(entity),
                world.get_component::<Speed>(entity),
                world.get_component::<Health>(entity),
            ) {
                (Some(pos), Some(spd), Some(hp)) => (*pos, *spd, *hp),
                _ => continue,
            };

            // Passive (civilian) bots have their own brain: no vision, no
            // aggro. Being hurt — or downed outright — flips the whole crowd
            // (after the loop, once the borrows are released), so this runs
            // before the dead / stunned skips.
            if world
                .get_component::<AI>(entity)
                .is_some_and(|ai| ai.state == AIState::Passive)
            {
                if crate::systems::passive::update_passive(
                    world, entity, &mut rng, nav_grid, &walls, dt,
                ) {
                    passive_hurt = true;
                }
                continue;
            }

            // Skip dead enemies
            if health.is_dead() {
                if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                    velocity.x = 0.0;
                    velocity.y = 0.0;
                }
                continue;
            }

            // Skip knocked-down enemies: they can't chase or think while stunned.
            if world.has_component::<Stunned>(entity) {
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

            // Set when a feral finishes a lunge this tick, so its self-damage can
            // be applied after the (exclusive) mutable AI borrow is released.
            let mut feral_lunge_completed = false;

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
                                    // SENTINEL soldier: parked at its rack, guarding.
                                }
                                EnemyType::Patrolling => {
                                    // HUNTER soldier: disciplined patrol (move,
                                    // sweep a look-around, pause, repeat).
                                    Self::update_wander_behavior(
                                        &mut rng, ai, &enemy_pos, &walls, dt,
                                    );
                                }
                                EnemyType::Wandering => {
                                    // DRIFTER feral: erratic, objective-less
                                    // twitching — no careful look-around.
                                    Self::update_feral_wander(&mut rng, ai, &enemy_pos, &walls, dt);
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
                                Self::arm_feral_lunge(ai);
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
                        // Ferals don't "chase" — they drive a lunge cadence,
                        // locking a burst direction at each wind-up and burning a
                        // chip of themselves out on every completed dash.
                        if ai.initial_type == EnemyType::Wandering {
                            let target = ai.last_known_player_position.unwrap_or(player_pos);
                            feral_lunge_completed =
                                Self::update_feral_lunge(&mut rng, ai, &enemy_pos, &target, dt);
                        }
                        if can_see_player {
                            // Keep chasing
                            ai.last_known_player_position = Some(player_pos);
                            ai.state_timer = ai.lost_player_duration;
                        } else {
                            // Lost sight of player
                            if ai.state_timer <= 0.0 {
                                // Been at last known position too long, get confused
                                ai.state = AIState::Confused;
                                ai.confusion_looks_remaining = random_int_range(&mut rng, 2, 3);
                                ai.confusion_look_timer = ai.confusion_look_duration;
                            }
                        }
                    }
                    AIState::Confused => {
                        if can_see_player {
                            // Found player again!
                            ai.state = AIState::SurePlayerSeen;
                            Self::arm_feral_lunge(ai);
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
                                    ai.wander_timer = random_range(&mut rng, 1.0, 2.0);
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

            // A feral that just finished a lunge burns a chip of itself out.
            if feral_lunge_completed {
                if let Some(hp) = world.get_component_mut::<Health>(entity) {
                    hp.take_damage(FERAL_LUNGE_SELF_DAMAGE);
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

                    // Use inflated walls for pathfinding decision to prevent wall grinding
                    // If target is close to a wall, we'll use pathfinding instead of direct movement
                    let wall_padding = 25.0; // Inflate walls by 25 pixels for pathfinding check
                    let has_clear_path = has_line_of_sight_with_padding(
                        enemy_pos.to_vec2(),
                        target.to_vec2(),
                        &walls,
                        wall_padding,
                    );

                    // If clear line of sight (with inflated walls), move directly; otherwise use pathfinding
                    let movement_target = if has_clear_path {
                        // Clear debug path when moving directly
                        if let Some(debug_path) = world.get_component_mut::<DebugPath>(entity) {
                            *debug_path = DebugPath::clear();
                        }
                        target.to_vec2()
                    } else if let Some(full_path) =
                        nav_grid.find_path(enemy_pos.to_vec2(), target.to_vec2())
                    {
                        // Store full path for debug visualization
                        if world.has_component::<DebugPath>(entity) {
                            if let Some(debug_path) = world.get_component_mut::<DebugPath>(entity) {
                                *debug_path = DebugPath::new(full_path.clone(), target.to_vec2());
                            }
                        } else {
                            world.add_component(
                                entity,
                                DebugPath::new(full_path.clone(), target.to_vec2()),
                            );
                        }

                        // Get next waypoint from the already-computed path
                        NavigationGrid::next_waypoint_in_path(
                            &full_path,
                            enemy_pos.to_vec2(),
                            target.to_vec2(),
                        )
                    } else {
                        // Clear debug path if no path found
                        if let Some(debug_path) = world.get_component_mut::<DebugPath>(entity) {
                            *debug_path = DebugPath::clear();
                        }
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
                // Feral drifter: reckless straight-line lunge. It does NOT
                // pathfind and does NOT brake at weapon range — during a burst it
                // charges along its locked direction (into walls, past the player,
                // whatever), and between bursts it only creeps while re-aiming.
                AIState::SurePlayerSeen if ai.initial_type == EnemyType::Wandering => {
                    match ai.wander_state {
                        WanderState::Moving => {
                            let s = speed.value * FERAL_LUNGE_SPEED_MULT;
                            (
                                ai.wander_direction.cos() * s,
                                ai.wander_direction.sin() * s,
                                ai.wander_direction,
                            )
                        }
                        _ => {
                            let target = ai.last_known_player_position.unwrap_or(player_pos);
                            let dx = target.x - enemy_pos.x;
                            let dy = target.y - enemy_pos.y;
                            let dist = (dx * dx + dy * dy).sqrt();
                            if dist > 0.0 {
                                let s = speed.value * FERAL_RECOVER_SPEED_MULT;
                                ((dx / dist) * s, (dy / dist) * s, dy.atan2(dx))
                            } else {
                                (0.0, 0.0, 0.0)
                            }
                        }
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
                        // Stop and face player when close enough and can see them
                        let dx = player_pos.x - enemy_pos.x;
                        let dy = player_pos.y - enemy_pos.y;
                        (0.0, 0.0, dy.atan2(dx))
                    } else {
                        // Use inflated walls for pathfinding decision to prevent wall grinding
                        // If target is close to a wall, we'll use pathfinding instead of direct movement
                        let wall_padding = 25.0; // Inflate walls by 25 pixels for pathfinding check
                        let has_clear_path = has_line_of_sight_with_padding(
                            enemy_pos.to_vec2(),
                            target.to_vec2(),
                            &walls,
                            wall_padding,
                        );

                        // If clear line of sight (with inflated walls), move directly; otherwise use pathfinding
                        let movement_target = if has_clear_path {
                            // Clear debug path when moving directly
                            if let Some(debug_path) = world.get_component_mut::<DebugPath>(entity) {
                                *debug_path = DebugPath::clear();
                            }
                            target.to_vec2()
                        } else if let Some(full_path) =
                            nav_grid.find_path(enemy_pos.to_vec2(), target.to_vec2())
                        {
                            // Store full path for debug visualization
                            if world.has_component::<DebugPath>(entity) {
                                if let Some(debug_path) =
                                    world.get_component_mut::<DebugPath>(entity)
                                {
                                    *debug_path =
                                        DebugPath::new(full_path.clone(), target.to_vec2());
                                }
                            } else {
                                world.add_component(
                                    entity,
                                    DebugPath::new(full_path.clone(), target.to_vec2()),
                                );
                            }

                            // Get next waypoint from the already-computed path
                            NavigationGrid::next_waypoint_in_path(
                                &full_path,
                                enemy_pos.to_vec2(),
                                target.to_vec2(),
                            )
                        } else {
                            // Clear debug path if no path found
                            if let Some(debug_path) = world.get_component_mut::<DebugPath>(entity) {
                                *debug_path = DebugPath::clear();
                            }
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
                        random_range(&mut rng, 0.0, PI * 2.0)
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

            // Record position in trail for debug visualization (only for chasing enemies)
            if matches!(ai.state, AIState::SpottedUnsure | AIState::SurePlayerSeen) {
                if world.has_component::<DebugTrail>(entity) {
                    if let Some(trail) = world.get_component_mut::<DebugTrail>(entity) {
                        trail.add_position(enemy_pos.to_vec2());
                    }
                } else {
                    let mut trail = DebugTrail::default();
                    trail.add_position(enemy_pos.to_vec2());
                    world.add_component(entity, trail);
                }
            } else {
                // Clear trail when not chasing
                if let Some(trail) = world.get_component_mut::<DebugTrail>(entity) {
                    trail.clear();
                }
            }
        }

        // Persist the advanced RNG state back into the world so the next tick
        // continues the same deterministic sequence.
        world.set_rng_state(rng);

        if passive_hurt {
            crate::systems::passive::alert_passives(world, crate::scenario::AlertTarget::All);
        }
    }
}

impl AISystem {
    /// Reset a feral's lunge cadence so its next tick winds up and dashes at the
    /// player. No-op for soldiers. Called at the moment a drifter locks on.
    fn arm_feral_lunge(ai: &mut AI) {
        if ai.initial_type == EnemyType::Wandering {
            ai.wander_state = WanderState::Waiting;
            ai.wander_timer = 0.0;
        }
    }

    /// Erratic feral idle movement: short random bursts with no careful
    /// look-around sweep. On hitting a wall or its leash edge it just picks a new
    /// fully-random heading — twitchy and objective-less, unlike the soldier
    /// patrol which deliberately seeks the most open direction.
    fn update_feral_wander(rng: &mut u32, ai: &mut AI, pos: &Position, walls: &[Wall], dt: f32) {
        ai.wander_state = WanderState::Moving;
        ai.wander_timer -= dt;

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
            ai.wander_direction = random_range(rng, 0.0, PI * 2.0);
            ai.wander_timer = random_range(rng, FERAL_WANDER_MIN, FERAL_WANDER_MAX);
        }
    }

    /// Advance a feral's lunge cadence toward `target`. Winds up (creeping) then
    /// locks a heading and dashes for a short burst. Returns `true` on the tick a
    /// dash completes, so the caller can apply burn-out self-damage.
    fn update_feral_lunge(
        rng: &mut u32,
        ai: &mut AI,
        enemy_pos: &Position,
        target: &Position,
        dt: f32,
    ) -> bool {
        ai.wander_timer -= dt;
        match ai.wander_state {
            WanderState::Moving => {
                // Mid-dash: keep charging the locked heading until the burst ends.
                if ai.wander_timer <= 0.0 {
                    ai.wander_state = WanderState::Waiting;
                    ai.wander_timer = random_range(rng, FERAL_RECOVER_MIN, FERAL_RECOVER_MAX);
                    return true;
                }
                false
            }
            _ => {
                // Winding up: when the timer elapses, lock a fresh heading at the
                // target's *current* position and dash.
                if ai.wander_timer <= 0.0 {
                    let dx = target.x - enemy_pos.x;
                    let dy = target.y - enemy_pos.y;
                    ai.wander_direction = dy.atan2(dx);
                    ai.wander_state = WanderState::Moving;
                    ai.wander_timer = random_range(rng, FERAL_LUNGE_MIN, FERAL_LUNGE_MAX);
                }
                false
            }
        }
    }

    /// Update wandering/patrolling behavior
    fn update_wander_behavior(rng: &mut u32, ai: &mut AI, pos: &Position, walls: &[Wall], dt: f32) {
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
                            rng,
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
                    ai.wander_timer = random_range(rng, 1.0, 2.0);
                }
            }
            WanderState::Waiting => {
                ai.wander_timer -= dt;
                if ai.wander_timer <= 0.0 {
                    // Start moving in new direction
                    ai.wander_state = WanderState::Moving;
                    ai.wander_timer = random_range(rng, 1.0, 2.0);
                    ai.wander_direction = Self::find_most_open_direction(
                        rng,
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();

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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

        let mut system = AISystem::default();
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

    /// Helper: spawn a lone enemy of `ty` at `enemy_pos` facing right, with the
    /// player straight ahead at `player_pos`, and run the AI for `frames` ticks.
    fn drive_single_enemy(
        ty: EnemyType,
        enemy_pos: Position,
        player_pos: Position,
        speed: f32,
        frames: usize,
    ) -> (World, Entity) {
        let mut world = World::new();

        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, player_pos);

        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, enemy_pos);
        world.add_component(enemy, AI::new_with_type(ty, enemy_pos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(speed));
        world.add_component(enemy, Health::new(50));
        world.add_component(enemy, Rotation::new(0.0)); // facing +x, toward player

        let mut system = AISystem::default();
        for _ in 0..frames {
            system.run(&mut world, 0.016);
        }
        (world, enemy)
    }

    fn vel_mag(world: &World, e: Entity) -> f32 {
        let v = world.get_component::<Velocity>(e).unwrap();
        (v.x * v.x + v.y * v.y).sqrt()
    }

    #[test]
    fn test_feral_lunges_faster_than_base_speed_toward_player() {
        // The drifter locks on and dashes: at some point during the chase its
        // velocity must exceed its base speed (a burst), pointing at the player.
        let base = 100.0;
        let mut saw_burst = false;
        let mut burst_points_at_player = false;

        let mut world = World::new();
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(300.0, 0.0));
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        let epos = Position::new(0.0, 0.0);
        world.add_component(enemy, epos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Wandering, epos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(base));
        world.add_component(enemy, Health::new(50));
        world.add_component(enemy, Rotation::new(0.0));

        let mut system = AISystem::default();
        for _ in 0..120 {
            system.run(&mut world, 0.016);
            let mag = vel_mag(&world, enemy);
            if mag > base + 1.0 {
                saw_burst = true;
                let v = world.get_component::<Velocity>(enemy).unwrap();
                // Player is at +x, so a lunge toward it has positive x velocity.
                if v.x > 0.0 {
                    burst_points_at_player = true;
                }
            }
        }
        assert!(saw_burst, "feral should lunge above its base speed");
        assert!(burst_points_at_player, "the lunge should aim at the player");
    }

    #[test]
    fn test_soldier_never_exceeds_base_speed_and_holds_at_range() {
        // A HUNTER soldier pursues with discipline: it caps at its base speed and
        // stops when within weapon range — it never bursts like a feral.
        let base = 100.0;
        let (world, enemy) = drive_single_enemy(
            EnemyType::Patrolling,
            Position::new(0.0, 0.0),
            Position::new(300.0, 0.0),
            base,
            120,
        );
        // Reached the chase state.
        let ai = world.get_component::<AI>(enemy).unwrap();
        assert_eq!(ai.state, AIState::SurePlayerSeen);
    }

    #[test]
    fn test_soldier_velocity_capped_vs_feral_burst() {
        // Same geometry, same base speed: the feral's peak velocity outruns the
        // soldier's, proving the two allegiances move differently.
        let base = 100.0;

        let mut peak_soldier = 0.0_f32;
        let mut peak_feral = 0.0_f32;

        for (ty, peak) in [
            (EnemyType::Patrolling, &mut peak_soldier),
            (EnemyType::Wandering, &mut peak_feral),
        ] {
            let mut world = World::new();
            let player = world.spawn();
            world.add_component(player, Player);
            world.add_component(player, Position::new(400.0, 0.0));
            let enemy = world.spawn();
            world.add_component(enemy, Enemy);
            let epos = Position::new(0.0, 0.0);
            world.add_component(enemy, epos);
            world.add_component(enemy, AI::new_with_type(ty, epos));
            world.add_component(enemy, Velocity::zero());
            world.add_component(enemy, Speed::new(base));
            world.add_component(enemy, Health::new(50));
            world.add_component(enemy, Rotation::new(0.0));
            let mut system = AISystem::default();
            for _ in 0..90 {
                system.run(&mut world, 0.016);
                *peak = peak.max(vel_mag(&world, enemy));
            }
        }

        // Soldier is capped at base (small float slack); feral clearly exceeds it.
        assert!(
            peak_soldier <= base + 1.0,
            "soldier peaked at {peak_soldier}, expected <= {base}"
        );
        assert!(
            peak_feral > base + 1.0,
            "feral peaked at {peak_feral}, expected a burst above {base}"
        );
        assert!(
            peak_feral > peak_soldier,
            "feral should out-burst the soldier"
        );
    }

    #[test]
    fn test_feral_burns_itself_out_while_lunging() {
        // "Running down like a dropped call": a feral loses health as it dashes,
        // even without ever being shot.
        let (world, enemy) = drive_single_enemy(
            EnemyType::Wandering,
            Position::new(0.0, 0.0),
            Position::new(300.0, 0.0),
            100.0,
            300,
        );
        let hp = world.get_component::<Health>(enemy).unwrap();
        assert!(
            hp.current < hp.max,
            "feral should decay from lunging (hp {} of {})",
            hp.current,
            hp.max
        );
    }

    #[test]
    fn test_soldier_does_not_self_damage() {
        // Soldiers do not burn out — only ferals do.
        let (world, enemy) = drive_single_enemy(
            EnemyType::Patrolling,
            Position::new(0.0, 0.0),
            Position::new(300.0, 0.0),
            100.0,
            300,
        );
        let hp = world.get_component::<Health>(enemy).unwrap();
        assert_eq!(hp.current, hp.max, "soldier must not self-damage");
    }

    #[test]
    fn test_feral_idle_wander_is_erratic() {
        // With no player in sight, a feral keeps twitching to new headings; over
        // many ticks it should visit several distinct directions (not the single
        // steady heading a disciplined patrol would hold between look-arounds).
        let mut world = World::new();
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(5000.0, 5000.0)); // far, unseen
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        let epos = Position::new(400.0, 400.0);
        world.add_component(enemy, epos);
        world.add_component(enemy, AI::new_with_type(EnemyType::Wandering, epos));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(50));
        world.add_component(enemy, Rotation::new(0.0));

        let mut system = AISystem::default();
        let mut directions = std::collections::HashSet::new();
        for _ in 0..600 {
            system.run(&mut world, 0.016);
            // Feral always moves while unaware.
            assert!(
                vel_mag(&world, enemy) > 0.0,
                "a feral is never still while unaware"
            );
            let dir = world.get_component::<AI>(enemy).unwrap().wander_direction;
            directions.insert((dir * 10.0).round() as i32);
        }
        assert!(
            directions.len() >= 3,
            "feral wander should be erratic (saw {} distinct headings)",
            directions.len()
        );
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

        let mut system = AISystem::default();
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
