// Library module for game logic (enables testing)
pub mod components;
pub mod ecs;
pub mod game;
pub mod render;
pub mod systems;

// Keep old modules for camera and level
pub mod camera;
pub mod level;

// Old modules (deprecated but kept for reference)
// These are not exported to avoid dead code warnings
#[allow(dead_code)]
mod collision;
#[allow(dead_code)]
mod enemy;
#[allow(dead_code)]
mod player;
#[allow(dead_code)]
mod weapon;
