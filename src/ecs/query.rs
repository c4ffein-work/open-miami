use super::{Component, Entity, World};

/// Query provides iteration over entities with specific components
pub struct Query<'a, T: Component> {
    entities: Vec<Entity>,
    world: &'a World,
    _phantom: std::marker::PhantomData<T>,
}

impl<'a, T: Component> Query<'a, T> {
    pub fn new(world: &'a World) -> Self {
        Query {
            entities: world.query::<T>(),
            world,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.entities.iter().filter_map(|&entity| {
            self.world
                .get_component::<T>(entity)
                .map(|comp| (entity, comp))
        })
    }
}

/// QueryMut provides mutable iteration over entities with specific components
pub struct QueryMut<'a, T: Component> {
    entities: Vec<Entity>,
    world: &'a mut World,
    _phantom: std::marker::PhantomData<T>,
}

impl<'a, T: Component> QueryMut<'a, T> {
    pub fn new(world: &'a mut World) -> Self {
        let entities = world.query::<T>();
        QueryMut {
            entities,
            world,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        // We need to use unsafe here to work around Rust's borrow checker
        // This is safe because we're not allowing overlapping mutable borrows
        let world_ptr = self.world as *mut World;
        self.entities.iter().filter_map(move |&entity| unsafe {
            (*world_ptr)
                .get_component_mut::<T>(entity)
                .map(|comp| (entity, comp))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[test]
    fn test_query_iter() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });

        let e2 = world.spawn();
        world.add_component(e2, Position { x: 3.0, y: 4.0 });

        let query = Query::<Position>::new(&world);
        let results: Vec<_> = query.iter().collect();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_query_mut_iter() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });

        let e2 = world.spawn();
        world.add_component(e2, Position { x: 3.0, y: 4.0 });

        {
            let mut query = QueryMut::<Position>::new(&mut world);
            for (_entity, pos) in query.iter_mut() {
                pos.x += 10.0;
            }
        }

        let pos1 = world.get_component::<Position>(e1).unwrap();
        let pos2 = world.get_component::<Position>(e2).unwrap();

        assert_eq!(pos1.x, 11.0);
        assert_eq!(pos2.x, 13.0);
    }
}
