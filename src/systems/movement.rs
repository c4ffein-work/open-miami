use crate::collision::circle_rect_collision;
use crate::components::{Position, Radius, Velocity};
use crate::ecs::{System, World};
use crate::math::Vec2;

/// Size of the playable world (matches the rendered floor). Entities are kept
/// inside `[0, WORLD_SIZE]` on both axes so nothing (notably wandering enemies)
/// can drift out of bounds where it becomes unreachable.
const WORLD_SIZE: f32 = 2000.0;

/// System that applies velocity to position
pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        // Query all entities with both Position and Velocity
        let entities: Vec<_> = world.query_with::<Position, Velocity>();

        // Get walls once before processing entities (to avoid borrow checker issues)
        let walls: Vec<_> = world.walls().to_vec();

        for entity in entities {
            // Get velocity and radius (immutable borrows), copy the values
            let vel = world.get_component::<Velocity>(entity).copied();
            let radius = world
                .get_component::<Radius>(entity)
                .map(|r| r.value)
                .unwrap_or(0.0);

            // Then get position (mutable borrow) and update it
            if let (Some(pos), Some(vel)) = (world.get_component_mut::<Position>(entity), vel) {
                // Calculate desired new position
                let new_x = pos.x + vel.x * dt;
                let new_y = pos.y + vel.y * dt;

                let mut final_x = new_x;
                let mut final_y = new_y;

                // Check each wall and resolve collisions
                for wall in &walls {
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

                        // Find the closest edge and push out
                        let min_dist = dist_to_left
                            .min(dist_to_right)
                            .min(dist_to_top)
                            .min(dist_to_bottom);

                        if min_dist == dist_to_left && dist_to_left > 0.0 {
                            final_x = wall.x - radius;
                        } else if min_dist == dist_to_right && dist_to_right > 0.0 {
                            final_x = wall_right + radius;
                        } else if min_dist == dist_to_top && dist_to_top > 0.0 {
                            final_y = wall.y - radius;
                        } else if min_dist == dist_to_bottom && dist_to_bottom > 0.0 {
                            final_y = wall_bottom + radius;
                        }
                    }
                }

                // Keep entities within the world bounds.
                pos.x = final_x.clamp(0.0, WORLD_SIZE);
                pos.y = final_y.clamp(0.0, WORLD_SIZE);
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
