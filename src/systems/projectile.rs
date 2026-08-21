use crate::collision;
use crate::components::{
    Bullet, Enemy, GameEvent, Health, Position, ProjectileTrail, Radius, Velocity,
};
use crate::ecs::{Entity, System, World};

/// System that updates and removes projectile trails
pub struct ProjectileTrailSystem;

impl System for ProjectileTrailSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let entities: Vec<_> = world.query::<ProjectileTrail>();

        // Update lifetimes and collect dead entities
        let mut dead_entities = Vec::new();

        for entity in entities {
            if let Some(trail) = world.get_component_mut::<ProjectileTrail>(entity) {
                trail.lifetime -= dt;

                if !trail.is_alive() {
                    dead_entities.push(entity);
                }
            }
        }

        // Remove dead trails
        for entity in dead_entities {
            world.despawn(entity);
        }
    }
}

/// System that updates bullets - movement, wall collision, enemy damage
pub struct BulletSystem;

impl System for BulletSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let bullets: Vec<Entity> = world.query::<Bullet>();
        // One enemy-list query for the whole tick (query allocates + sorts):
        // nothing spawns mid-loop, and enemies killed by an earlier bullet
        // are skipped by the per-bullet Health check below.
        let enemies: Vec<Entity> = world.query::<Enemy>();
        let mut bullets_to_remove = Vec::new();

        for bullet_entity in bullets {
            let (bullet, bullet_pos, bullet_vel) = match (
                world.get_component::<Bullet>(bullet_entity),
                world.get_component::<Position>(bullet_entity),
                world.get_component::<Velocity>(bullet_entity),
            ) {
                (Some(b), Some(p), Some(v)) => (*b, *p, *v),
                _ => continue,
            };

            // Check if bullet has expired
            if !bullet.is_alive() {
                bullets_to_remove.push(bullet_entity);
                continue;
            }

            // Calculate new position
            let new_x = bullet_pos.x + bullet_vel.x * dt;
            let new_y = bullet_pos.y + bullet_vel.y * dt;

            // Check wall collision (bullets are small, use 2.0 radius).
            // Swept old->new segment check: a bullet covers ~13 px per 60 Hz
            // frame, so an endpoint-only test would tunnel straight through
            // walls thinner than that.
            let bullet_radius = 2.0;
            let walls = world.walls();
            let mut hit_wall = false;

            for wall in walls {
                if collision::swept_circle_rect_collision(
                    crate::math::Vec2::new(bullet_pos.x, bullet_pos.y),
                    crate::math::Vec2::new(new_x, new_y),
                    bullet_radius,
                    wall.x,
                    wall.y,
                    wall.width,
                    wall.height,
                ) {
                    hit_wall = true;
                    break;
                }
            }

            if hit_wall {
                bullets_to_remove.push(bullet_entity);
                continue;
            }

            // Check enemy collision
            let mut hit_enemy = false;

            for &enemy_entity in &enemies {
                let (enemy_pos, enemy_radius, enemy_health) = match (
                    world.get_component::<Position>(enemy_entity),
                    world.get_component::<Radius>(enemy_entity),
                    world.get_component::<Health>(enemy_entity),
                ) {
                    (Some(p), Some(r), Some(h)) => (*p, *r, *h),
                    _ => continue,
                };

                // Skip dead enemies
                if enemy_health.is_dead() {
                    continue;
                }

                // Swept old->new check: at closing speeds above the
                // combined radii per frame (a bullet meeting a rushing bot
                // head-on) an endpoint-only test tunnels straight through.
                if collision::swept_circle_circle_collision(
                    crate::math::Vec2::new(bullet_pos.x, bullet_pos.y),
                    crate::math::Vec2::new(new_x, new_y),
                    bullet_radius,
                    crate::math::Vec2::new(enemy_pos.x, enemy_pos.y),
                    enemy_radius.value,
                ) {
                    // Deal damage
                    let mut killed = false;
                    if let Some(health) = world.get_component_mut::<Health>(enemy_entity) {
                        health.take_damage(bullet.damage);
                        killed = health.is_dead();
                    }
                    world.push_event(GameEvent::EnemyHit {
                        by: bullet.weapon_type,
                    });
                    // Shove the enemy along the bullet's travel direction — the
                    // live combat knockback (process_shoot is test-only; real
                    // bullet damage resolves here in BulletSystem).
                    crate::systems::combat::CombatSystem::apply_knockback(
                        world,
                        enemy_entity,
                        bullet_vel.x,
                        bullet_vel.y,
                        crate::systems::combat::BULLET_KNOCKBACK,
                    );
                    // A killing round lays the corpse out along its flight:
                    // sprawled away from the shooter, head first.
                    if killed {
                        crate::systems::combat::CombatSystem::record_corpse_fall(
                            world,
                            enemy_entity,
                            bullet_vel.x,
                            bullet_vel.y,
                        );
                    }
                    hit_enemy = true;
                    break;
                }
            }

            if hit_enemy {
                bullets_to_remove.push(bullet_entity);
                continue;
            }

            // Update bullet position and lifetime
            if let Some(pos) = world.get_component_mut::<Position>(bullet_entity) {
                pos.x = new_x;
                pos.y = new_y;
            }

            if let Some(b) = world.get_component_mut::<Bullet>(bullet_entity) {
                b.lifetime -= dt;
            }
        }

        // Remove bullets that hit something or expired
        for bullet_entity in bullets_to_remove {
            world.despawn(bullet_entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Position;

    #[test]
    fn test_projectile_trail_system_decreases_lifetime() {
        let mut world = World::new();
        let entity = world.spawn();

        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 100.0);
        world.add_component(entity, ProjectileTrail::new(start, end));

        let mut system = ProjectileTrailSystem;
        system.run(&mut world, 0.1);

        let trail = world.get_component::<ProjectileTrail>(entity).unwrap();
        assert!((trail.lifetime - 0.05).abs() < 0.001); // 0.15 - 0.1 = 0.05
    }

    #[test]
    fn test_projectile_trail_system_removes_dead_trails() {
        let mut world = World::new();
        let entity = world.spawn();

        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 100.0);
        world.add_component(entity, ProjectileTrail::new(start, end));

        let mut system = ProjectileTrailSystem;
        // Run for longer than trail lifetime
        system.run(&mut world, 0.2);

        // Trail should be removed
        assert!(world.get_component::<ProjectileTrail>(entity).is_none());
    }

    #[test]
    fn test_bullet_hit_damages_and_announces_weapon() {
        use crate::components::WeaponType;
        let mut world = World::new();
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(50.0, 0.0));
        world.add_component(enemy, Radius::new(12.0));
        world.add_component(enemy, Health::new(100));

        let bullet = world.spawn();
        world.add_component(bullet, Bullet::new(WeaponType::Shotgun, 30));
        world.add_component(bullet, Position::new(0.0, 0.0));
        world.add_component(bullet, Velocity::new(800.0, 0.0));
        world.add_component(bullet, Radius::new(2.0));

        let mut system = BulletSystem;
        for _ in 0..10 {
            system.run(&mut world, 0.016);
            if world.query::<Bullet>().is_empty() {
                break;
            }
        }
        assert!(world.query::<Bullet>().is_empty(), "bullet consumed on hit");
        assert_eq!(world.get_component::<Health>(enemy).unwrap().current, 70);
        assert_eq!(
            world.drain_events(),
            vec![GameEvent::EnemyHit {
                by: WeaponType::Shotgun
            }]
        );
    }

    #[test]
    fn test_bullet_into_wall_is_silent() {
        use crate::components::WeaponType;
        let mut world = World::new();
        world.add_wall(40.0, -50.0, 20.0, 100.0);
        let bullet = world.spawn();
        world.add_component(bullet, Bullet::new(WeaponType::Pistol, 30));
        world.add_component(bullet, Position::new(0.0, 0.0));
        world.add_component(bullet, Velocity::new(800.0, 0.0));
        world.add_component(bullet, Radius::new(2.0));

        let mut system = BulletSystem;
        for _ in 0..10 {
            system.run(&mut world, 0.016);
        }
        assert!(world.query::<Bullet>().is_empty());
        assert!(world.drain_events().is_empty());
    }

    /// TASK-3 compass rig: a shooter placed at `offset` from the victim
    /// fires a pistol round straight AT the victim (one hit kills). Asserts
    /// the knockback impulse points attacker -> victim (per-axis signs and a
    /// positive dot with the shot) and the recorded corpse fall angle equals
    /// `atan2(dy, dx)` of attacker -> victim within 0.01 rad.
    fn corpse_falls_away_from(offset: crate::math::Vec2) {
        use crate::components::{Knockback, Stunned};
        use crate::game::{fire_player_weapon, spawn_player};
        use crate::math::Vec2;

        let victim_pos = Vec2::new(400.0, 400.0);
        let shooter_pos = victim_pos + offset;

        let mut world = World::new();
        let victim = world.spawn();
        world.add_component(victim, Enemy);
        world.add_component(victim, Position::new(victim_pos.x, victim_pos.y));
        world.add_component(victim, Radius::new(12.0));
        world.add_component(victim, Health::new(10)); // one pistol round kills

        spawn_player(&mut world, shooter_pos); // holds a pistol
        assert!(!fire_player_weapon(&mut world, victim_pos)); // round spawned
        assert_eq!(world.query::<Bullet>().len(), 1);

        // Fly the round home (no movement system runs, so the knockback
        // impulse is still untouched when we inspect it).
        let mut system = BulletSystem;
        for _ in 0..60 {
            system.run(&mut world, 0.016);
            if world.query::<Bullet>().is_empty() {
                break;
            }
        }
        assert!(world.query::<Bullet>().is_empty(), "bullet resolved");
        assert!(
            world.get_component::<Health>(victim).unwrap().is_dead(),
            "one round kills"
        );

        // (a) The shove points attacker -> victim.
        let shot = victim_pos - shooter_pos; // == -offset
        let kb = *world
            .get_component::<Knockback>(victim)
            .expect("the killing round shoves the victim");
        for (kb_axis, shot_axis) in [(kb.x, shot.x), (kb.y, shot.y)] {
            if shot_axis == 0.0 {
                assert!(kb_axis.abs() < 1e-3, "no sideways shove: {kb_axis}");
            } else {
                assert!(
                    kb_axis * shot_axis > 0.0,
                    "shove sign follows the shot: kb {kb_axis} vs shot {shot_axis}"
                );
            }
        }
        assert!(
            kb.x * shot.x + kb.y * shot.y > 0.0,
            "shove has positive dot with the shot direction"
        );

        // (b) The corpse's recorded fall angle is attacker -> victim.
        let fall = world
            .get_component::<Stunned>(victim)
            .expect("the kill recorded a corpse fall")
            .fall_angle;
        let expected = shot.y.atan2(shot.x);
        let diff = (fall - expected).rem_euclid(std::f32::consts::TAU);
        let wrapped = diff.min(std::f32::consts::TAU - diff);
        assert!(wrapped < 0.01, "fall {fall} != expected {expected}");
    }

    #[test]
    fn corpse_falls_away_nw() {
        corpse_falls_away_from(crate::math::Vec2::new(-100.0, -100.0));
    }

    #[test]
    fn corpse_falls_away_nn() {
        corpse_falls_away_from(crate::math::Vec2::new(0.0, -100.0));
    }

    #[test]
    fn corpse_falls_away_ne() {
        corpse_falls_away_from(crate::math::Vec2::new(100.0, -100.0));
    }

    #[test]
    fn corpse_falls_away_ww() {
        corpse_falls_away_from(crate::math::Vec2::new(-100.0, 0.0));
    }

    #[test]
    fn corpse_falls_away_ee() {
        corpse_falls_away_from(crate::math::Vec2::new(100.0, 0.0));
    }

    #[test]
    fn corpse_falls_away_sw() {
        corpse_falls_away_from(crate::math::Vec2::new(-100.0, 100.0));
    }

    #[test]
    fn corpse_falls_away_ss() {
        corpse_falls_away_from(crate::math::Vec2::new(0.0, 100.0));
    }

    #[test]
    fn corpse_falls_away_se() {
        corpse_falls_away_from(crate::math::Vec2::new(100.0, 100.0));
    }

    #[test]
    fn test_projectile_trail_system_multiple_trails() {
        let mut world = World::new();

        // Create multiple trails
        for i in 0..3 {
            let entity = world.spawn();
            let start = Position::new(i as f32 * 10.0, 0.0);
            let end = Position::new(i as f32 * 10.0 + 100.0, 100.0);
            world.add_component(entity, ProjectileTrail::new(start, end));
        }

        let mut system = ProjectileTrailSystem;
        system.run(&mut world, 0.1);

        // All trails should still exist
        let trails = world.query::<ProjectileTrail>();
        assert_eq!(trails.len(), 3);

        // Run until all trails are dead
        system.run(&mut world, 0.2);

        // All trails should be removed
        let trails = world.query::<ProjectileTrail>();
        assert_eq!(trails.len(), 0);
    }
}
