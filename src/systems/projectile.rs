use crate::components::ProjectileTrail;
use crate::ecs::{System, World};

/// System that updates and removes projectile trails
pub struct ProjectileTrailSystem;

impl System for ProjectileTrailSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let entities: Vec<_> = world.query::<ProjectileTrail>();

        // Update lifetimes and collect dead entities
        let mut dead_entities = Vec::new();

        for entity in entities {
            if let Some(trail) = world.get_component_mut::<ProjectileTrail>(entity) {
                trail.lifetime -= dt;

                if !trail.is_alive() {
                    dead_entities.push(entity);
                }
            }
        }

        // Remove dead trails
        for entity in dead_entities {
            world.despawn(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Position;

    #[test]
    fn test_projectile_trail_system_decreases_lifetime() {
        let mut world = World::new();
        let entity = world.spawn();

        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 100.0);
        world.add_component(entity, ProjectileTrail::new(start, end));

        let mut system = ProjectileTrailSystem;
        system.run(&mut world, 0.1);

        let trail = world.get_component::<ProjectileTrail>(entity).unwrap();
        assert!((trail.lifetime - 0.05).abs() < 0.001); // 0.15 - 0.1 = 0.05
    }

    #[test]
    fn test_projectile_trail_system_removes_dead_trails() {
        let mut world = World::new();
        let entity = world.spawn();

        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 100.0);
        world.add_component(entity, ProjectileTrail::new(start, end));

        let mut system = ProjectileTrailSystem;
        // Run for longer than trail lifetime
        system.run(&mut world, 0.2);

        // Trail should be removed
        assert!(world.get_component::<ProjectileTrail>(entity).is_none());
    }

    #[test]
    fn test_projectile_trail_system_multiple_trails() {
        let mut world = World::new();

        // Create multiple trails
        for i in 0..3 {
            let entity = world.spawn();
            let start = Position::new(i as f32 * 10.0, 0.0);
            let end = Position::new(i as f32 * 10.0 + 100.0, 100.0);
            world.add_component(entity, ProjectileTrail::new(start, end));
        }

        let mut system = ProjectileTrailSystem;
        system.run(&mut world, 0.1);

        // All trails should still exist
        let trails = world.query::<ProjectileTrail>();
        assert_eq!(trails.len(), 3);

        // Run until all trails are dead
        system.run(&mut world, 0.2);

        // All trails should be removed
        let trails = world.query::<ProjectileTrail>();
        assert_eq!(trails.len(), 0);
    }
}
