use crate::components::{Position, Velocity};
use crate::ecs::{System, World};

/// System that applies velocity to position
pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        // Query all entities with both Position and Velocity
        let entities: Vec<_> = world.query_with::<Position, Velocity>();

        for entity in entities {
            // Get velocity first (immutable borrow), copy the values
            let vel = world.get_component::<Velocity>(entity).copied();

            // Then get position (mutable borrow) and update it
            if let (Some(pos), Some(vel)) = (
                world.get_component_mut::<Position>(entity),
                vel,
            ) {
                pos.x += vel.x * dt;
                pos.y += vel.y * dt;
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
