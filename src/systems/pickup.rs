use crate::components::{
    Downed, Enemy, GameEvent, Health, Player, Position, Radius, Weapon, WeaponPickup, WeaponType,
};
use crate::ecs::{Entity, System, World};

/// Rogue drops are never a full magazine: a downed rogue's weapon lands with a
/// random 30–100 % of its magazine (rounded up, at least one round), so
/// scavenging stays a gamble and the player keeps having to swap.
pub const DROP_AMMO_MIN_PERCENT: i32 = 30;

/// System that handles weapons dropped by downed enemies and their collection
/// by the player. Downed enemies drop their weapon on the floor; the player
/// swaps for it — ammo and all — by standing over it and pressing the pick-up
/// key. See [`PickupSystem::swap_for_player`]. It also does the "downed"
/// bookkeeping: the first frame any enemy's health is zero it is tagged
/// [`Downed`] and a [`GameEvent::EnemyDown`] is emitted, whatever killed it.
pub struct PickupSystem;

impl PickupSystem {
    /// Tag every newly dead enemy [`Downed`] and announce it once. Runs after
    /// every damage source in the frame (bullets, melee, thrown weapons, feral
    /// self-damage, scenario purges) so nothing has to remember to emit it.
    pub fn announce_downed(world: &mut World) {
        for enemy in world.query::<Enemy>() {
            let is_dead = world
                .get_component::<Health>(enemy)
                .map(|h| h.is_dead())
                .unwrap_or(false);
            if is_dead && !world.has_component::<Downed>(enemy) {
                world.add_component(enemy, Downed);
                world.push_event(GameEvent::EnemyDown);
            }
        }
    }

    /// Spawn a weapon pickup for any dead enemy that still carries a weapon.
    /// The weapon is removed from the enemy so it is only dropped once. The
    /// drop carries a partial magazine (see [`DROP_AMMO_MIN_PERCENT`]).
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

            let ammo = Self::drop_ammo(world, weapon.weapon_type);
            let pickup = world.spawn();
            world.add_component(pickup, WeaponPickup::with_ammo(weapon.weapon_type, ammo));
            world.add_component(pickup, pos);
            world.add_component(pickup, Radius::new(14.0));
        }
    }

    /// Rounds in a downed rogue's dropped weapon: a random 30–100 % of a
    /// magazine (rounded up, at least 1). Melee just keeps its nominal cap.
    fn drop_ammo(world: &mut World, weapon_type: WeaponType) -> i32 {
        let mag = weapon_type.magazine();
        if weapon_type.is_melee() {
            return mag;
        }
        let percent = world.random_int_range(DROP_AMMO_MIN_PERCENT, 100);
        ((mag * percent + 99) / 100).clamp(1, mag)
    }

    /// Swap the player's weapon with a pickup they are standing on (Hotline
    /// Miami rules): the player takes the pickup's weapon WITH the ammo it
    /// holds, and their previous weapon is dropped in its place WITH its
    /// remaining ammo, so nothing is ever lost, refilled, or duplicated. An
    /// unarmed player simply consumes the pickup.
    ///
    /// This is deliberately not run every frame — the caller gates it behind a
    /// key press so the player chooses when to swap. Returns the newly held
    /// weapon type if a swap happened, and emits [`GameEvent::Pickup`].
    pub fn swap_for_player(world: &mut World) -> Option<WeaponType> {
        let player = world.query::<Player>().into_iter().next()?;
        let player_pos = *world.get_component::<Position>(player)?;
        let player_radius = world
            .get_component::<Radius>(player)
            .map(|r| r.value)
            .unwrap_or(15.0);
        let current_weapon = world.get_component::<Weapon>(player).copied();

        let pickups: Vec<Entity> = world.query::<WeaponPickup>();
        for pickup in pickups {
            let (pickup_pos, pickup_radius, found) = match (
                world.get_component::<Position>(pickup),
                world.get_component::<Radius>(pickup),
                world.get_component::<WeaponPickup>(pickup),
            ) {
                (Some(p), Some(r), Some(w)) => (*p, r.value, *w),
                _ => continue,
            };

            if player_pos.distance_to(&pickup_pos) > player_radius + pickup_radius {
                continue;
            }

            // Hand the player the weapon exactly as it lies (an unarmed player —
            // e.g. right after a throw — has no Weapon component: add it).
            let new_weapon = Weapon::with_ammo(found.weapon_type, found.ammo);
            if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
                *weapon = new_weapon;
            } else {
                world.add_component(player, new_weapon);
            }

            match current_weapon {
                // Drop the old weapon, with its remaining rounds, where the
                // picked-up one was (swap in place).
                Some(old) => {
                    if let Some(dropped) = world.get_component_mut::<WeaponPickup>(pickup) {
                        *dropped = WeaponPickup::with_ammo(old.weapon_type, old.ammo);
                    }
                }
                // Player was unarmed: just consume the pickup.
                None => world.despawn(pickup),
            }

            world.push_event(GameEvent::Pickup);
            return Some(found.weapon_type);
        }

        None
    }
}

