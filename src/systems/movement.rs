use crate::collision::circle_rect_collision;
use crate::components::{Knockback, Position, Radius, Velocity};
use crate::ecs::{System, World};
use crate::math::Vec2;

/// Size of the playable world (matches the rendered floor). Entities are kept
/// inside `[0, WORLD_SIZE]` on both axes so nothing (notably wandering enemies)
/// can drift out of bounds where it becomes unreachable.
const WORLD_SIZE: f32 = 2000.0;

/// Time constant for the exponential decay of a knockback impulse: it sheds
/// ~1/e of its speed every `KNOCKBACK_DECAY_TAU` seconds. With this value a
/// shove is essentially spent after ~0.2-0.3s — a snappy punch, not a slide.
const KNOCKBACK_DECAY_TAU: f32 = 0.06;

/// Once a knockback impulse drops below this speed (px/s) it is negligible, so
/// the component is removed to stop it nudging the entity forever.
const KNOCKBACK_MIN_SPEED: f32 = 8.0;

/// Hard cap on movement sub-steps per frame (safety valve for absurd speeds;
/// with it a stacked multi-kill shove still resolves in a handful of steps).
const MAX_SUBSTEPS: usize = 32;

/// A knockback impulse slamming into a wall faster than this (px/s along the
/// wall normal) BOUNCES off it instead of stopping dead. Only the tagged
/// `Knockback` vector ever bounces — ordinary walking `Velocity` still slides
/// and pins against walls — and the threshold keeps the decayed tail of a
/// spent shove from jittering against a wall it is resting on.
const BOUNCE_MIN_NORMAL_SPEED: f32 = 250.0;

/// How much of the into-wall speed survives a bounce (reflected back out).
const BOUNCE_RESTITUTION: f32 = 0.45;

/// How much of the along-wall speed survives a bounce (slight scrub).
const BOUNCE_TANGENT_DAMPING: f32 = 0.9;

