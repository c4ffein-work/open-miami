use crate::components::{
    AIState, Boss, Enemy, GameEvent, Health, Knockback, Player, Position, Radius, Stunned,
    WeaponType, AI,
};
use crate::ecs::{Entity, System, World};

/// Damage an enemy deals to the player per contact attack. Kept high so a swarm
/// is genuinely lethal (a few hits kills the player), matching the fast, fragile
/// feel of the genre while still giving the player a chance to react.
pub const ENEMY_ATTACK_DAMAGE: i32 = 25;

/// Contact damage from the boss while its mask is still on (menacing but slow).
pub const BOSS_MASK_DAMAGE: i32 = 15;
/// Contact damage from the boss once the mask cracks off (frantic and deadly).
pub const BOSS_ENRAGED_DAMAGE: i32 = 45;

/// Impulse speed (px/s) a single bullet imparts to the enemy it strikes, thrown
/// along the bullet's travel direction. With the movement system's decay this is
/// a shove of a few dozen pixels — punchy, but nowhere near a room-crossing fling.
pub const BULLET_KNOCKBACK: f32 = 500.0;

/// Melee connects with the whole body, so it shoves the target noticeably harder
/// than a single bullet (matching the heavier feel of a swing).
pub const MELEE_KNOCKBACK: f32 = 1000.0;

/// How hard an enemy's contact attack shoves the player straight away from it.
pub const PLAYER_KNOCKBACK: f32 = 650.0;

/// System that handles combat damage dealing
pub struct CombatSystem;

