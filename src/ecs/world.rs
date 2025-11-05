use super::{Component, Entity};
use std::any::{Any, TypeId};
use std::collections::HashMap;

/// ComponentStorage stores all components of a specific type
type ComponentStorage = HashMap<Entity, Box<dyn Any>>;

/// World manages all entities and their components
pub struct World {
    next_entity_id: u64,
    // Map from ComponentId to storage for that component type
    components: HashMap<TypeId, ComponentStorage>,
    // Track which entities exist
    entities: Vec<Entity>,
}

impl World {
    pub fn new() -> Self {
        World {
            next_entity_id: 0,
            components: HashMap::new(),
            entities: Vec::new(),
        }
    }

    /// Create a new entity
    pub fn spawn(&mut self) -> Entity {
        let entity = Entity::new(self.next_entity_id);
        self.next_entity_id += 1;
        self.entities.push(entity);
        entity
    }

    /// Add a component to an entity
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        let type_id = TypeId::of::<T>();
        let storage = self.components.entry(type_id).or_insert_with(HashMap::new);
        storage.insert(entity, Box::new(component));
    }

    /// Get an immutable reference to a component
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let type_id = TypeId::of::<T>();
        self.components
            .get(&type_id)?
            .get(&entity)?
            .downcast_ref::<T>()
    }

    /// Get a mutable reference to a component
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let type_id = TypeId::of::<T>();
        self.components
            .get_mut(&type_id)?
            .get_mut(&entity)?
            .downcast_mut::<T>()
    }

    /// Check if an entity has a component
    pub fn has_component<T: Component>(&self, entity: Entity) -> bool {
        let type_id = TypeId::of::<T>();
        self.components
            .get(&type_id)
            .map(|storage| storage.contains_key(&entity))
            .unwrap_or(false)
    }

    /// Remove a component from an entity
    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let type_id = TypeId::of::<T>();
        self.components
            .get_mut(&type_id)?
            .remove(&entity)?
            .downcast::<T>()
            .ok()
            .map(|boxed| *boxed)
    }

    /// Destroy an entity and all its components
    pub fn despawn(&mut self, entity: Entity) {
        self.entities.retain(|&e| e != entity);
        for storage in self.components.values_mut() {
            storage.remove(&entity);
        }
    }

    /// Get all entities that have a specific component
    pub fn query<T: Component>(&self) -> Vec<Entity> {
        let type_id = TypeId::of::<T>();
        self.components
            .get(&type_id)
            .map(|storage| storage.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Get all entities that have all specified component types
    pub fn query_with<T1: Component, T2: Component>(&self) -> Vec<Entity> {
        let entities_with_t1: Vec<Entity> = self.query::<T1>();
        entities_with_t1
            .into_iter()
            .filter(|&e| self.has_component::<T2>(e))
            .collect()
    }

    /// Get all entities that have three specific component types
    pub fn query_with3<T1: Component, T2: Component, T3: Component>(&self) -> Vec<Entity> {
        let entities: Vec<Entity> = self.query::<T1>();
        entities
            .into_iter()
            .filter(|&e| self.has_component::<T2>(e) && self.has_component::<T3>(e))
            .collect()
    }

    /// Get all entities
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Clear all entities and components
    pub fn clear(&mut self) {
        self.entities.clear();
        self.components.clear();
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
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

    #[derive(Debug, Clone, PartialEq)]
    struct Velocity {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Health {
        current: i32,
        max: i32,
    }

    #[test]
    fn test_spawn_entity() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();

        assert_ne!(e1, e2);
        assert_eq!(e1.id(), 0);
        assert_eq!(e2.id(), 1);
    }

    #[test]
    fn test_add_and_get_component() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position { x: 10.0, y: 20.0 });

        let pos = world.get_component::<Position>(entity).unwrap();
        assert_eq!(pos.x, 10.0);
        assert_eq!(pos.y, 20.0);
    }

    #[test]
    fn test_get_component_mut() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position { x: 10.0, y: 20.0 });

        {
            let pos = world.get_component_mut::<Position>(entity).unwrap();
            pos.x = 30.0;
        }

        let pos = world.get_component::<Position>(entity).unwrap();
        assert_eq!(pos.x, 30.0);
    }

    #[test]
    fn test_has_component() {
        let mut world = World::new();
        let entity = world.spawn();

        assert!(!world.has_component::<Position>(entity));

        world.add_component(entity, Position { x: 0.0, y: 0.0 });

        assert!(world.has_component::<Position>(entity));
        assert!(!world.has_component::<Velocity>(entity));
    }

    #[test]
    fn test_remove_component() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position { x: 10.0, y: 20.0 });
        assert!(world.has_component::<Position>(entity));

        let removed = world.remove_component::<Position>(entity).unwrap();
        assert_eq!(removed.x, 10.0);
        assert!(!world.has_component::<Position>(entity));
    }

    #[test]
    fn test_despawn_entity() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, Position { x: 10.0, y: 20.0 });
        world.add_component(entity, Velocity { x: 1.0, y: 2.0 });

        world.despawn(entity);

        assert!(!world.has_component::<Position>(entity));
        assert!(!world.has_component::<Velocity>(entity));
        assert_eq!(world.entities().len(), 0);
    }

    #[test]
    fn test_query_single_component() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });

        let e2 = world.spawn();
        world.add_component(e2, Position { x: 3.0, y: 4.0 });

        let e3 = world.spawn();
        world.add_component(e3, Velocity { x: 5.0, y: 6.0 });

        let entities = world.query::<Position>();
        assert_eq!(entities.len(), 2);
        assert!(entities.contains(&e1));
        assert!(entities.contains(&e2));
        assert!(!entities.contains(&e3));
    }

    #[test]
    fn test_query_with_two_components() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });
        world.add_component(e1, Velocity { x: 1.0, y: 1.0 });

        let e2 = world.spawn();
        world.add_component(e2, Position { x: 3.0, y: 4.0 });

        let e3 = world.spawn();
        world.add_component(e3, Velocity { x: 5.0, y: 6.0 });

        let entities = world.query_with::<Position, Velocity>();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0], e1);
    }

    #[test]
    fn test_query_with_three_components() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });
        world.add_component(e1, Velocity { x: 1.0, y: 1.0 });
        world.add_component(
            e1,
            Health {
                current: 100,
                max: 100,
            },
        );

        let e2 = world.spawn();
        world.add_component(e2, Position { x: 3.0, y: 4.0 });
        world.add_component(e2, Velocity { x: 2.0, y: 2.0 });

        let entities = world.query_with3::<Position, Velocity, Health>();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0], e1);
    }

    #[test]
    fn test_multiple_entities_different_components() {
        let mut world = World::new();

        // Create 100 entities with various components
        for i in 0..100 {
            let entity = world.spawn();
            world.add_component(
                entity,
                Position {
                    x: i as f32,
                    y: i as f32,
                },
            );

            if i % 2 == 0 {
                world.add_component(entity, Velocity { x: 1.0, y: 1.0 });
            }

            if i % 3 == 0 {
                world.add_component(
                    entity,
                    Health {
                        current: 100,
                        max: 100,
                    },
                );
            }
        }

        assert_eq!(world.query::<Position>().len(), 100);
        assert_eq!(world.query::<Velocity>().len(), 50);
        assert_eq!(world.query::<Health>().len(), 34); // 0, 3, 6, ..., 99
    }

    #[test]
    fn test_clear_world() {
        let mut world = World::new();

        for i in 0..10 {
            let entity = world.spawn();
            world.add_component(
                entity,
                Position {
                    x: i as f32,
                    y: 0.0,
                },
            );
        }

        assert_eq!(world.entities().len(), 10);

        world.clear();

        assert_eq!(world.entities().len(), 0);
        assert_eq!(world.query::<Position>().len(), 0);
    }
}
