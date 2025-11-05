use crate::components::{Player, Position, Rotation, Speed, Velocity, Weapon, WeaponType};
use crate::ecs::{Entity, World};
use crate::input;
use crate::math::Vec2;
use crate::systems::combat::CombatSystem;

/// System that handles player input
pub struct InputSystem;

impl InputSystem {
    fn find_player(world: &World) -> Option<Entity> {
        world.query::<Player>().first().copied()
    }

    /// Update player rotation to face mouse
    pub fn update_player_rotation(world: &mut World, mouse_world_pos: Vec2) {
        let player = match Self::find_player(world) {
            Some(e) => e,
            None => return,
        };

        let player_pos = match world.get_component::<Position>(player) {
            Some(pos) => pos.to_vec2(),
            None => return,
        };

        let dx = mouse_world_pos.x - player_pos.x;
        let dy = mouse_world_pos.y - player_pos.y;
        let angle = dy.atan2(dx);

        if let Some(rotation) = world.get_component_mut::<Rotation>(player) {
            rotation.angle = angle;
        }
    }

    /// Update player velocity based on WASD input
    pub fn update_player_movement(world: &mut World) {
        let player = match Self::find_player(world) {
            Some(e) => e,
            None => return,
        };

        let speed = match world.get_component::<Speed>(player) {
            Some(s) => s.value,
            None => return,
        };

        let mut move_x = 0.0;
        let mut move_y = 0.0;

        if input::is_key_down(input::keys::W) || input::is_key_down(input::keys::ARROW_UP) {
            move_y -= 1.0;
        }
        if input::is_key_down(input::keys::S) || input::is_key_down(input::keys::ARROW_DOWN) {
            move_y += 1.0;
        }
        if input::is_key_down(input::keys::A) || input::is_key_down(input::keys::ARROW_LEFT) {
            move_x -= 1.0;
        }
        if input::is_key_down(input::keys::D) || input::is_key_down(input::keys::ARROW_RIGHT) {
            move_x += 1.0;
        }

        // Normalize diagonal movement
        let len = (move_x * move_x + move_y * move_y).sqrt();
        if len > 0.0 {
            move_x /= len;
            move_y /= len;
        }

        if let Some(velocity) = world.get_component_mut::<Velocity>(player) {
            velocity.x = move_x * speed;
            velocity.y = move_y * speed;
        }
    }

    /// Handle shooting input
    pub fn handle_shoot_input(world: &mut World, mouse_world_pos: Vec2) -> bool {
        if !input::is_mouse_button_down(input::mouse_buttons::LEFT) {
            return false;
        }

        let player = match Self::find_player(world) {
            Some(e) => e,
            None => return false,
        };

        let player_pos = match world.get_component::<Position>(player) {
            Some(pos) => *pos,
            None => return false,
        };

        let weapon = match world.get_component_mut::<Weapon>(player) {
            Some(w) => w,
            None => return false,
        };

        // Check if weapon can fire
        if !weapon.can_fire() {
            return false;
        }

        let damage = weapon.damage;
        let is_melee = weapon.weapon_type == WeaponType::Melee;

        // Fire weapon
        weapon.fire();

        let target_pos = Position::from_vec2(mouse_world_pos);

        // Process attack
        if is_melee {
            CombatSystem::process_melee(world, player_pos, target_pos, damage, 50.0)
        } else {
            CombatSystem::process_shoot(world, player_pos, target_pos, damage)
        }
    }

    /// Handle weapon switching (1-4 keys)
    pub fn handle_weapon_switch(world: &mut World) {
        let player = match Self::find_player(world) {
            Some(e) => e,
            None => return,
        };

        let new_weapon_type = if input::is_key_down("1") {
            Some(WeaponType::Pistol)
        } else if input::is_key_down("2") {
            Some(WeaponType::Shotgun)
        } else if input::is_key_down("3") {
            Some(WeaponType::MachineGun)
        } else if input::is_key_down("4") {
            Some(WeaponType::Melee)
        } else {
            None
        };

        if let Some(weapon_type) = new_weapon_type {
            if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
                *weapon = Weapon::new(weapon_type);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_player() {
        let mut world = World::new();
        let player = world.spawn();
        world.add_component(player, Player);

        let found = InputSystem::find_player(&world);
        assert_eq!(found, Some(player));
    }

    #[test]
    fn test_find_player_none() {
        let world = World::new();
        let found = InputSystem::find_player(&world);
        assert_eq!(found, None);
    }
}
