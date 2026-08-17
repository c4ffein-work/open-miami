// Game Systems - Pure logic operating on components
pub mod ai;
pub mod boss;
pub mod combat;
pub mod elevator;
#[cfg(target_arch = "wasm32")]
pub mod input;
pub mod movement;
pub mod passive;
pub mod pickup;
pub mod projectile;
pub mod stun;
pub mod thrown;
pub mod weapon;

pub use ai::AISystem;
pub use boss::BossSystem;
pub use combat::CombatSystem;
pub use elevator::ElevatorSystem;
#[cfg(target_arch = "wasm32")]
pub use input::InputSystem;
pub use movement::MovementSystem;
pub use pickup::PickupSystem;
pub use projectile::{BulletSystem, ProjectileTrailSystem};
pub use stun::StunSystem;
pub use thrown::ThrownWeaponSystem;
pub use weapon::WeaponUpdateSystem;
