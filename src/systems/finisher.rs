//! Weapon-specific FINISHERS on downed enemies (Hotline-Miami ground kills).
//!
//! When the player clicks while standing over a DOWNED (stunned, alive) enemy,
//! [`FinisherSystem::try_start`] begins a finisher instead of a normal attack:
//! a [`Finisher`] component lands on the player, locking their movement and
//! fire for its duration (the input layer checks [`FinisherSystem::active`]).
//! The flavour depends on what the player holds ([`FinisherSystem::kind_for`]):
//!
//! * unarmed — POUND: drop onto the bot and hammer it, three quick hits;
//! * metal bar (or an empty gun swung like one) — OVERHEAD: one heavy blow;
//! * loaded gun — EXECUTE: one point-blank shot straight down (costs a round).
//!
//! The system then ticks the animation: at every scheduled impact the victim
//! is shoved and an [`GameEvent::EnemyHit`] rings out (the existing per-weapon
//! impact SFX); the FINAL impact is the kill — the victim dies at that moment,
//! not at the start. The victim's [`Stunned`] timer is topped up every frame so
//! it can never get back up mid-execution. The boss can never be downed (see
//! `combat::process_punch` / `boss::BossSystem`), and `try_start` skips it
//! outright, so it can never be finished either.

use crate::components::{
    Boss, Enemy, Finisher, FinisherKind, GameEvent, Health, Player, Position, Rotation, Stunned,
    Velocity, Weapon, WeaponType,
};
use crate::ecs::{Entity, System, World};
use crate::systems::combat::CombatSystem;

/// How close (px, centre to centre) the player must be to a downed enemy to
/// finish it: player radius 15 + enemy radius 12 + a hand's reach of slack —
/// "standing over / adjacent", well inside the 45-50 px strike ranges.
pub const FINISHER_RANGE: f32 = 60.0;

/// The shove the victim's body takes at each finisher impact (px/s): a solid
/// jolt that makes the sprawled sprite lurch, not a room-crossing fling.
pub const FINISHER_IMPACT_KNOCKBACK: f32 = 260.0;

/// Extra seconds of knockdown kept on the victim beyond the finisher's end,
/// so the kill always lands while the body is still down.
const PIN_MARGIN: f32 = 0.25;

/// System that runs the player's active finisher (see the module docs).
pub struct FinisherSystem;

impl FinisherSystem {
    /// The finisher flavour for what the player currently holds.
    pub fn kind_for(weapon: Option<&Weapon>) -> FinisherKind {
        match weapon {
            None => FinisherKind::Pound,
            Some(w) if w.weapon_type.is_melee() => FinisherKind::Overhead,
            Some(w) if w.ammo > 0 => FinisherKind::Execute(w.weapon_type),
            // An empty gun has nothing to fire point-blank: swing it instead.
            Some(_) => FinisherKind::Overhead,
        }
    }

