use crate::components::{AIState, Enemy, Health, Player, Position, Radius, AI};
use crate::ecs::{Entity, System, World};

/// System that handles combat damage dealing
pub struct CombatSystem;

impl CombatSystem {
    /// Check if a line segment (bullet) intersects with a circle (enemy)
    fn line_circle_collision(
        start: &Position,
        end: &Position,
        circle: &Position,
        radius: f32,
    ) -> bool {
        // Vector from start to end
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let len_sq = dx * dx + dy * dy;

        if len_sq == 0.0 {
            // Start and end are the same point
            return start.distance_to(circle) <= radius;
        }

        // Vector from start to circle
        let fx = circle.x - start.x;
        let fy = circle.y - start.y;

        // Project circle onto line
        let t = ((fx * dx + fy * dy) / len_sq).clamp(0.0, 1.0);

        // Closest point on line to circle
        let closest_x = start.x + t * dx;
        let closest_y = start.y + t * dy;

        // Distance from closest point to circle center
        let dist_x = circle.x - closest_x;
        let dist_y = circle.y - closest_y;
        let dist_sq = dist_x * dist_x + dist_y * dist_y;

        dist_sq <= radius * radius
    }

    /// Process shooting from one position to another
    pub fn process_shoot(
        world: &mut World,
        shooter_pos: Position,
        target_pos: Position,
        damage: i32,
    ) -> bool {
        let enemies: Vec<Entity> = world.query::<Enemy>();

        for enemy in enemies {
            let (enemy_pos, enemy_radius, enemy_health) = match (
                world.get_component::<Position>(enemy),
                world.get_component::<Radius>(enemy),
                world.get_component::<Health>(enemy),
            ) {
                (Some(pos), Some(rad), Some(hp)) => (*pos, *rad, *hp),
                _ => continue,
            };

            // Skip dead enemies
            if enemy_health.is_dead() {
                continue;
            }

            // Check if bullet line hits enemy circle
            if Self::line_circle_collision(
                &shooter_pos,
                &target_pos,
                &enemy_pos,
                enemy_radius.value,
            ) {
                // Deal damage
                if let Some(health) = world.get_component_mut::<Health>(enemy) {
                    health.take_damage(damage);
                    return true; // Hit confirmed
                }
            }
        }

        false // No hit
    }

    /// Process melee attack in a cone
    pub fn process_melee(
        world: &mut World,
        attacker_pos: Position,
        target_pos: Position,
        damage: i32,
        range: f32,
    ) -> bool {
        let enemies: Vec<Entity> = world.query::<Enemy>();

        // Direction to target
        let dx = target_pos.x - attacker_pos.x;
        let dy = target_pos.y - attacker_pos.y;
        let target_angle = dy.atan2(dx);

        let mut hit_any = false;

        for enemy in enemies {
            let (enemy_pos, enemy_health) = match (
                world.get_component::<Position>(enemy),
                world.get_component::<Health>(enemy),
            ) {
                (Some(pos), Some(hp)) => (*pos, *hp),
                _ => continue,
            };

            // Skip dead enemies
            if enemy_health.is_dead() {
                continue;
            }

            let distance = attacker_pos.distance_to(&enemy_pos);
            if distance > range {
                continue;
            }

            // Check angle (90 degree cone)
            let enemy_dx = enemy_pos.x - attacker_pos.x;
            let enemy_dy = enemy_pos.y - attacker_pos.y;
            let enemy_angle = enemy_dy.atan2(enemy_dx);

            let angle_diff = (enemy_angle - target_angle).abs();
            let normalized_angle = if angle_diff > std::f32::consts::PI {
                2.0 * std::f32::consts::PI - angle_diff
            } else {
                angle_diff
            };

            if normalized_angle < std::f32::consts::PI / 4.0 {
                // Within 45 degree cone (90 degrees total)
                if let Some(health) = world.get_component_mut::<Health>(enemy) {
                    health.take_damage(damage);
                    hit_any = true;
                }
            }
        }

        hit_any
    }

    /// Process enemy attacks on player
    fn process_enemy_attacks(world: &mut World) {
        // Find player
        let player_entity = match world.query::<Player>().first() {
            Some(&e) => e,
            None => return,
        };

        let player_pos = match world.get_component::<Position>(player_entity) {
            Some(pos) => *pos,
            None => return,
        };

        // Check all enemies in attack state
        let enemies: Vec<Entity> = world.query::<Enemy>();

        for enemy in enemies {
            let (ai, enemy_pos, enemy_health) = match (
                world.get_component::<AI>(enemy),
                world.get_component::<Position>(enemy),
                world.get_component::<Health>(enemy),
            ) {
                (Some(ai), Some(pos), Some(hp)) => (*ai, *pos, *hp),
                _ => continue,
            };

            // Skip dead enemies
            if enemy_health.is_dead() {
                continue;
            }

            if ai.state == AIState::Attack && ai.can_attack() {
                let distance = enemy_pos.distance_to(&player_pos);
                if distance < ai.attack_range {
                    // Deal damage to player
                    if let Some(health) = world.get_component_mut::<Health>(player_entity) {
                        health.take_damage(10); // Enemy damage
                    }

                    // Reset cooldown
                    if let Some(ai) = world.get_component_mut::<AI>(enemy) {
                        ai.reset_attack_timer();
                    }
                }
            }
        }
    }
}

