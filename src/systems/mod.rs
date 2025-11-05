// Game Systems - Pure logic operating on components
pub mod ai;
pub mod combat;
#[cfg(target_arch = "wasm32")]
pub mod input;
pub mod movement;
pub mod weapon;

pub use ai::AISystem;
pub use combat::CombatSystem;
#[cfg(target_arch = "wasm32")]
pub use input::InputSystem;
pub use movement::MovementSystem;
pub use weapon::WeaponUpdateSystem;