/// System that applies velocity to position
pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        // Query all entities with both Position and Velocity
        let entities: Vec<_> = world.query_with::<Position, Velocity>();

        for entity in entities {
            // Get velocity, any knockback impulse, and radius (immutable
            // borrows), copy the values.
            let vel = world.get_component::<Velocity>(entity).copied();
            let knockback = world.get_component::<Knockback>(entity).copied();
            let radius = world
                .get_component::<Radius>(entity)
                .map(|r| r.value)
                .unwrap_or(0.0);

            let pos0 = world.get_component::<Position>(entity).copied();
            let (Some(pos0), Some(vel)) = (pos0, vel) else {
                continue;
            };

            // Layer the knockback impulse on top of the normal velocity so the
            // shove travels through the same wall/bounds clamping below. The
            // impulse is worked on locally: a bounce below rewrites it.
            let (mut kb_x, mut kb_y) = knockback.map(|k| (k.x, k.y)).unwrap_or((0.0, 0.0));
            let mut final_x = pos0.x;
            let mut final_y = pos0.y;

            // Sub-step the motion so a fast mover can never jump clean over a
            // thin wall in one integration step: each sub-step advances at most
            // `radius`, which guarantees the endpoint check below always finds
            // the collision and resolves it back out the side the mover came
            // from (the centre can never reach a wall's midline in one step).
            // Radius-less entities never collide with walls, so one step does.
            let speed = ((vel.x + kb_x).powi(2) + (vel.y + kb_y).powi(2)).sqrt();
            let steps = if radius > 0.0 && speed * dt > radius {
                ((speed * dt / radius).ceil() as usize).clamp(1, MAX_SUBSTEPS)
            } else {
                1
            };
            let sub_dt = dt / steps as f32;

            // Borrow the walls in place for the sub-step loop (no per-tick
            // clone): the loop only touches locals, and the shared borrow
            // ends before the component writes below.
            let walls = world.walls();

            for _ in 0..steps {
                final_x += (vel.x + kb_x) * sub_dt;
                final_y += (vel.y + kb_y) * sub_dt;

                // Check each wall and resolve collisions
                for wall in walls {
                    // Check if the new position would collide
                    if circle_rect_collision(
                        Vec2::new(final_x, final_y),
                        radius,
                        wall.x,
                        wall.y,
                        wall.width,
                        wall.height,
                    ) {
                        // Resolve collision by clamping position outside the wall
                        // Find the closest point outside the wall

                        // Check each wall edge and clamp to stay outside
                        let wall_right = wall.x + wall.width;
                        let wall_bottom = wall.y + wall.height;

                        // Calculate distances to each edge
                        let dist_to_left = final_x - (wall.x - radius);
                        let dist_to_right = (wall_right + radius) - final_x;
                        let dist_to_top = final_y - (wall.y - radius);
                        let dist_to_bottom = (wall_bottom + radius) - final_y;

                        // Find the closest edge and push out. When a knockback
                        // impulse drives the mover into the wall fast enough, it
                        // BOUNCES: the impulse component along the wall normal
                        // reflects (damped by the restitution), the tangential
                        // component is scrubbed slightly. Walking velocity is
                        // never reflected — it keeps sliding/pinning as before.
                        let min_dist = dist_to_left
                            .min(dist_to_right)
                            .min(dist_to_top)
                            .min(dist_to_bottom);

                        if min_dist == dist_to_left && dist_to_left > 0.0 {
                            final_x = wall.x - radius;
                            if kb_x > BOUNCE_MIN_NORMAL_SPEED {
                                kb_x = -kb_x * BOUNCE_RESTITUTION;
                                kb_y *= BOUNCE_TANGENT_DAMPING;
                            }
                        } else if min_dist == dist_to_right && dist_to_right > 0.0 {
                            final_x = wall_right + radius;
                            if kb_x < -BOUNCE_MIN_NORMAL_SPEED {
                                kb_x = -kb_x * BOUNCE_RESTITUTION;
                                kb_y *= BOUNCE_TANGENT_DAMPING;
                            }
                        } else if min_dist == dist_to_top && dist_to_top > 0.0 {
                            final_y = wall.y - radius;
                            if kb_y > BOUNCE_MIN_NORMAL_SPEED {
                                kb_y = -kb_y * BOUNCE_RESTITUTION;
                                kb_x *= BOUNCE_TANGENT_DAMPING;
                            }
                        } else if min_dist == dist_to_bottom && dist_to_bottom > 0.0 {
                            final_y = wall_bottom + radius;
                            if kb_y < -BOUNCE_MIN_NORMAL_SPEED {
                                kb_y = -kb_y * BOUNCE_RESTITUTION;
                                kb_x *= BOUNCE_TANGENT_DAMPING;
                            }
                        }
                    }
                }
            }

            // Keep entities within the world bounds.
            if let Some(pos) = world.get_component_mut::<Position>(entity) {
                pos.x = final_x.clamp(0.0, WORLD_SIZE);
                pos.y = final_y.clamp(0.0, WORLD_SIZE);
            }

            // Write back the (possibly bounced) impulse, decay it
            // (frame-rate independent), and drop it once it is spent so it
            // stops nudging the entity forever.
            if knockback.is_some() {
                let decay = (-dt / KNOCKBACK_DECAY_TAU).exp();
                let mut spent = false;
                if let Some(kb) = world.get_component_mut::<Knockback>(entity) {
                    kb.x = kb_x * decay;
                    kb.y = kb_y * decay;
                    spent = kb.magnitude() < KNOCKBACK_MIN_SPEED;
                }
                if spent {
                    world.remove_component::<Knockback>(entity);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_movement_system_applies_velocity() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position::new(0.0, 0.0));
        world.add_component(entity, Velocity::new(100.0, 50.0));

        let mut system = MovementSystem;
        system.run(&mut world, 1.0); // 1 second

        let pos = world.get_component::<Position>(entity).unwrap();
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 50.0);
    }

    #[test]
    fn test_movement_system_with_dt() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position::new(10.0, 20.0));
        world.add_component(entity, Velocity::new(100.0, 200.0));

        let mut system = MovementSystem;
        system.run(&mut world, 0.5); // Half second

        let pos = world.get_component::<Position>(entity).unwrap();
        assert_eq!(pos.x, 60.0);
        assert_eq!(pos.y, 120.0);
    }

    #[test]
    fn test_movement_system_multiple_entities() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position::new(0.0, 0.0));
        world.add_component(e1, Velocity::new(10.0, 0.0));

        let e2 = world.spawn();
        world.add_component(e2, Position::new(0.0, 0.0));
        world.add_component(e2, Velocity::new(0.0, 20.0));

        let mut system = MovementSystem;
        system.run(&mut world, 1.0);

        let pos1 = world.get_component::<Position>(e1).unwrap();
        let pos2 = world.get_component::<Position>(e2).unwrap();

        assert_eq!(pos1.x, 10.0);
        assert_eq!(pos1.y, 0.0);
        assert_eq!(pos2.x, 0.0);
        assert_eq!(pos2.y, 20.0);
    }

    #[test]
    fn test_knockback_shoves_then_decays() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position::new(100.0, 100.0));
        world.add_component(entity, Velocity::zero());
        world.add_component(entity, Knockback::new(600.0, 0.0));

        let mut system = MovementSystem;

        // One frame of a punchy shove moves the entity along +x on top of its
        // (zero) velocity.
        system.run(&mut world, 0.016);
        let pos = world.get_component::<Position>(entity).unwrap();
        assert!(pos.x > 100.0, "knockback should move the entity");
        assert_eq!(pos.y, 100.0);
        // ...and the impulse is weaker afterwards (decaying).
        let kb = world.get_component::<Knockback>(entity).unwrap();
        assert!(kb.x < 600.0 && kb.x > 0.0);

        // After enough time the impulse is spent and the component is dropped so
        // it stops nudging the entity forever.
        for _ in 0..40 {
            system.run(&mut world, 0.016);
        }
        assert!(!world.has_component::<Knockback>(entity));
    }

    #[test]
    fn test_movement_system_ignores_entities_without_velocity() {
        let mut world = World::new();

        let entity = world.spawn();
        world.add_component(entity, Position::new(10.0, 20.0));
        // No velocity component

        let mut system = MovementSystem;
        system.run(&mut world, 1.0);

        let pos = world.get_component::<Position>(entity).unwrap();
        assert_eq!(pos.x, 10.0);
        assert_eq!(pos.y, 20.0); // Position unchanged
    }
}
