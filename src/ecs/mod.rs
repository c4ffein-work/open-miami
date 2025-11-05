// Custom ECS Engine
// Simple, testable architecture with minimal dependencies

pub mod component;
pub mod entity;
pub mod query;
pub mod system;
pub mod world;

pub use component::Component;
pub use entity::Entity;
pub use query::{Query, QueryMut};
pub use system::System;
pub use world::World;

// Re-export common types
pub use macroquad::prelude::Vec2;
