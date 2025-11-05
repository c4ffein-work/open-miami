use super::World;

/// System trait for implementing game logic
pub trait System {
    fn run(&mut self, world: &mut World, dt: f32);
}

/// Helper to create function-based systems
pub struct FnSystem<F>
where
    F: FnMut(&mut World, f32),
{
    func: F,
}

impl<F> FnSystem<F>
where
    F: FnMut(&mut World, f32),
{
    pub fn new(func: F) -> Self {
        FnSystem { func }
    }
}

impl<F> System for FnSystem<F>
where
    F: FnMut(&mut World, f32),
{
    fn run(&mut self, world: &mut World, dt: f32) {
        (self.func)(world, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Counter {
        value: i32,
    }

    #[test]
    fn test_fn_system() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Counter { value: 0 });

        let mut system = FnSystem::new(|world: &mut World, _dt: f32| {
            let entities: Vec<_> = world.query::<Counter>();
            for entity in entities {
                if let Some(counter) = world.get_component_mut::<Counter>(entity) {
                    counter.value += 1;
                }
            }
        });

        system.run(&mut world, 0.016);

        let counter = world.get_component::<Counter>(entity).unwrap();
        assert_eq!(counter.value, 1);
    }

    #[test]
    fn test_system_multiple_runs() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Counter { value: 0 });

        let mut system = FnSystem::new(|world: &mut World, _dt: f32| {
            let entities: Vec<_> = world.query::<Counter>();
            for entity in entities {
                if let Some(counter) = world.get_component_mut::<Counter>(entity) {
                    counter.value += 1;
                }
            }
        });

        for _ in 0..10 {
            system.run(&mut world, 0.016);
        }

        let counter = world.get_component::<Counter>(entity).unwrap();
        assert_eq!(counter.value, 10);
    }
}
