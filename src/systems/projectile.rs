use crate::collision;
use crate::components::{Bullet, Enemy, Health, Position, ProjectileTrail, Radius, Velocity};
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

            // Check wall collision (bullets are small, use 2.0 radius)
            let bullet_radius = 2.0;
            let walls = world.walls();
            let mut hit_wall = false;

            for wall in walls {
                if collision::circle_rect_collision(
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
            let enemies: Vec<Entity> = world.query::<Enemy>();
            let mut hit_enemy = false;

            for enemy_entity in enemies {
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

                // Check circle-circle collision
                if collision::circle_circle_collision(
                    crate::math::Vec2::new(new_x, new_y),
                    bullet_radius,
                    crate::math::Vec2::new(enemy_pos.x, enemy_pos.y),
                    enemy_radius.value,
                ) {
                    // Deal damage
                    if let Some(health) = world.get_component_mut::<Health>(enemy_entity) {
                        health.take_damage(bullet.damage);
                    }
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
