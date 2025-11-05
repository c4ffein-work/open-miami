// Custom ECS Engine
// Simple, testable architecture with minimal dependencies

pub mod entity;
pub mod component;
pub mod world;
pub mod query;
pub mod system;

pub use entity::Entity;
pub use component::Component;
pub use world::World;
pub use query::{Query, QueryMut};
pub use system::System;

// Re-export common types
pub use macroquad::prelude::Vec2;
