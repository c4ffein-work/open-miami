use crate::collision;
use crate::components::{
    Enemy, Health, Player, Position, Radius, Stunned, ThrownWeapon, Weapon, WeaponPickup,
    WeaponType,
};
use crate::ecs::{Entity, System, World};
use crate::math::Vec2;

/// How fast a thrown weapon travels (pixels/second).
pub const THROW_SPEED: f32 = 700.0;
/// How far a thrown weapon flies before dropping to the floor (pixels).
pub const THROW_RANGE: f32 = 550.0;
/// Chip damage a thrown weapon deals on impact (the knockdown is the point).
pub const THROW_DAMAGE: i32 = 15;
/// How long an enemy stays knocked down after being hit by a thrown weapon.
pub const STUN_DURATION: f32 = 3.0;
/// Collision radius of a weapon in flight.
const THROWN_RADIUS: f32 = 10.0;

/// System that moves thrown weapons, resolves their impacts, and turns them
/// back into floor pickups when they land.
pub struct ThrownWeaponSystem;

impl ThrownWeaponSystem {
    /// Throw the player's currently held weapon in `aim_dir`. The player is left
    /// unarmed (their `Weapon` is removed) until they pick another one up.
    /// Returns `true` if a weapon was actually thrown.
    pub fn throw_from_player(world: &mut World, aim_dir: Vec2) -> bool {
        let player = match world.query::<Player>().into_iter().next() {
            Some(p) => p,
            None => return false,
        };
        let weapon_type = match world.get_component::<Weapon>(player) {
            Some(w) => w.weapon_type,
            None => return false, // nothing in hand to throw
        };
        let pos = match world.get_component::<Position>(player) {
            Some(p) => *p,
            None => return false,
        };

        // A zero-length aim can't produce a direction.
        if aim_dir.length() == 0.0 {
            return false;
        }

        world.remove_component::<Weapon>(player);

        let thrown = world.spawn();
        world.add_component(
            thrown,
            ThrownWeapon::new(weapon_type, THROW_DAMAGE, aim_dir, THROW_SPEED, THROW_RANGE),
        );
        world.add_component(thrown, pos);
        world.add_component(thrown, Radius::new(THROWN_RADIUS));

        true
    }

    /// Drop a thrown weapon to the floor as a pickup at `pos`.
    fn land(world: &mut World, thrown: Entity, pos: Position, weapon_type: WeaponType) {
        world.despawn(thrown);
        let pickup = world.spawn();
        world.add_component(pickup, WeaponPickup::new(weapon_type));
        world.add_component(pickup, pos);
        world.add_component(pickup, Radius::new(14.0));
    }

    /// First alive, not-already-stunned enemy whose body the flight path enters.
    fn enemy_hit_at(world: &World, point: Vec2) -> Option<Entity> {
        for enemy in world.query::<Enemy>() {
            let (pos, radius, health) = match (
                world.get_component::<Position>(enemy),
                world.get_component::<Radius>(enemy),
                world.get_component::<Health>(enemy),
            ) {
                (Some(p), Some(r), Some(h)) => (*p, r.value, *h),
                _ => continue,
            };
            if health.is_dead() || world.has_component::<Stunned>(enemy) {
                continue;
            }
            if collision::circle_circle_collision(point, THROWN_RADIUS, pos.to_vec2(), radius) {
                return Some(enemy);
            }
        }
        None
    }
}

impl System for ThrownWeaponSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        for thrown in world.query::<ThrownWeapon>() {
            let tw = match world.get_component::<ThrownWeapon>(thrown) {
                Some(t) => *t,
                None => continue,
            };
            let pos = match world.get_component::<Position>(thrown) {
                Some(p) => *p,
                None => continue,
            };

            let step = (tw.vx * tw.vx + tw.vy * tw.vy).sqrt() * dt;
            let new_point = Vec2::new(pos.x + tw.vx * dt, pos.y + tw.vy * dt);
            let new_pos = Position::new(new_point.x, new_point.y);

            // Hit a wall? Drop where it struck.
            let hit_wall = world.walls().iter().any(|wall| {
                collision::circle_rect_collision(
                    new_point,
                    THROWN_RADIUS,
                    wall.x,
                    wall.y,
                    wall.width,
                    wall.height,
                )
            });
            if hit_wall {
                Self::land(world, thrown, pos, tw.weapon_type);
                continue;
            }

