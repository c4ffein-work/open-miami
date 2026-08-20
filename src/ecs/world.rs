use super::component::AnyComponent;
use super::{Component, Entity};
use crate::components::GameEvent;
use std::any::TypeId;
use std::collections::HashMap;

/// ComponentStorage stores all components of a specific type. The boxes are
/// clonable ([`AnyComponent`]) so the whole world can be snapshotted (the
/// `checkpoint` scenario action).
type ComponentStorage = HashMap<Entity, Box<dyn AnyComponent>>;

/// Wall obstacle represented as a rectangle
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wall {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Wall {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Wall {
            x,
            y,
            width,
            height,
        }
    }
}

/// World manages all entities and their components.
///
/// `Clone` produces a full deep snapshot (entities, components, walls, RNG
/// state, pending events) — the `checkpoint` scenario action stores one and
/// death restores it.
#[derive(Clone)]
pub struct World {
    next_entity_id: u64,
    // Map from ComponentId to storage for that component type
    components: HashMap<TypeId, ComponentStorage>,
    // Track which entities exist
    entities: Vec<Entity>,
    // Static walls in the world
    walls: Vec<Wall>,
    // Per-world RNG state (LCG). Seeded deterministically so each World is
    // independently reproducible; formerly this lived in a process-global
    // `static mut` in the AI system.
    rng_state: u32,
    // One-shot gameplay events queued this frame (see `push_event`).
    events: Vec<GameEvent>,
    // Whether debug visualisation is being shown this frame (the `?debug` + I
    // overlays). Set once per frame by the browser layer; `false` by default
    // (headless sims, tests). Systems skip recording debug-only structures
    // (DebugPath / DebugTrail) while this is off.
    debug_viz: bool,
}

/// Upper bound on queued events. A consumer that never drains (e.g. the
/// headless simulation) must not make the queue grow without limit; past this
/// many pending events new ones are dropped.
pub const MAX_PENDING_EVENTS: usize = 256;

impl World {
    pub fn new() -> Self {
        World {
            next_entity_id: 0,
            components: HashMap::new(),
            entities: Vec::new(),
            walls: Vec::new(),
            rng_state: 12345,
            events: Vec::new(),
            debug_viz: false,
        }
    }

    /// Whether debug visualisation is on this frame (see the field docs).
    pub fn debug_viz(&self) -> bool {
        self.debug_viz
    }

    /// Set by the browser layer once per frame when the debug overlays are
    /// visible; everything headless leaves it `false`.
    pub fn set_debug_viz(&mut self, on: bool) {
        self.debug_viz = on;
    }

    /// Queue a one-shot gameplay event for whoever drains the queue this frame
    /// (the browser layer turns them into sound effects). Silently dropped once
    /// [`MAX_PENDING_EVENTS`] are pending, so an undrained world stays bounded.
    pub fn push_event(&mut self, event: GameEvent) {
        if self.events.len() < MAX_PENDING_EVENTS {
            self.events.push(event);
        }
    }