impl CombatSystem {
    /// Apply a directional knockback impulse to `entity`, shoving it along the
    /// `(dx, dy)` direction at `speed` px/s (the vector is normalised here). A
    /// zero-length direction (attacker and target exactly overlapping) is
    /// ignored. If the entity is already being shoved, the new impulse stacks
    /// additively with the one still in flight.
    pub(crate) fn apply_knockback(world: &mut World, entity: Entity, dx: f32, dy: f32, speed: f32) {
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON {
            return;
        }
        let ix = dx / len * speed;
        let iy = dy / len * speed;
        if let Some(kb) = world.get_component_mut::<Knockback>(entity) {
            kb.x += ix;
            kb.y += iy;
        } else {
            world.add_component(entity, Knockback::new(ix, iy));
        }
    }

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
                    // Shove the enemy along the bullet's travel direction
                    // (shooter -> target), i.e. away from the shooter.
                    let dir_x = target_pos.x - shooter_pos.x;
                    let dir_y = target_pos.y - shooter_pos.y;
                    Self::apply_knockback(world, enemy, dir_x, dir_y, BULLET_KNOCKBACK);
                    return true; // Hit confirmed
                }
            }
        }

        false // No hit
    }

    /// Process melee attack in a cone. Emits one [`GameEvent::EnemyHit`] (by
    /// melee) per enemy struck.
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
                    world.push_event(GameEvent::EnemyHit {
                        by: WeaponType::Melee,
                    });
                }
                // Shove the enemy away from the attacker (attacker -> enemy).
                let dir_x = enemy_pos.x - attacker_pos.x;
                let dir_y = enemy_pos.y - attacker_pos.y;
                Self::apply_knockback(world, enemy, dir_x, dir_y, MELEE_KNOCKBACK);
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

        // A downed player is not a target: no more hits, knockback or hurt sounds.
        if world
            .get_component::<Health>(player_entity)
            .is_some_and(|h| h.is_dead())
        {
            return;
        }

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

            // Knocked-down enemies can't attack.
            if world.has_component::<Stunned>(enemy) {
                continue;
            }

            // Attack if in SurePlayerSeen state and within range
            if ai.state == AIState::SurePlayerSeen && ai.can_attack() {
                let distance = enemy_pos.distance_to(&player_pos);
                if distance < ai.attack_range {
                    // The boss hits harder than a regular rogue (harder still
                    // once its mask is off).
                    let damage = match world.get_component::<Boss>(enemy) {
                        Some(boss) if boss.enraged => BOSS_ENRAGED_DAMAGE,
                        Some(_) => BOSS_MASK_DAMAGE,
                        None => ENEMY_ATTACK_DAMAGE,
                    };
                    // Deal damage to player
                    if let Some(health) = world.get_component_mut::<Health>(player_entity) {
                        health.take_damage(damage);
                    }
                    world.push_event(GameEvent::PlayerHurt);

                    // Shove the player directly away from the attacking enemy.
                    let dir_x = player_pos.x - enemy_pos.x;
                    let dir_y = player_pos.y - enemy_pos.y;
                    Self::apply_knockback(world, player_entity, dir_x, dir_y, PLAYER_KNOCKBACK);

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
    fn test_process_melee_announces_each_hit() {
        let mut world = World::new();
        for x in [30.0, 40.0] {
            let enemy = world.spawn();
            world.add_component(enemy, Enemy);
            world.add_component(enemy, Position::new(x, 0.0));
            world.add_component(enemy, Health::new(100));
        }
        CombatSystem::process_melee(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            50,
            50.0,
        );
        assert_eq!(
            world.drain_events(),
            vec![
                GameEvent::EnemyHit {
                    by: WeaponType::Melee
                };
                2
            ]
        );
    }

    #[test]
    fn test_enemy_attack_announces_player_hurt() {
        let mut world = World::new();
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));
        world.add_component(player, Health::new(100));
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(100));
        let mut ai = AI::new();
        ai.state = AIState::SurePlayerSeen;
        world.add_component(enemy, ai);

        let mut system = CombatSystem;
        system.run(&mut world, 0.016);
        assert_eq!(world.drain_events(), vec![GameEvent::PlayerHurt]);
        // Cooldown: no second hit, no second event.
        system.run(&mut world, 0.016);
        assert!(world.drain_events().is_empty());
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
        ai.state = AIState::SurePlayerSeen; // Changed from Attack to SurePlayerSeen
        world.add_component(enemy, ai);

        let mut system = CombatSystem;
        system.run(&mut world, 0.016);

        let player_health = world.get_component::<Health>(player).unwrap();
        assert_eq!(player_health.current, 100 - ENEMY_ATTACK_DAMAGE);
    }

    #[test]
    fn test_process_shoot_knocks_enemy_along_bullet() {
        let mut world = World::new();

        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(50.0, 0.0));
        world.add_component(enemy, Radius::new(10.0));
        world.add_component(enemy, Health::new(100));

        // Bullet travels straight along +x.
        CombatSystem::process_shoot(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            30,
        );

        let kb = world.get_component::<Knockback>(enemy).unwrap();
        // Shoved along +x (away from the shooter), no sideways component.
        assert!((kb.x - BULLET_KNOCKBACK).abs() < 0.001);
        assert!(kb.y.abs() < 0.001);
    }

    #[test]
    fn test_process_melee_knocks_enemy_away_from_attacker() {
        let mut world = World::new();

        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(100));

        CombatSystem::process_melee(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            50,
            50.0,
        );

        let kb = world.get_component::<Knockback>(enemy).unwrap();
        // Enemy sits on +x from the attacker, so it is shoved further along +x.
        assert!((kb.x - MELEE_KNOCKBACK).abs() < 0.001);
        assert!(kb.y.abs() < 0.001);
    }

    #[test]
    fn test_enemy_attack_knocks_player_away() {
        let mut world = World::new();

        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));
        world.add_component(player, Health::new(100));

        // Enemy to the player's +x side.
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(100));
        let mut ai = AI::new();
        ai.state = AIState::SurePlayerSeen;
        world.add_component(enemy, ai);

        let mut system = CombatSystem;
        system.run(&mut world, 0.016);

        let kb = world.get_component::<Knockback>(player).unwrap();
        // Player is shoved away from the enemy: toward -x.
        assert!((kb.x + PLAYER_KNOCKBACK).abs() < 0.001);
        assert!(kb.y.abs() < 0.001);
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
        ai.state = AIState::SurePlayerSeen;
        ai.reset_attack_timer(); // Cooldown active
        world.add_component(enemy, ai);

        let mut system = CombatSystem;
        system.run(&mut world, 0.016);

        let player_health = world.get_component::<Health>(player).unwrap();
        assert_eq!(player_health.current, 100); // No damage due to cooldown
    }
}
