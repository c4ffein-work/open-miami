use std::any::{Any, TypeId};

/// Marker trait for components
/// Any type that is 'static can be a component
pub trait Component: 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// Blanket implementation for all 'static types
impl<T: 'static> Component for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Type-safe wrapper for component type IDs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentId(pub TypeId);

impl ComponentId {
    pub fn of<T: Component>() -> Self {
        ComponentId(TypeId::of::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Debug, PartialEq)]
    struct Velocity {
        x: f32,
        y: f32,
    }

    #[test]
    fn test_component_id_uniqueness() {
        let pos_id = ComponentId::of::<Position>();
        let vel_id = ComponentId::of::<Velocity>();

        assert_ne!(pos_id, vel_id);
    }

    #[test]
    fn test_component_id_consistency() {
        let id1 = ComponentId::of::<Position>();
        let id2 = ComponentId::of::<Position>();

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_component_as_any() {
        let mut pos = Position { x: 10.0, y: 20.0 };

        let any_ref = pos.as_any();
        let downcast = any_ref.downcast_ref::<Position>().unwrap();
        assert_eq!(downcast.x, 10.0);

        let any_mut = pos.as_any_mut();
        let downcast_mut = any_mut.downcast_mut::<Position>().unwrap();
        downcast_mut.x = 30.0;
        assert_eq!(pos.x, 30.0);
    }
}
