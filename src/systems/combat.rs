use crate::components::{
    AIState, Boss, Enemy, GameEvent, Health, Knockback, Player, Position, Radius, Stunned,
    WeaponType, AI,
};
use crate::ecs::{Entity, System, World};

// The player dies to ANY connected enemy hit (one-hit death — the genre's
// core loop; death is cheap: checkpoint restore / hold-R restart). The old
// per-hit damage tiers are retired.

/// Impulse speed (px/s) a single bullet imparts to the enemy it strikes, thrown
/// along the bullet's travel direction. With the movement system's decay this is
/// a shove of a few dozen pixels — punchy, but nowhere near a room-crossing fling.
pub const BULLET_KNOCKBACK: f32 = 500.0;

/// Melee connects with the whole body, so it shoves the target noticeably harder
/// than a single bullet (matching the heavier feel of a swing).
pub const MELEE_KNOCKBACK: f32 = 1000.0;

/// Chip damage of a bare-fist strike — the KNOCKDOWN is the point, the damage
/// is a scratch (it still flips passive civilians hostile, like any hurt).
pub const PUNCH_DAMAGE: i32 = 10;

/// Reach of the player's armed melee swing (the metal bar), centre to centre.
/// Robots are drawn as 60 px tiles, so two sprites standing visibly adjacent
/// are ~60 px apart at the centres: the old 50 px reach forced the player to
/// physically overlap the target before a swing could connect (a whiff gives
/// no feedback beyond the whoosh), which soft-locked the floor-1 `strike`
/// tutorial gate in the browser. 70 px lands the swing from a natural
/// point-blank stance while still demanding contact range.
pub const MELEE_RANGE: f32 = 70.0;

/// Reach of a punch: a touch shorter than the metal bar's
/// [`MELEE_RANGE`] swing.
pub const PUNCH_RANGE: f32 = 65.0;

/// The shove of a punch: between a bullet and the metal bar.
pub const PUNCH_KNOCKBACK: f32 = 700.0;