impl System for CombatSystem {
    fn run(&mut self, world: &mut World, _dt: f32) {
        Self::process_enemy_attacks(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_circle_collision_hit() {
        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 0.0);
        let circle = Position::new(50.0, 5.0);
        let radius = 10.0;

        assert!(CombatSystem::line_circle_collision(
            &start, &end, &circle, radius
        ));
    }

    #[test]
    fn test_line_circle_collision_miss() {
        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 0.0);
        let circle = Position::new(50.0, 20.0);
        let radius = 10.0;

        assert!(!CombatSystem::line_circle_collision(
            &start, &end, &circle, radius
        ));
    }

    #[test]
    fn test_line_circle_collision_direct_hit() {
        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 0.0);
        let circle = Position::new(50.0, 0.0); // Directly on line
        let radius = 5.0;

        assert!(CombatSystem::line_circle_collision(
            &start, &end, &circle, radius
        ));
    }

    #[test]
    fn test_process_shoot_hits_enemy() {
        let mut world = World::new();

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(50.0, 0.0));
        world.add_component(enemy, Radius::new(10.0));
        world.add_component(enemy, Health::new(100));

        let shooter_pos = Position::new(0.0, 0.0);
        let target_pos = Position::new(100.0, 0.0);

        let hit = CombatSystem::process_shoot(&mut world, shooter_pos, target_pos, 30);

        assert!(hit);
        let health = world.get_component::<Health>(enemy).unwrap();
        assert_eq!(health.current, 70);
    }

    #[test]
    fn test_process_shoot_misses_enemy() {
        let mut world = World::new();

        // Create enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(50.0, 50.0)); // Off to the side
        world.add_component(enemy, Radius::new(10.0));
        world.add_component(enemy, Health::new(100));

        let shooter_pos = Position::new(0.0, 0.0);
        let target_pos = Position::new(100.0, 0.0);

        let hit = CombatSystem::process_shoot(&mut world, shooter_pos, target_pos, 30);

        assert!(!hit);
        let health = world.get_component::<Health>(enemy).unwrap();
        assert_eq!(health.current, 100); // No damage
    }

    #[test]
    fn test_process_shoot_ignores_dead_enemies() {
        let mut world = World::new();

        // Create dead enemy
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(50.0, 0.0));
        world.add_component(enemy, Radius::new(10.0));
        world.add_component(
            enemy,
            Health {
                current: 0,
                max: 100,
            },
        );

        let shooter_pos = Position::new(0.0, 0.0);
        let target_pos = Position::new(100.0, 0.0);

        let hit = CombatSystem::process_shoot(&mut world, shooter_pos, target_pos, 30);

        assert!(!hit); // Dead enemies don't count as hits
    }

    #[test]
    fn test_process_melee_in_range() {
        let mut world = World::new();

        // Create enemy in melee range
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(100));

        let attacker_pos = Position::new(0.0, 0.0);
        let target_pos = Position::new(100.0, 0.0);

        let hit = CombatSystem::process_melee(&mut world, attacker_pos, target_pos, 50, 50.0);

        assert!(hit);
        let health = world.get_component::<Health>(enemy).unwrap();
        assert_eq!(health.current, 50);
    }

    #[test]
    fn test_process_melee_out_of_range() {
        let mut world = World::new();

        // Create enemy out of range
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(100.0, 0.0));
        world.add_component(enemy, Health::new(100));

        let attacker_pos = Position::new(0.0, 0.0);
        let target_pos = Position::new(100.0, 0.0);

        let hit = CombatSystem::process_melee(&mut world, attacker_pos, target_pos, 50, 50.0);

        assert!(!hit);
        let health = world.get_component::<Health>(enemy).unwrap();
        assert_eq!(health.current, 100);
    }

    #[test]
    fn test_combat_system_enemy_attacks_player() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));
        world.add_component(player, Health::new(100));

        // Create enemy in attack range
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(100));
        let mut ai = AI::new();
        ai.state = AIState::Attack;
        world.add_component(enemy, ai);

        let mut system = CombatSystem;
        system.run(&mut world, 0.016);

        let player_health = world.get_component::<Health>(player).unwrap();
        assert_eq!(player_health.current, 90); // Took 10 damage
    }

    #[test]
    fn test_combat_system_respects_attack_cooldown() {
        let mut world = World::new();

        // Create player
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));
        world.add_component(player, Health::new(100));

        // Create enemy with cooldown
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(100));
        let mut ai = AI::new();
        ai.state = AIState::Attack;
        ai.reset_attack_timer(); // Cooldown active
        world.add_component(enemy, ai);

        let mut system = CombatSystem;
        system.run(&mut world, 0.016);

        let player_health = world.get_component::<Health>(player).unwrap();
        assert_eq!(player_health.current, 100); // No damage due to cooldown
    }
}