            // Hit an enemy? Knock them down, deal chip damage, drop the weapon.
            if let Some(enemy) = Self::enemy_hit_at(world, new_point) {
                if let Some(health) = world.get_component_mut::<Health>(enemy) {
                    health.take_damage(tw.damage);
                }
                world.add_component(enemy, Stunned::new(STUN_DURATION));
                Self::land(world, thrown, new_pos, tw.weapon_type);
                continue;
            }

            // Out of range? It falls to the ground.
            if tw.distance_remaining - step <= 0.0 {
                Self::land(world, thrown, new_pos, tw.weapon_type);
                continue;
            }

            // Still flying: advance it.
            if let Some(p) = world.get_component_mut::<Position>(thrown) {
                p.x = new_point.x;
                p.y = new_point.y;
            }
            if let Some(t) = world.get_component_mut::<ThrownWeapon>(thrown) {
                t.distance_remaining -= step;
                t.spin += dt * 20.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player_with_weapon(world: &mut World, pos: Vec2, weapon: WeaponType) -> Entity {
        let p = world.spawn();
        world.add_component(p, Player);
        world.add_component(p, Position::from_vec2(pos));
        world.add_component(p, Radius::new(15.0));
        world.add_component(p, Weapon::new(weapon));
        p
    }

    #[test]
    fn test_throw_disarms_player_and_spawns_projectile() {
        let mut world = World::new();
        let player = player_with_weapon(&mut world, Vec2::new(100.0, 100.0), WeaponType::Shotgun);

        let thrown = ThrownWeaponSystem::throw_from_player(&mut world, Vec2::new(1.0, 0.0));

        assert!(thrown);
        assert!(!world.has_component::<Weapon>(player)); // disarmed
        let projectiles = world.query::<ThrownWeapon>();
        assert_eq!(projectiles.len(), 1);
        assert_eq!(
            world
                .get_component::<ThrownWeapon>(projectiles[0])
                .unwrap()
                .weapon_type,
            WeaponType::Shotgun
        );
    }

    #[test]
    fn test_unarmed_player_cannot_throw() {
        let mut world = World::new();
        let p = world.spawn();
        world.add_component(p, Player);
        world.add_component(p, Position::new(0.0, 0.0));
        // no weapon

        assert!(!ThrownWeaponSystem::throw_from_player(
            &mut world,
            Vec2::new(1.0, 0.0)
        ));
        assert_eq!(world.query::<ThrownWeapon>().len(), 0);
    }

    #[test]
    fn test_thrown_weapon_stuns_enemy_and_drops() {
        let mut world = World::new();
        player_with_weapon(&mut world, Vec2::new(0.0, 100.0), WeaponType::Pistol);

        // Enemy just to the right, in the throw's path.
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(40.0, 100.0));
        world.add_component(enemy, Radius::new(12.0));
        world.add_component(enemy, Health::new(50));

        ThrownWeaponSystem::throw_from_player(&mut world, Vec2::new(1.0, 0.0));

        // Step until it reaches the enemy.
        let mut system = ThrownWeaponSystem;
        for _ in 0..10 {
            system.run(&mut world, 0.016);
            if world.has_component::<Stunned>(enemy) {
                break;
            }
        }

        assert!(
            world.has_component::<Stunned>(enemy),
            "enemy should be stunned"
        );
        // The projectile became a floor pickup.
        assert_eq!(world.query::<ThrownWeapon>().len(), 0);
        assert_eq!(world.query::<WeaponPickup>().len(), 1);
        // Enemy took chip damage.
        assert_eq!(
            world.get_component::<Health>(enemy).unwrap().current,
            50 - THROW_DAMAGE
        );
    }

    #[test]
    fn test_thrown_weapon_lands_after_max_range() {
        let mut world = World::new();
        player_with_weapon(&mut world, Vec2::new(0.0, 0.0), WeaponType::MachineGun);
        ThrownWeaponSystem::throw_from_player(&mut world, Vec2::new(1.0, 0.0));

        let mut system = ThrownWeaponSystem;
        // No walls, no enemies: it should travel its range and drop.
        for _ in 0..200 {
            system.run(&mut world, 0.016);
            if world.query::<ThrownWeapon>().is_empty() {
                break;
            }
        }

        assert_eq!(world.query::<ThrownWeapon>().len(), 0);
        assert_eq!(world.query::<WeaponPickup>().len(), 1);
    }
}
