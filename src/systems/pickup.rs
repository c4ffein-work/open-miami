use crate::components::{
    Enemy, Health, Player, Position, Radius, Weapon, WeaponPickup, WeaponType,
};
use crate::ecs::{Entity, System, World};

/// System that handles weapons dropped by downed enemies and their collection
/// by the player. Downed enemies drop their weapon on the floor; the player
/// swaps for it (refilling ammo) by standing over it and pressing the pick-up
/// key. See [`PickupSystem::swap_for_player`].
pub struct PickupSystem;

impl PickupSystem {
    /// Spawn a weapon pickup for any dead enemy that still carries a weapon.
    /// The weapon is removed from the enemy so it is only dropped once.
    pub fn drop_from_dead_enemies(world: &mut World) {
        let enemies: Vec<Entity> = world.query::<Enemy>();

        for enemy in enemies {
            let is_dead = match world.get_component::<Health>(enemy) {
                Some(h) => h.is_dead(),
                None => continue,
            };
            if !is_dead {
                continue;
            }

            let weapon = match world.get_component::<Weapon>(enemy) {
                Some(w) => *w,
                None => continue, // already dropped (or never carried one)
            };
            let pos = match world.get_component::<Position>(enemy) {
                Some(p) => *p,
                None => continue,
            };

            // Remove the weapon so this enemy does not drop again next frame.
            world.remove_component::<Weapon>(enemy);

            let pickup = world.spawn();
            world.add_component(pickup, WeaponPickup::new(weapon.weapon_type));
            world.add_component(pickup, pos);
            world.add_component(pickup, Radius::new(14.0));
        }
    }

    /// Swap the player's weapon with a pickup they are standing on (Hotline
    /// Miami style): the player takes the pickup's weapon (fully loaded) and
    /// their previous weapon is dropped in its place, so nothing is ever lost.
    ///
    /// This is deliberately not run every frame — the caller gates it behind a
    /// key press so the player chooses when to swap. Returns the newly held
    /// weapon type if a swap happened.
    pub fn swap_for_player(world: &mut World) -> Option<WeaponType> {
        let player = world.query::<Player>().into_iter().next()?;
        let player_pos = *world.get_component::<Position>(player)?;
        let player_radius = world
            .get_component::<Radius>(player)
            .map(|r| r.value)
            .unwrap_or(15.0);
        let current_weapon = world.get_component::<Weapon>(player).map(|w| w.weapon_type);

        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        for pickup in pickups {
            let (pickup_pos, pickup_radius, new_weapon) = match (
                world.get_component::<Position>(pickup),
                world.get_component::<Radius>(pickup),
                world.get_component::<WeaponPickup>(pickup),
            ) {
                (Some(p), Some(r), Some(w)) => (*p, r.value, w.weapon_type),
                _ => continue,
            };

            if player_pos.distance_to(&pickup_pos) > player_radius + pickup_radius {
                continue;
            }

            // Give the player the new weapon, fully loaded (an unarmed player —
            // e.g. right after a throw — has no Weapon component: add it).
            if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
                *weapon = Weapon::new(new_weapon);
            } else {
                world.add_component(player, Weapon::new(new_weapon));
            }

            match current_weapon {
                // Drop the old weapon where the picked-up one was (swap in place).
                Some(old) => {
                    if let Some(dropped) = world.get_component_mut::<WeaponPickup>(pickup) {
                        dropped.weapon_type = old;
                    }
                }
                // Player was unarmed: just consume the pickup.
                None => world.despawn(pickup),
            }

            return Some(new_weapon);
        }

        None
    }
}

