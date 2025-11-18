use crate::components::{Player, Position, Radius, Weapon, WeaponPickup};
use crate::ecs::{Entity, System, World};

/// System that handles weapon pickup and swapping
pub struct PickupSystem;

impl PickupSystem {
    /// Check if player collides with any weapon pickups
    fn process_pickups(world: &mut World) {
        // Find player
        let player_entity = match world.query::<Player>().first() {
            Some(&e) => e,
            None => return,
        };

        let (player_pos, player_radius) = match (
            world.get_component::<Position>(player_entity),
            world.get_component::<Radius>(player_entity),
        ) {
            (Some(pos), Some(rad)) => (*pos, *rad),
            _ => return,
        };

        // Find all weapon pickups
        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        let mut pickup_to_collect = None;
        let pickup_radius = 20.0; // Collision radius for pickups

        for pickup_entity in pickups {
            let (pickup_pos, weapon_pickup) = match (
                world.get_component::<Position>(pickup_entity),
                world.get_component::<WeaponPickup>(pickup_entity),
            ) {
                (Some(pos), Some(pickup)) => (*pos, *pickup),
                _ => continue,
            };

            // Check collision
            let distance = player_pos.distance_to(&pickup_pos);
            if distance < player_radius.value + pickup_radius {
                pickup_to_collect = Some((pickup_entity, weapon_pickup.weapon_type));
                break;
            }
        }

        // If player touched a pickup, collect it
        if let Some((pickup_entity, weapon_type)) = pickup_to_collect {
            // Create the new weapon for the player
            let new_weapon = Weapon::new(weapon_type);

            // Get old weapon if player has one
            let old_weapon = world.get_component::<Weapon>(player_entity).copied();

            // Give player the new weapon
            world.add_component(player_entity, new_weapon);

            // If player had an old weapon, drop it at pickup location
            if let Some(old) = old_weapon {
                if let Some(pickup_pos) = world.get_component::<Position>(pickup_entity) {
                    let drop_pos = *pickup_pos;

                    // Reuse the pickup entity for the dropped weapon
                    world.add_component(pickup_entity, WeaponPickup::new(old.weapon_type));
                    world.add_component(pickup_entity, drop_pos);

                    // The pickup entity now represents the dropped weapon, so we're done
                    return;
                }
            }

            // If no old weapon, just remove the pickup
            world.despawn(pickup_entity);
        }
    }

    /// Drop the player's current weapon at their position
    pub fn drop_weapon(world: &mut World) -> bool {
        // Find player
        let player_entity = match world.query::<Player>().first() {
            Some(&e) => e,
            None => return false,
        };

        // Get player's weapon and position
        let (weapon, player_pos) = match (
            world.get_component::<Weapon>(player_entity),
            world.get_component::<Position>(player_entity),
        ) {
            (Some(w), Some(pos)) => (*w, *pos),
            _ => return false,
        };

        // Create weapon pickup at player's position
        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(weapon.weapon_type));
        world.add_component(pickup, player_pos);
        world.add_component(pickup, Radius::new(20.0));

        // Remove weapon from player
        // Note: In a real ECS, we'd have a remove_component method
        // For now, we'll give player a "no weapon" state by giving them a pistol with 0 ammo
        let mut empty_weapon = Weapon::new(weapon.weapon_type);
        empty_weapon.ammo = 0;
        world.add_component(player_entity, empty_weapon);

        true
    }
}

impl System for PickupSystem {
    fn run(&mut self, world: &mut World, _dt: f32) {
        Self::process_pickups(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::WeaponType;

    #[test]
    fn test_pickup_weapon_basic() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 100.0));
        world.add_component(player, Radius::new(15.0));
        world.add_component(player, Weapon::new(WeaponType::Pistol));

        // Create weapon pickup near player
        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(WeaponType::Shotgun));
        world.add_component(pickup, Position::new(105.0, 100.0)); // Close to player
        world.add_component(pickup, Radius::new(20.0));

        // Run pickup system
        let mut system = PickupSystem;
        system.run(&mut world, 0.016);

        // Player should now have shotgun
        let weapon = world.get_component::<Weapon>(player).unwrap();
        assert_eq!(weapon.weapon_type, WeaponType::Shotgun);

        // Old pistol should now be at the pickup location as a pickup
        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);
        let pickup_weapon = world.get_component::<WeaponPickup>(pickups[0]).unwrap();
        assert_eq!(pickup_weapon.weapon_type, WeaponType::Pistol);
    }

    #[test]
    fn test_pickup_weapon_too_far() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 100.0));
        world.add_component(player, Radius::new(15.0));
        world.add_component(player, Weapon::new(WeaponType::Pistol));

        // Create weapon pickup far from player
        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(WeaponType::Shotgun));
        world.add_component(pickup, Position::new(200.0, 100.0)); // Too far
        world.add_component(pickup, Radius::new(20.0));

        // Run pickup system
        let mut system = PickupSystem;
        system.run(&mut world, 0.016);

        // Player should still have pistol
        let weapon = world.get_component::<Weapon>(player).unwrap();
        assert_eq!(weapon.weapon_type, WeaponType::Pistol);

        // Shotgun pickup should still exist
        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);
        let pickup_weapon = world.get_component::<WeaponPickup>(pickups[0]).unwrap();
        assert_eq!(pickup_weapon.weapon_type, WeaponType::Shotgun);
    }

    #[test]
    fn test_drop_weapon() {
        let mut world = World::new();

        // Create player with weapon
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 100.0));
        world.add_component(player, Weapon::new(WeaponType::MachineGun));

        // Drop weapon
        let dropped = PickupSystem::drop_weapon(&mut world);
        assert!(dropped);

        // Should create a pickup
        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);

        let pickup_weapon = world.get_component::<WeaponPickup>(pickups[0]).unwrap();
        assert_eq!(pickup_weapon.weapon_type, WeaponType::MachineGun);

        let pickup_pos = world.get_component::<Position>(pickups[0]).unwrap();
        assert_eq!(pickup_pos.x, 100.0);
        assert_eq!(pickup_pos.y, 100.0);
    }

    #[test]
    fn test_drop_weapon_no_player() {
        let mut world = World::new();

        // No player exists
        let dropped = PickupSystem::drop_weapon(&mut world);
        assert!(!dropped);

        // No pickups should be created
        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 0);
    }

    #[test]
    fn test_pickup_multiple_weapons_nearby() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(100.0, 100.0));
        world.add_component(player, Radius::new(15.0));
        world.add_component(player, Weapon::new(WeaponType::Pistol));

        // Create two weapon pickups near player
        let pickup1 = world.spawn();
        world.add_component(pickup1, WeaponPickup::new(WeaponType::Shotgun));
        world.add_component(pickup1, Position::new(105.0, 100.0));

        let pickup2 = world.spawn();
        world.add_component(pickup2, WeaponPickup::new(WeaponType::MachineGun));
        world.add_component(pickup2, Position::new(95.0, 100.0));

        // Run pickup system
        let mut system = PickupSystem;
        system.run(&mut world, 0.016);

        // Player should pick up one weapon
        let weapon = world.get_component::<Weapon>(player).unwrap();
        assert!(
            weapon.weapon_type == WeaponType::Shotgun
                || weapon.weapon_type == WeaponType::MachineGun
        );

        // Should still have 2 pickups total (one swapped + one untouched)
        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 2);
    }
}