    /// Take every pending event, in emission order, leaving the queue empty.
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }

    /// Peek at the pending events without draining them.
    pub fn pending_events(&self) -> &[GameEvent] {
        &self.events
    }

    /// Advance the world's LCG and return the next pseudo-random `u32`.
    ///
    /// Uses the classic Numerical Recipes constants; the seed (`12345`) and the
    /// constants match the previous process-global generator, so a freshly
    /// created world produces the exact same random sequence as before.
    pub fn next_random(&mut self) -> u32 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        self.rng_state
    }

    /// Random float in `[min, max)` (or `[min, max]` at the endpoint).
    pub fn random_range(&mut self, min: f32, max: f32) -> f32 {
        let r = self.next_random() as f32 / u32::MAX as f32;
        r * (max - min) + min
    }

    /// Random integer in `[min, max]` (inclusive).
    pub fn random_int_range(&mut self, min: i32, max: i32) -> i32 {
        let range = (max - min + 1) as u32;
        min + (self.next_random() % range) as i32
    }

    /// Read the current raw RNG state. Systems that need to draw several random
    /// numbers while holding other borrows of the world can copy this out, use
    /// it locally, then write it back via [`World::set_rng_state`].
    pub fn rng_state(&self) -> u32 {
        self.rng_state
    }

    /// Overwrite the raw RNG state (see [`World::rng_state`]).
    pub fn set_rng_state(&mut self, state: u32) {
        self.rng_state = state;
    }

    /// Create a new entity
    pub fn spawn(&mut self) -> Entity {
        let entity = Entity::new(self.next_entity_id);
        self.next_entity_id += 1;
        self.entities.push(entity);
        entity
    }

    /// Add a component to an entity. Components must be `Clone` so the world
    /// can be snapshotted (see the struct docs).
    pub fn add_component<T: Component + Clone>(&mut self, entity: Entity, component: T) {
        let type_id = TypeId::of::<T>();
        let storage = self.components.entry(type_id).or_default();
        storage.insert(entity, Box::new(component));
    }

    /// Get an immutable reference to a component
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let type_id = TypeId::of::<T>();
        AnyComponent::as_any(self.components.get(&type_id)?.get(&entity)?.as_ref())
            .downcast_ref::<T>()
    }

    /// Get a mutable reference to a component
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let type_id = TypeId::of::<T>();
        AnyComponent::as_any_mut(
            self.components
                .get_mut(&type_id)?
                .get_mut(&entity)?
                .as_mut(),
        )
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
            .into_any()
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

    /// Get all entities that have a specific component.
    ///
    /// Results are sorted by entity id (i.e. spawn order). Component storage is a
    /// `HashMap`, whose iteration order is randomized per-instance, so returning
    /// the raw key order would make system behavior (RNG-consumption order,
    /// nearest-target tie-breaks, ...) differ between otherwise identical worlds.
    /// Sorting makes the whole simulation deterministic.
    pub fn query<T: Component>(&self) -> Vec<Entity> {
        let type_id = TypeId::of::<T>();
        let mut entities: Vec<Entity> = self
            .components
            .get(&type_id)
            .map(|storage| storage.keys().copied().collect())
            .unwrap_or_default();
        entities.sort_unstable_by_key(|e| e.id());
        entities
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

    /// Add a wall obstacle to the world
    pub fn add_wall(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.walls.push(Wall::new(x, y, width, height));
    }

    /// Get all walls in the world
    pub fn walls(&self) -> &[Wall] {
        &self.walls
    }

    /// Clear all entities and components
    pub fn clear(&mut self) {
        self.entities.clear();
        self.components.clear();
        self.walls.clear();
        self.events.clear();
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
    fn test_events_push_drain_and_cap() {
        let mut world = World::new();
        assert!(world.pending_events().is_empty());
        world.push_event(GameEvent::EnemyDown);
        world.push_event(GameEvent::Pickup);
        assert_eq!(
            world.pending_events(),
            &[GameEvent::EnemyDown, GameEvent::Pickup]
        );
        let drained = world.drain_events();
        assert_eq!(drained, vec![GameEvent::EnemyDown, GameEvent::Pickup]);
        assert!(world.drain_events().is_empty(), "drained once");

        // Never grows past the cap when nobody drains.
        for _ in 0..(MAX_PENDING_EVENTS * 2) {
            world.push_event(GameEvent::DryFire);
        }
        assert_eq!(world.pending_events().len(), MAX_PENDING_EVENTS);
        world.clear();
        assert!(world.pending_events().is_empty());
    }

    #[test]
    fn test_clone_is_a_deep_snapshot() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position { x: 100.0, y: 200.0 });
        world.add_wall(0.0, 0.0, 50.0, 50.0);
        world.push_event(GameEvent::Pickup);
        world.next_random();

        let snapshot = world.clone();

        // Mutate the original: the snapshot must not follow.
        world.get_component_mut::<Position>(e).unwrap().x = 999.0;
        let e2 = world.spawn();
        world.add_component(e2, Velocity { x: 1.0, y: 1.0 });
        world.despawn(e);

        assert_eq!(snapshot.entities().len(), 1);
        let pos = snapshot.get_component::<Position>(e).unwrap();
        assert_eq!(pos.x, 100.0);
        assert!(snapshot.query::<Velocity>().is_empty());
        assert_eq!(snapshot.walls().len(), 1);
        assert_eq!(snapshot.pending_events(), &[GameEvent::Pickup]);
        // RNG state travels with the snapshot: both sides produce the same
        // next number.
        let mut a = snapshot.clone();
        let mut b = snapshot.clone();
        assert_eq!(a.next_random(), b.next_random());
        // Entity ids keep counting from where the snapshot was taken.
        let mut c = snapshot.clone();
        assert_eq!(c.spawn().id(), e2.id());
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