impl System for PickupSystem {
    /// Per-frame work: only the automatic part (dead enemies dropping their
    /// weapons). Collection is player-driven and handled via `swap_for_player`.
    fn run(&mut self, world: &mut World, _dt: f32) {
        Self::drop_from_dead_enemies(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Rotation, Speed, Velocity, AI};
    use crate::math::Vec2;

    fn spawn_dead_enemy(world: &mut World, pos: Vec2, weapon: WeaponType) -> Entity {
        let e = world.spawn();
        world.add_component(e, Enemy);
        world.add_component(e, Position::from_vec2(pos));
        world.add_component(e, Radius::new(12.0));
        world.add_component(
            e,
            Health {
                current: 0,
                max: 50,
            },
        );
        world.add_component(e, Weapon::new(weapon));
        e
    }

    fn spawn_test_player(world: &mut World, pos: Vec2) -> Entity {
        let p = world.spawn();
        world.add_component(p, Player);
        world.add_component(p, Position::from_vec2(pos));
        world.add_component(p, Velocity::zero());
        world.add_component(p, Speed::new(200.0));
        world.add_component(p, Radius::new(15.0));
        world.add_component(p, Rotation::new(0.0));
        world.add_component(p, Weapon::new(WeaponType::Pistol));
        p
    }

    #[test]
    fn test_dead_enemy_drops_weapon_pickup() {
        let mut world = World::new();
        let enemy = spawn_dead_enemy(&mut world, Vec2::new(100.0, 100.0), WeaponType::Shotgun);

        PickupSystem::drop_from_dead_enemies(&mut world);

        // Enemy no longer carries the weapon
        assert!(!world.has_component::<Weapon>(enemy));

        // Exactly one pickup exists, at the enemy's position, of the right type
        let pickups = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);
        let pickup = pickups[0];
        assert_eq!(
            world
                .get_component::<WeaponPickup>(pickup)
                .unwrap()
                .weapon_type,
            WeaponType::Shotgun
        );
        let pos = world.get_component::<Position>(pickup).unwrap();
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 100.0);
    }

    #[test]
    fn test_dead_enemy_drops_only_once() {
        let mut world = World::new();
        spawn_dead_enemy(&mut world, Vec2::new(0.0, 0.0), WeaponType::Pistol);

        PickupSystem::drop_from_dead_enemies(&mut world);
        PickupSystem::drop_from_dead_enemies(&mut world);

        assert_eq!(world.query::<WeaponPickup>().len(), 1);
    }

    #[test]
    fn test_alive_enemy_keeps_weapon() {
        let mut world = World::new();
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(0.0, 0.0));
        world.add_component(enemy, Health::new(50)); // alive
        world.add_component(enemy, Weapon::new(WeaponType::Pistol));

        PickupSystem::drop_from_dead_enemies(&mut world);

        assert!(world.has_component::<Weapon>(enemy));
        assert_eq!(world.query::<WeaponPickup>().len(), 0);
    }

    #[test]
    fn test_player_swaps_with_overlapping_pickup() {
        let mut world = World::new();
        let player = spawn_test_player(&mut world, Vec2::new(50.0, 50.0));

        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(WeaponType::MachineGun));
        world.add_component(pickup, Position::new(55.0, 50.0)); // within radius sum
        world.add_component(pickup, Radius::new(14.0));

        let swapped = PickupSystem::swap_for_player(&mut world);

        // Player now holds the machine gun...
        assert_eq!(swapped, Some(WeaponType::MachineGun));
        assert_eq!(
            world.get_component::<Weapon>(player).unwrap().weapon_type,
            WeaponType::MachineGun
        );
        // ...and their old pistol is dropped in place (still one pickup, now a pistol).
        let pickups = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);
        assert_eq!(
            world
                .get_component::<WeaponPickup>(pickups[0])
                .unwrap()
                .weapon_type,
            WeaponType::Pistol
        );
    }

    #[test]
    fn test_unarmed_player_consumes_pickup() {
        let mut world = World::new();
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));
        world.add_component(player, Radius::new(15.0));
        // No Weapon component: the player is unarmed.

        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(WeaponType::Shotgun));
        world.add_component(pickup, Position::new(5.0, 0.0));
        world.add_component(pickup, Radius::new(14.0));

        let swapped = PickupSystem::swap_for_player(&mut world);

        assert_eq!(swapped, Some(WeaponType::Shotgun));
        // Nothing to drop, so the pickup is consumed…
        assert_eq!(world.query::<WeaponPickup>().len(), 0);
        // …and the player now actually holds it (regression: it used to vanish).
        assert_eq!(
            world.get_component::<Weapon>(player).map(|w| w.weapon_type),
            Some(WeaponType::Shotgun)
        );
    }

    #[test]
    fn test_player_ignores_distant_pickup() {
        let mut world = World::new();
        let player = spawn_test_player(&mut world, Vec2::new(0.0, 0.0));

        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(WeaponType::Shotgun));
        world.add_component(pickup, Position::new(500.0, 500.0));
        world.add_component(pickup, Radius::new(14.0));

        let swapped = PickupSystem::swap_for_player(&mut world);

        assert_eq!(swapped, None);
        assert_eq!(
            world.get_component::<Weapon>(player).unwrap().weapon_type,
            WeaponType::Pistol // unchanged
        );
        assert_eq!(world.query::<WeaponPickup>().len(), 1);
    }

    #[test]
    fn test_full_drop_then_swap_flow() {
        let mut world = World::new();
        let player = spawn_test_player(&mut world, Vec2::new(100.0, 100.0));
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 100.0));
        world.add_component(enemy, Radius::new(12.0));
        world.add_component(
            enemy,
            Health {
                current: 0,
                max: 50,
            },
        );
        world.add_component(enemy, Rotation::new(0.0));
        world.add_component(enemy, AI::new());
        world.add_component(enemy, Weapon::new(WeaponType::Shotgun));

        let mut system = PickupSystem;
        // Frame update drops the dead enemy's shotgun on the floor.
        system.run(&mut world, 0.016);
        assert_eq!(world.query::<WeaponPickup>().len(), 1);

        // Player presses pick-up: swaps pistol for shotgun, pistol left behind.
        let swapped = PickupSystem::swap_for_player(&mut world);
        assert_eq!(swapped, Some(WeaponType::Shotgun));
        assert_eq!(
            world.get_component::<Weapon>(player).unwrap().weapon_type,
            WeaponType::Shotgun
        );
        let pickups = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);
        assert_eq!(
            world
                .get_component::<WeaponPickup>(pickups[0])
                .unwrap()
                .weapon_type,
            WeaponType::Pistol
        );
    }
}
