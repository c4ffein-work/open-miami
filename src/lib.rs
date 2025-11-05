// Core modules
pub mod math;

// WASM-only modules for browser integration
#[cfg(target_arch = "wasm32")]
pub mod graphics;
#[cfg(target_arch = "wasm32")]
pub mod input;

// Library module for game logic (enables testing)
pub mod components;
pub mod ecs;
pub mod game;
#[cfg(target_arch = "wasm32")]
pub mod render;
pub mod systems;

// Keep old modules for camera and level (WASM-only)
#[cfg(target_arch = "wasm32")]
pub mod camera;
#[cfg(target_arch = "wasm32")]
pub mod level;

// Old modules (deprecated but kept for reference)
// These are not exported to avoid dead code warnings
// Only compile for WASM as they use macroquad
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod collision;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod enemy;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod player;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod weapon;
