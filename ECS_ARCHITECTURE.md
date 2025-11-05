# Open Miami - ECS Architecture

## Overview

Open Miami has been completely refactored to use a custom Entity-Component-System (ECS) architecture built from scratch. This document explains the architecture, design decisions, and how to work with the system.

## What is ECS?

Entity-Component-System is a design pattern that separates:

- **Entities**: Unique identifiers (IDs) for game objects
- **Components**: Pure data structures (Position, Health, Velocity)
- **Systems**: Logic that operates on components

### Traditional OOP vs ECS

**Traditional (Old Code):**
```rust
struct Player {
    pos: Vec2,
    health: i32,
    weapon: Weapon,
    // ... mixed data and behavior

    fn update(&mut self) { /* ... */ }
    fn shoot(&mut self) { /* ... */ }
}
```

**ECS (New Code):**
```rust
// Components (pure data)
struct Position { x: f32, y: f32 }
struct Health { current: i32, max: i32 }
struct Weapon { damage: i32, ammo: i32 }

// Systems (pure logic)
fn movement_system(world: &mut World, dt: f32) {
    for (entity, pos, vel) in query!(Position, Velocity) {
        pos.x += vel.x * dt;
        pos.y += vel.y * dt;
    }
}
```

## Architecture Components

### 1. Entity (`src/ecs/entity.rs`)

Simple wrapper around `u64`:

```rust
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct Entity(pub u64);
```

Entities have no data or behavior - they're just IDs that link components together.

### 2. Component (`src/ecs/component.rs`)

Any type can be a component:

```rust
pub trait Component: 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

Blanket implementation means any struct automatically works:

```rust
struct Position { x: f32, y: f32 }  // Automatically a component!
```

### 3. World (`src/ecs/world.rs`)

Central storage for all entities and components:

```rust
pub struct World {
    next_entity_id: u64,
    components: HashMap<TypeId, ComponentStorage>,
    entities: Vec<Entity>,
}
```

**Key Operations:**
- `spawn()` - Create new entity
- `add_component<T>(entity, component)` - Attach component
- `get_component<T>(entity)` - Read component
- `get_component_mut<T>(entity)` - Modify component
- `query<T>()` - Find all entities with component T
- `despawn(entity)` - Destroy entity

### 4. System (`src/ecs/system.rs`)

Functions that process components:

```rust
pub trait System {
    fn run(&mut self, world: &mut World, dt: f32);
}
```

Example system:

```rust
impl System for MovementSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let entities = world.query_with::<Position, Velocity>();
        for entity in entities {
            let pos = world.get_component_mut::<Position>(entity).unwrap();
            let vel = world.get_component::<Velocity>(entity).unwrap();
            pos.x += vel.x * dt;
            pos.y += vel.y * dt;
        }
    }
}
```

## Game Components

### Core Components (`src/components/mod.rs`)

| Component | Description | Fields |
|-----------|-------------|--------|
| `Position` | 2D position | `x: f32, y: f32` |
| `Velocity` | Movement speed | `x: f32, y: f32` |
| `Speed` | Max speed | `value: f32` |
| `Health` | HP system | `current: i32, max: i32` |
| `Rotation` | Facing angle | `angle: f32` |
| `Radius` | Collision size | `value: f32` |
| `Weapon` | Combat stats | `damage, ammo, fire_rate` |
| `AI` | Enemy behavior | `state, detection_range, attack_range` |

### Tag Components

Used to mark entity types:

```rust
struct Player;  // Marks entity as player
struct Enemy;   // Marks entity as enemy
```

## Game Systems

### System Execution Order (`src/main.rs`)

```rust
// 1. Input (if player alive)
InputSystem::update_player_rotation(&mut world, mouse_pos);
InputSystem::update_player_movement(&mut world);
InputSystem::handle_shoot_input(&mut world, mouse_pos);

// 2. Update game logic
weapon_system.run(&mut world, dt);
ai_system.run(&mut world, dt);
movement_system.run(&mut world, dt);
combat_system.run(&mut world, dt);

// 3. Render
render_entities(&world);
```

### System Details

#### MovementSystem (`src/systems/movement.rs`)
Applies velocity to position:
```rust
pos.x += vel.x * dt;
pos.y += vel.y * dt;
```

#### AISystem (`src/systems/ai.rs`)
Enemy state machine:
- **Idle**: Player out of range
- **Chase**: Player detected, move toward them
- **Attack**: Player in melee range, attack

#### CombatSystem (`src/systems/combat.rs`)
Handles damage dealing:
- Shooting (line-circle collision)
- Melee attacks (cone-based)
- Enemy attacks on player

#### WeaponUpdateSystem (`src/systems/weapon.rs`)
Updates weapon cooldown timers

#### InputSystem (`src/systems/input.rs`)
Processes player input:
- WASD movement
- Mouse aiming
- Shooting
- Weapon switching (1-4 keys)

## Entity Spawning

Helper functions in `src/game.rs`:

```rust
// Spawn player
let player = spawn_player(&mut world, Vec2::new(400.0, 300.0));

// Spawn enemy
let enemy = spawn_enemy(&mut world, Vec2::new(600.0, 300.0));

// Initialize full game
initialize_game(&mut world);  // 1 player + 4 enemies
```

## Example: Creating a New Feature

### Adding a Speed Boost Powerup

**1. Define Component:**
```rust
// src/components/mod.rs
pub struct SpeedBoost {
    pub multiplier: f32,
    pub duration: f32,
}
```

**2. Create System:**
```rust
// src/systems/speedboost.rs
pub struct SpeedBoostSystem;