    /// The nearest downed enemy in finisher range of `from`: alive, currently
    /// [`Stunned`], not the boss. `None` if there is none in range.
    pub fn downed_target(world: &World, from: Position) -> Option<Entity> {
        let mut best: Option<(Entity, f32)> = None;
        for enemy in world.query::<Enemy>() {
            if world.has_component::<Boss>(enemy) {
                continue; // the shoggoth cannot be finished
            }
            let stunned = world
                .get_component::<Stunned>(enemy)
                .map(|s| s.is_active())
                .unwrap_or(false);
            if !stunned {
                continue;
            }
            let alive = world
                .get_component::<Health>(enemy)
                .map(|h| h.is_alive())
                .unwrap_or(false);
            if !alive {
                continue;
            }
            let pos = match world.get_component::<Position>(enemy) {
                Some(p) => *p,
                None => continue,
            };
            let d = from.distance_to(&pos);
            if d <= FINISHER_RANGE && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((enemy, d));
            }
        }
        best.map(|(e, _)| e)
    }

    /// Start a finisher if a downed enemy is in range: the player turns to the
    /// victim and is locked into the animation. Returns whether one started —
    /// `false` means the click should behave exactly as a normal attack.
    pub fn try_start(world: &mut World) -> bool {
        let player = match world.query::<Player>().first() {
            Some(&p) => p,
            None => return false,
        };
        if world.has_component::<Finisher>(player) {
            return false; // already mid-finisher
        }
        let player_pos = match world.get_component::<Position>(player) {
            Some(p) => *p,
            None => return false,
        };
        let victim = match Self::downed_target(world, player_pos) {
            Some(v) => v,
            None => return false,
        };
        let victim_pos = match world.get_component::<Position>(victim) {
            Some(p) => *p,
            None => return false,
        };

        let dx = victim_pos.x - player_pos.x;
        let dy = victim_pos.y - player_pos.y;
        let len = (dx * dx + dy * dy).sqrt();
        let (dir_x, dir_y) = if len > f32::EPSILON {
            (dx / len, dy / len)
        } else {
            (1.0, 0.0)
        };

        let kind = Self::kind_for(world.get_component::<Weapon>(player));

        // Face the victim and stop dead for the whole animation.
        if let Some(rot) = world.get_component_mut::<Rotation>(player) {
            rot.angle = dir_y.atan2(dir_x);
        }
        if let Some(vel) = world.get_component_mut::<Velocity>(player) {
            vel.x = 0.0;
            vel.y = 0.0;
        }
        // The bar (or the empty gun used as one) goes up with a swing whoosh;
        // pound and execute make their noise at the impact itself.
        if kind == FinisherKind::Overhead {
            world.push_event(GameEvent::PlayerFired(WeaponType::Melee));
        }

        world.add_component(
            player,
            Finisher {
                target: victim,
                kind,
                timer: 0.0,
                dir_x,
                dir_y,
                hits_done: 0,
            },
        );
        true
    }

    /// Is a finisher currently running (the input layer locks the player out
    /// of movement / fire / throw / pickup while it is)?
    pub fn active(world: &World) -> bool {
        !world.query::<Finisher>().is_empty()
    }

    /// One scheduled blow lands on the victim. `kill` marks the final impact.
    fn impact(world: &mut World, player: Entity, fin: &Finisher, kill: bool) {
        // The body lurches with the blow.
        CombatSystem::apply_knockback(
            world,
            fin.target,
            fin.dir_x,
            fin.dir_y,
            FINISHER_IMPACT_KNOCKBACK,
        );
        match fin.kind {
            // A fist / bar slamming a metal bot: the melee impact clank.
            FinisherKind::Pound | FinisherKind::Overhead => {
                world.push_event(GameEvent::EnemyHit {
                    by: WeaponType::Melee,
                });
            }
            // The point-blank shot: gunshot + that gun's impact, one round gone.
            FinisherKind::Execute(gun) => {
                if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
                    weapon.ammo = (weapon.ammo - 1).max(0);
                    weapon.fire_timer = weapon.fire_rate;
                }
                world.push_event(GameEvent::PlayerFired(gun));
                world.push_event(GameEvent::EnemyHit { by: gun });
            }
        }
        if kill {
            if let Some(health) = world.get_component_mut::<Health>(fin.target) {
                health.take_damage(health.max.max(health.current));
            }
        }
    }
}

impl System for FinisherSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        for player in world.query::<Finisher>() {
            let mut fin = match world.get_component::<Finisher>(player) {
                Some(f) => *f,
                None => continue,
            };
            let impacts = fin.kind.impacts();

            // If the victim vanished or was killed by something else before
            // our own killing blow, the execution has nothing left to do.
            let victim_alive = world
                .get_component::<Health>(fin.target)
                .map(|h| h.is_alive())
                .unwrap_or(false);
            if !victim_alive && fin.hits_done < impacts.len() {
                world.remove_component::<Finisher>(player);
                continue;
            }

            // The player is locked in the animation.
            if let Some(vel) = world.get_component_mut::<Velocity>(player) {
                vel.x = 0.0;
                vel.y = 0.0;
            }

            // The victim can never get back up mid-execution: top its
            // knockdown up to outlast the animation. (Runs before StunSystem
            // in the frame, so the timer never expires under us.)
            let remaining = fin.kind.duration() - fin.timer;
            if let Some(stun) = world.get_component_mut::<Stunned>(fin.target) {
                stun.timer = stun.timer.max(remaining + PIN_MARGIN);
                stun.duration = stun.duration.max(stun.timer);
            }

