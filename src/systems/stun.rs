use crate::components::{Stunned, Velocity};
use crate::ecs::{Entity, System, World};

/// System that ticks down knockdown timers. While stunned, an entity is held in
/// place (velocity zeroed); when the timer runs out the `Stunned` marker is
/// removed and the entity resumes normal behaviour.
pub struct StunSystem;

impl System for StunSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let mut expired = Vec::new();

        for entity in world.query::<Stunned>() {
            // Keep stunned entities pinned in place.
            if let Some(velocity) = world.get_component_mut::<Velocity>(entity) {
                velocity.x = 0.0;
                velocity.y = 0.0;
            }

            if let Some(stun) = world.get_component_mut::<Stunned>(entity) {
                stun.timer -= dt;
                if !stun.is_active() {
                    expired.push(entity);
                }
            }
        }

        for entity in expired {
            world.remove_component::<Stunned>(entity);
        }
    }
}

impl StunSystem {
    /// Convenience helper mirroring what the system does, for callers that only
    /// need to know whether an entity is currently incapacitated.
    pub fn is_stunned(world: &World, entity: Entity) -> bool {
        world
            .get_component::<Stunned>(entity)
            .map(|s| s.is_active())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stun_expires_after_duration() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Stunned::new(1.0));
        world.add_component(e, Velocity::new(50.0, 50.0));

        let mut system = StunSystem;

        system.run(&mut world, 0.5);
        assert!(world.has_component::<Stunned>(e)); // still stunned
                                                    // Velocity pinned to zero while stunned.
        let v = world.get_component::<Velocity>(e).unwrap();
        assert_eq!(v.x, 0.0);
        assert_eq!(v.y, 0.0);

        system.run(&mut world, 0.6); // total 1.1s > 1.0s
        assert!(!world.has_component::<Stunned>(e)); // recovered
    }

    #[test]
    fn test_is_stunned_helper() {
        let mut world = World::new();
        let e = world.spawn();
        assert!(!StunSystem::is_stunned(&world, e));
        world.add_component(e, Stunned::new(2.0));
        assert!(StunSystem::is_stunned(&world, e));
    }
}