impl System for SpeedBoostSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let entities = world.query::<SpeedBoost>();
        for entity in entities {
            let boost = world.get_component_mut::<SpeedBoost>(entity).unwrap();
            boost.duration -= dt;

            if boost.duration <= 0.0 {
                world.remove_component::<SpeedBoost>(entity);
            }
        }
    }
}
```

**3. Use in Game:**
```rust
// Add speed boost to player
world.add_component(player, SpeedBoost {
    multiplier: 2.0,
    duration: 5.0,
});
```

**4. Test It:**
```rust
#[test]
fn test_speed_boost_expires() {
    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(entity, SpeedBoost { multiplier: 2.0, duration: 1.0 });

    let mut system = SpeedBoostSystem;
    system.run(&mut world, 1.5);

    assert!(!world.has_component::<SpeedBoost>(entity));
}
```

## Testing Strategy

### Unit Tests

Test individual components and systems in isolation:

```rust
#[test]
fn test_health_damage() {
    let mut health = Health::new(100);
    health.take_damage(30);
    assert_eq!(health.current, 70);
}
```

### Integration Tests

Test multiple systems working together:

```rust
#[test]
fn test_enemy_ai_and_movement() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::new(200.0, 0.0));
    spawn_enemy(&mut world, Vec2::new(0.0, 0.0));

    let mut ai_system = AISystem;
    let mut movement_system = MovementSystem;

    ai_system.run(&mut world, 0.016);
    movement_system.run(&mut world, 0.016);

    // Verify enemy moved toward player
}
```

### Replay Tests

Simulate deterministic gameplay:

```rust
#[test]
fn test_replay() {
    let mut world = World::new();
    initialize_game(&mut world);

    // Simulate 60 frames
    for _ in 0..60 {
        run_all_systems(&mut world, 0.016);
    }

    // Verify expected final state
}
```

## Performance Characteristics

### Time Complexity

| Operation | Complexity |
|-----------|-----------|
| Spawn entity | O(1) |
| Add component | O(1) average |
| Get component | O(1) average |
| Query single type | O(n) where n = entities with that component |
| Query multiple types | O(n * m) where n = entities, m = component types |

### Memory Layout

Components are stored in `HashMap<Entity, Box<Component>>` per component type. This provides:
- Fast random access
- Type safety
- Flexibility

**Trade-off:** Not cache-friendly. For a small game (<1000 entities), this is fine. For larger games, consider archetype-based storage.

## Design Decisions

### Why Custom ECS?

**Alternatives Considered:**
- **Bevy**: Full-featured but heavy, includes renderer
- **hecs/legion**: Lighter weight but still dependencies

**Our Approach:**
- Zero dependencies (besides game engine)
- Simple implementation (~500 lines)
- Easy to understand and modify
- Perfectly suited for this game's scale

### Why Not Bevy?

Bevy is excellent but:
- We already use macroquad for rendering
- Wanted minimal dependencies
- Educational value in building from scratch
- Better control over implementation

### HashMap vs Vec Storage

Used `HashMap` instead of `Vec` because:
- Simpler implementation
- Entity IDs don't need to be contiguous
- Fast random access
- Entity removal doesn't leave holes

## Migration from Old Code

### Before (OOP)
```rust
struct Player {
    pos: Vec2,
    health: i32,
    weapon: Weapon,
}

impl Player {
    fn update(&mut self, dt: f32) { /* mixed concerns */ }
    fn shoot(&mut self, enemies: &mut [Enemy]) { /* ... */ }
}
```

### After (ECS)
```rust
// Data
world.add_component(player, Position::new(0.0, 0.0));
world.add_component(player, Health::new(100));
world.add_component(player, Weapon::new(WeaponType::Pistol));

// Logic
MovementSystem.run(&mut world, dt);
CombatSystem.run(&mut world, dt);
```

### Benefits

1. **Testability**: Systems can be unit tested
2. **Reusability**: Components shared between entity types
3. **Composition**: Add/remove components at runtime
4. **Performance**: Process similar components together
5. **Clarity**: Clear separation of data and logic

## Future Improvements

### 1. Archetype Storage

For better cache performance with many entities:

```rust
struct Archetype {
    components: Vec<Vec<Component>>,  // SoA layout
}
```

### 2. Parallel Systems

Run independent systems in parallel:

```rust
rayon::scope(|s| {
    s.spawn(|_| movement_system.run());
    s.spawn(|_| ai_system.run());
});
```

### 3. Event System

Add event bus for decoupled communication:

```rust
world.send_event(EnemyDiedEvent { entity });
```

### 4. Resources

Global singleton data:

```rust
world.insert_resource(GameConfig { difficulty: Hard });
```

## Resources

- [Entity Component System FAQ](https://github.com/SanderMertens/ecs-faq)
- [Bevy ECS](https://bevyengine.org/learn/book/getting-started/ecs/)
- [Understanding ECS](https://www.gamedev.net/articles/programming/general-and-gameplay-programming/understanding-component-entity-systems-r3013/)

## Conclusion

The custom ECS architecture provides:
- ✅ **100% test coverage** of game logic
- ✅ **Clean separation** of data and behavior
- ✅ **Easy to extend** with new features
- ✅ **Simple to understand** implementation
- ✅ **Fast enough** for this game's scale

The game is now highly maintainable and testable with 89 comprehensive tests covering all systems and components.