/// How long a punched rogue stays sprawled on the floor before getting back
/// up (unless finished off first).
pub const KNOCKDOWN_SECS: f32 = 3.0;

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

    /// A killing blow landed on `entity` travelling along `(dx, dy)`
    /// (attacker -> victim / the projectile's flight): record the direction as
    /// the corpse's fall so the body sprawls along the shot, head pointing
    /// AWAY from the attacker — the same convention as [`Stunned::with_fall`]
    /// knockdowns (the render layer draws a downed body facing
    /// `fall_angle + PI` and the pose topples it onto its back, so the head
    /// ends up along `+fall_angle`). Reuses [`Stunned`]: on a corpse it only
    /// drives the fall animation and final orientation (the stun system's
    /// get-up writes the matching facing into `Rotation`, which the dead-robot
    /// draw keeps — a dead entity never stands back up). The boss keeps its
    /// own draw and is exempt, like it is from knockdowns.
    pub(crate) fn record_corpse_fall(world: &mut World, entity: Entity, dx: f32, dy: f32) {
        if world.has_component::<Boss>(entity) {
            return;
        }
        let fall = dy.atan2(dx);
        world.add_component(entity, Stunned::with_fall(KNOCKDOWN_SECS, fall));
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
                    let killed = health.is_dead();
                    // Shove the enemy along the bullet's travel direction
                    // (shooter -> target), i.e. away from the shooter.
                    let dir_x = target_pos.x - shooter_pos.x;
                    let dir_y = target_pos.y - shooter_pos.y;
                    Self::apply_knockback(world, enemy, dir_x, dir_y, BULLET_KNOCKBACK);
                    if killed {
                        Self::record_corpse_fall(world, enemy, dir_x, dir_y);
                    }
                    return true; // Hit confirmed
                }
            }
        }

        false // No hit
    }

    /// Is `enemy_pos` inside the 90-degree melee cone: within `range` of the
    /// attacker and within 45 degrees of the attacker->target direction?
    fn in_melee_cone(
        attacker_pos: Position,
        target_angle: f32,
        enemy_pos: Position,
        range: f32,
    ) -> bool {
        if attacker_pos.distance_to(&enemy_pos) > range {
            return false;
        }
        let enemy_dx = enemy_pos.x - attacker_pos.x;
        let enemy_dy = enemy_pos.y - attacker_pos.y;
        let enemy_angle = enemy_dy.atan2(enemy_dx);

        let angle_diff = (enemy_angle - target_angle).abs();
        let normalized_angle = if angle_diff > std::f32::consts::PI {
            2.0 * std::f32::consts::PI - angle_diff
        } else {
            angle_diff
        };
        normalized_angle < std::f32::consts::PI / 4.0
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

            if Self::in_melee_cone(attacker_pos, target_angle, enemy_pos, range) {
                let mut killed = false;
                if let Some(health) = world.get_component_mut::<Health>(enemy) {
                    health.take_damage(damage);
                    killed = health.is_dead();
                    hit_any = true;
                    world.push_event(GameEvent::EnemyHit {
                        by: WeaponType::Melee,
                    });
                    world.push_event(GameEvent::StrikeLanded);
                }
                // Shove the enemy away from the attacker (attacker -> enemy).
                let dir_x = enemy_pos.x - attacker_pos.x;
                let dir_y = enemy_pos.y - attacker_pos.y;
                Self::apply_knockback(world, enemy, dir_x, dir_y, MELEE_KNOCKBACK);
                if killed {
                    Self::record_corpse_fall(world, enemy, dir_x, dir_y);
                }
            }
        }

        hit_any
    }

    /// Process a bare-fist strike in the same 90-degree cone as melee, but
    /// Hotline-Miami style: instead of plain damage the enemy is KNOCKED DOWN —
    /// chip damage ([`PUNCH_DAMAGE`]), a shove, and a [`Stunned`] sprawl of
    /// [`KNOCKDOWN_SECS`] falling AWAY from the blow (attacker -> enemy). The
    /// boss shrugs the knockdown off (it only takes the chip damage). Emits one
    /// [`GameEvent::EnemyHit`] (by melee — a fist clanging on a metal bot) per
    /// enemy struck.
    pub fn process_punch(
        world: &mut World,
        attacker_pos: Position,
        target_pos: Position,
        damage: i32,
        range: f32,
    ) -> bool {
        let enemies: Vec<Entity> = world.query::<Enemy>();

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

            if enemy_health.is_dead() {
                continue;
            }

            if Self::in_melee_cone(attacker_pos, target_angle, enemy_pos, range) {
                if let Some(health) = world.get_component_mut::<Health>(enemy) {
                    health.take_damage(damage);
                    hit_any = true;
                    world.push_event(GameEvent::EnemyHit {
                        by: WeaponType::Melee,
                    });
                    world.push_event(GameEvent::PunchLanded);
                }
                let dir_x = enemy_pos.x - attacker_pos.x;
                let dir_y = enemy_pos.y - attacker_pos.y;
                Self::apply_knockback(world, enemy, dir_x, dir_y, PUNCH_KNOCKBACK);
                // Knock the bot down, sprawled along the direction of the blow.
                // The boss is immune to knockdown (and re-punching a downed bot
                // simply restarts its sprawl).
                if !world.has_component::<Boss>(enemy) {
                    let fall = dir_y.atan2(dir_x);
                    world.add_component(enemy, Stunned::with_fall(KNOCKDOWN_SECS, fall));
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
                    // ONE-HIT DEATH: any connected hit ends the run — the
                    // genre's whole loop (die instantly, R restarts in a
                    // heartbeat). Boss and rogue alike.
                    if let Some(health) = world.get_component_mut::<Health>(player_entity) {
                        health.take_damage(health.max.max(health.current));
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
                },
                GameEvent::StrikeLanded,
                GameEvent::EnemyHit {
                    by: WeaponType::Melee
                },
                GameEvent::StrikeLanded,
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
    fn test_combat_system_enemy_attack_is_lethal() {
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

        // ONE-HIT DEATH: a single connected hit ends the run.
        let player_health = world.get_component::<Health>(player).unwrap();
        assert!(player_health.is_dead());
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