impl System for PickupSystem {
    /// Per-frame work: only the automatic part (noticing downed enemies and
    /// dropping their weapons). Collection is player-driven and handled via
    /// `swap_for_player`.
    fn run(&mut self, world: &mut World, _dt: f32) {
        Self::announce_downed(world);
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
        let dropped = *world.get_component::<WeaponPickup>(pickup).unwrap();
        assert_eq!(dropped.weapon_type, WeaponType::Shotgun);
        // Rogue drops carry a partial magazine: 30-100 %, at least one round.
        let mag = WeaponType::Shotgun.magazine();
        assert!(dropped.ammo >= 1 && dropped.ammo <= mag, "{}", dropped.ammo);
        assert!(dropped.ammo * 100 >= mag * DROP_AMMO_MIN_PERCENT - 99);
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
        // The player's pistol has been fired down to 7 rounds.
        world.get_component_mut::<Weapon>(player).unwrap().ammo = 7;

        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::with_ammo(WeaponType::MachineGun, 19));
        world.add_component(pickup, Position::new(55.0, 50.0)); // within radius sum
        world.add_component(pickup, Radius::new(14.0));

        let swapped = PickupSystem::swap_for_player(&mut world);

        // Player now holds the machine gun, with exactly the rounds it had...
        assert_eq!(swapped, Some(WeaponType::MachineGun));
        let held = world.get_component::<Weapon>(player).unwrap();
        assert_eq!(held.weapon_type, WeaponType::MachineGun);
        assert_eq!(held.ammo, 19);
        // ...and their old pistol is dropped in place with ITS remaining ammo
        // (still one pickup, now a 7-round pistol).
        let pickups = world.query::<WeaponPickup>();
        assert_eq!(pickups.len(), 1);
        let dropped = world.get_component::<WeaponPickup>(pickups[0]).unwrap();
        assert_eq!(dropped.weapon_type, WeaponType::Pistol);
        assert_eq!(dropped.ammo, 7);
        assert_eq!(world.drain_events(), vec![GameEvent::Pickup]);
    }

    #[test]
    fn test_ammo_round_trips_through_swap_back() {
        // Swap pistol(7) for shotgun(2), then swap back: the pistol still has 7
        // and the shotgun still has 2 — no refill anywhere.
        let mut world = World::new();
        let player = spawn_test_player(&mut world, Vec2::new(0.0, 0.0));
        world.get_component_mut::<Weapon>(player).unwrap().ammo = 7;
        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::with_ammo(WeaponType::Shotgun, 2));
        world.add_component(pickup, Position::new(0.0, 0.0));
        world.add_component(pickup, Radius::new(14.0));

        assert_eq!(
            PickupSystem::swap_for_player(&mut world),
            Some(WeaponType::Shotgun)
        );
        assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 2);
        assert_eq!(
            PickupSystem::swap_for_player(&mut world),
            Some(WeaponType::Pistol)
        );
        let held = world.get_component::<Weapon>(player).unwrap();
        assert_eq!((held.weapon_type, held.ammo), (WeaponType::Pistol, 7));
        let floor = world.get_component::<WeaponPickup>(pickup).unwrap();
        assert_eq!((floor.weapon_type, floor.ammo), (WeaponType::Shotgun, 2));
    }

    #[test]
    fn test_empty_gun_swaps_for_floor_weapon() {
        // The intended loop: gun runs dry -> swap it for what's on the floor.
        let mut world = World::new();
        let player = spawn_test_player(&mut world, Vec2::new(0.0, 0.0));
        world.get_component_mut::<Weapon>(player).unwrap().ammo = 0;
        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(WeaponType::Shotgun));
        world.add_component(pickup, Position::new(0.0, 0.0));
        world.add_component(pickup, Radius::new(14.0));

        PickupSystem::swap_for_player(&mut world);
        let held = world.get_component::<Weapon>(player).unwrap();
        assert!(held.can_fire());
        assert_eq!(held.ammo, WeaponType::Shotgun.magazine());
        // The empty pistol lies there, still empty.
        let floor = world.get_component::<WeaponPickup>(pickup).unwrap();
        assert_eq!((floor.weapon_type, floor.ammo), (WeaponType::Pistol, 0));
    }

    #[test]
    fn test_announce_downed_emits_once_per_enemy() {
        let mut world = World::new();
        let e = spawn_dead_enemy(&mut world, Vec2::new(0.0, 0.0), WeaponType::Pistol);
        let alive = world.spawn();
        world.add_component(alive, Enemy);
        world.add_component(alive, Health::new(50));

        let mut system = PickupSystem;
        system.run(&mut world, 0.016);
        system.run(&mut world, 0.016);
        assert_eq!(world.drain_events(), vec![GameEvent::EnemyDown]);
        assert!(world.has_component::<Downed>(e));
        assert!(!world.has_component::<Downed>(alive));

        // The survivor dies later: exactly one more.
        world
            .get_component_mut::<Health>(alive)
            .unwrap()
            .take_damage(99);
        system.run(&mut world, 0.016);
        assert_eq!(world.drain_events(), vec![GameEvent::EnemyDown]);
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