            fin.timer += dt;
            while fin.hits_done < impacts.len() && fin.timer >= impacts[fin.hits_done] {
                fin.hits_done += 1;
                let kill = fin.hits_done == impacts.len();
                Self::impact(world, player, &fin, kill);
            }

            if fin.timer >= fin.kind.duration() {
                world.remove_component::<Finisher>(player);
                // The execution ran to its end (the abort path above never
                // reaches here): announce it for the tutorial `finish` gate.
                world.push_event(GameEvent::FinisherDone);
            } else if let Some(slot) = world.get_component_mut::<Finisher>(player) {
                *slot = fin;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Radius;
    use crate::systems::combat::{CombatSystem, KNOCKDOWN_SECS, PUNCH_DAMAGE, PUNCH_RANGE};
    use crate::systems::StunSystem;

    fn spawn_player_at(world: &mut World, x: f32, y: f32, weapon: Option<Weapon>) -> Entity {
        let p = world.spawn();
        world.add_component(p, Player);
        world.add_component(p, Position::new(x, y));
        world.add_component(p, Velocity::new(0.0, 0.0));
        world.add_component(p, Rotation::new(0.0));
        world.add_component(p, Radius::new(15.0));
        if let Some(w) = weapon {
            world.add_component(p, w);
        }
        p
    }

    fn spawn_downed_enemy(world: &mut World, x: f32, y: f32) -> Entity {
        let e = world.spawn();
        world.add_component(e, Enemy);
        world.add_component(e, Position::new(x, y));
        world.add_component(e, Rotation::new(0.0));
        world.add_component(e, Health::new(50));
        world.add_component(e, Stunned::with_fall(3.0, 0.0));
        e
    }

    /// Run the finisher + stun systems interleaved (the frame order) for `secs`.
    fn run_secs(world: &mut World, secs: f32) {
        let mut finisher = FinisherSystem;
        let mut stun = StunSystem;
        let dt = 0.016;
        let steps = (secs / dt).ceil() as usize;
        for _ in 0..steps {
            finisher.run(world, dt);
            stun.run(world, dt);
        }
    }

    #[test]
    fn test_unarmed_punch_downs_but_does_not_kill() {
        let mut world = World::new();
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(50));

        let hit = CombatSystem::process_punch(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            PUNCH_DAMAGE,
            PUNCH_RANGE,
        );

        assert!(hit);
        let health = world.get_component::<Health>(enemy).unwrap();
        assert!(health.is_alive());
        assert_eq!(health.current, 50 - PUNCH_DAMAGE);
        // Knocked down, sprawled along the blow (+x here).
        let stun = world.get_component::<Stunned>(enemy).unwrap();
        assert_eq!(stun.timer, KNOCKDOWN_SECS);
        assert!(stun.fall_angle.abs() < 0.001);
        assert_eq!(
            world.drain_events(),
            vec![
                GameEvent::EnemyHit {
                    by: WeaponType::Melee
                },
                GameEvent::PunchLanded,
            ]
        );
    }

    #[test]
    fn test_punched_enemy_gets_back_up_after_timeout() {
        let mut world = World::new();
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(50));
        world.add_component(enemy, Rotation::new(0.0));

        CombatSystem::process_punch(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            PUNCH_DAMAGE,
            PUNCH_RANGE,
        );
        assert!(world.has_component::<Stunned>(enemy));

        // No finisher: the knockdown runs out and the bot gets back up...
        run_secs(&mut world, KNOCKDOWN_SECS + 0.1);
        assert!(!world.has_component::<Stunned>(enemy));
        // ...facing back along its fall (toward the attacker at -x).
        let rot = world.get_component::<Rotation>(enemy).unwrap();
        assert!((rot.angle - std::f32::consts::PI).abs() < 0.001);
    }

    #[test]
    fn test_punch_does_not_knock_down_boss() {
        let mut world = World::new();
        let boss = world.spawn();
        world.add_component(boss, Enemy);
        world.add_component(boss, Boss::new());
        world.add_component(boss, Position::new(30.0, 0.0));
        world.add_component(boss, Health::new(500));

        CombatSystem::process_punch(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            PUNCH_DAMAGE,
            PUNCH_RANGE,
        );

        // Chip damage lands, but the shoggoth never goes down.
        assert_eq!(
            world.get_component::<Health>(boss).unwrap().current,
            500 - PUNCH_DAMAGE
        );
        assert!(!world.has_component::<Stunned>(boss));
    }

    #[test]
    fn test_armed_melee_stays_lethal_and_never_knocks_down() {
        let mut world = World::new();
        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(30.0, 0.0));
        world.add_component(enemy, Health::new(50));

        // The metal bar's damage (100) kills a 50 hp rogue outright.
        let bar = Weapon::new(WeaponType::Melee);
        CombatSystem::process_melee(
            &mut world,
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            bar.damage,
            50.0,
        );

        assert!(world.get_component::<Health>(enemy).unwrap().is_dead());
        assert!(!world.has_component::<Stunned>(enemy));
    }

    #[test]
    fn test_finisher_only_starts_in_range() {
        let mut world = World::new();
        spawn_player_at(&mut world, 0.0, 0.0, None);
        // Downed but too far away: no finisher, the click stays a normal attack.
        spawn_downed_enemy(&mut world, FINISHER_RANGE + 20.0, 0.0);
        assert!(!FinisherSystem::try_start(&mut world));

        // One in reach: the finisher starts.
        spawn_downed_enemy(&mut world, 30.0, 0.0);
        assert!(FinisherSystem::try_start(&mut world));
    }

    #[test]
    fn test_finisher_requires_a_downed_target() {
        let mut world = World::new();
        spawn_player_at(&mut world, 0.0, 0.0, None);
        // A standing enemy in reach is NOT a finisher target.
        let e = world.spawn();
        world.add_component(e, Enemy);
        world.add_component(e, Position::new(30.0, 0.0));
        world.add_component(e, Health::new(50));
        assert!(!FinisherSystem::try_start(&mut world));
    }

    #[test]
    fn test_finisher_never_targets_the_boss() {
        let mut world = World::new();
        spawn_player_at(&mut world, 0.0, 0.0, None);
        let boss = world.spawn();
        world.add_component(boss, Enemy);
        world.add_component(boss, Boss::new());
        world.add_component(boss, Position::new(30.0, 0.0));
        world.add_component(boss, Health::new(500));
        // Even with a Stunned somehow present, the boss is skipped.
        world.add_component(boss, Stunned::new(3.0));
        assert!(!FinisherSystem::try_start(&mut world));
    }

    #[test]
    fn test_pound_finisher_kills_at_the_last_hit() {
        let mut world = World::new();
        let player = spawn_player_at(&mut world, 0.0, 0.0, None); // unarmed
        let victim = spawn_downed_enemy(&mut world, 30.0, 0.0);

        assert!(FinisherSystem::try_start(&mut world));
        assert_eq!(
            world.get_component::<Finisher>(player).unwrap().kind,
            FinisherKind::Pound
        );

        // Halfway through: two pounds in, the victim is hurt but NOT dead yet.
        run_secs(&mut world, 0.5);
        assert!(world.get_component::<Health>(victim).unwrap().is_alive());
        assert!(world.has_component::<Finisher>(player));

        // Past the last hit: dead, and the player is released.
        run_secs(&mut world, 0.3);
        assert!(world.get_component::<Health>(victim).unwrap().is_dead());
        assert!(!world.has_component::<Finisher>(player));

        // Three pound clanks rang out.
        let clanks = world
            .drain_events()
            .into_iter()
            .filter(|e| {
                *e == GameEvent::EnemyHit {
                    by: WeaponType::Melee,
                }
            })
            .count();
        assert_eq!(clanks, 3);
    }

    #[test]
    fn test_victim_stays_pinned_through_the_finisher() {
        let mut world = World::new();
        let player = spawn_player_at(&mut world, 0.0, 0.0, None);
        let victim = spawn_downed_enemy(&mut world, 30.0, 0.0);
        // The knockdown is almost over when the finisher starts...
        world.get_component_mut::<Stunned>(victim).unwrap().timer = 0.05;

        assert!(FinisherSystem::try_start(&mut world));
        // ...but the victim never gets back up before dying.
        run_secs(&mut world, 0.5);
        assert!(world.has_component::<Stunned>(victim));
        assert!(world.get_component::<Health>(victim).unwrap().is_alive());
        run_secs(&mut world, 0.3);
        assert!(world.get_component::<Health>(victim).unwrap().is_dead());
        let _ = player;
    }

    #[test]
    fn test_overhead_finisher_with_the_bar() {
        let mut world = World::new();
        let player = spawn_player_at(&mut world, 0.0, 0.0, Some(Weapon::new(WeaponType::Melee)));
        let victim = spawn_downed_enemy(&mut world, 30.0, 0.0);

        assert!(FinisherSystem::try_start(&mut world));
        assert_eq!(
            world.get_component::<Finisher>(player).unwrap().kind,
            FinisherKind::Overhead
        );

        // Before the swing lands (0.35 s) the victim is still alive.
        run_secs(&mut world, 0.25);
        assert!(world.get_component::<Health>(victim).unwrap().is_alive());
        run_secs(&mut world, 0.35);
        assert!(world.get_component::<Health>(victim).unwrap().is_dead());
        assert!(!world.has_component::<Finisher>(player));
    }

    #[test]
    fn test_gun_finisher_costs_one_round() {
        let mut world = World::new();
        let player = spawn_player_at(&mut world, 0.0, 0.0, Some(Weapon::new(WeaponType::Pistol)));
        let victim = spawn_downed_enemy(&mut world, 30.0, 0.0);

        assert!(FinisherSystem::try_start(&mut world));
        assert_eq!(
            world.get_component::<Finisher>(player).unwrap().kind,
            FinisherKind::Execute(WeaponType::Pistol)
        );

        run_secs(&mut world, 0.5);
        assert!(world.get_component::<Health>(victim).unwrap().is_dead());
        assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 11);
        // The shot and its point-blank impact were announced.
        let events = world.drain_events();
        assert!(events.contains(&GameEvent::PlayerFired(WeaponType::Pistol)));
        assert!(events.contains(&GameEvent::EnemyHit {
            by: WeaponType::Pistol
        }));
    }

    #[test]
    fn test_empty_gun_pistol_whips_instead_of_firing() {
        let mut world = World::new();
        let empty = Weapon::with_ammo(WeaponType::Shotgun, 0);
        let player = spawn_player_at(&mut world, 0.0, 0.0, Some(empty));
        let victim = spawn_downed_enemy(&mut world, 30.0, 0.0);

        assert!(FinisherSystem::try_start(&mut world));
        assert_eq!(
            world.get_component::<Finisher>(player).unwrap().kind,
            FinisherKind::Overhead
        );
        run_secs(&mut world, 0.6);
        assert!(world.get_component::<Health>(victim).unwrap().is_dead());
        // No round appeared from nowhere.
        assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 0);
    }

    #[test]
    fn test_finisher_picks_the_nearest_downed_enemy() {
        let mut world = World::new();
        let player = spawn_player_at(&mut world, 0.0, 0.0, None);
        let far = spawn_downed_enemy(&mut world, 50.0, 0.0);
        let near = spawn_downed_enemy(&mut world, -25.0, 0.0);

        assert!(FinisherSystem::try_start(&mut world));
        assert_eq!(
            world.get_component::<Finisher>(player).unwrap().target,
            near
        );
        let _ = far;
    }

    #[test]
    fn test_finisher_aborts_if_victim_dies_early() {
        let mut world = World::new();
        let player = spawn_player_at(&mut world, 0.0, 0.0, None);
        let victim = spawn_downed_enemy(&mut world, 30.0, 0.0);

        assert!(FinisherSystem::try_start(&mut world));
        // Something else (a stray shotgun blast) kills the victim first.
        world
            .get_component_mut::<Health>(victim)
            .unwrap()
            .take_damage(999);
        run_secs(&mut world, 0.1);
        assert!(!world.has_component::<Finisher>(player));
    }
}
