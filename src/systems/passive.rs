//! Passive (civilian) bots — the cold-open crowd.
//!
//! A `type: "passive"` spawn is a bot in [`AIState::Passive`]: it has no
//! vision cone, never aggroes and never attacks. If it was given a `walk_to`
//! zone it strolls there (pathfinding, at [`PASSIVE_SPEED_FACTOR`] of a
//! rogue's chase speed) to a random point inside the zone, then stands and
//! fidgets, settling on its `face` heading; without a zone it wanders gently
//! around its spawn point. It collides, gets knocked back and takes damage
//! like any rogue, and it counts as a rogue for `kills` / `all_dead`.
//!
//! The crowd turns hostile in two ways: the scenario `alert` action
//! ([`alert_passives`]) — all of them, the ones inside a zone, or a spawn
//! `group` — or any passive taking damage, which flips every passive on the
//! floor (the player started it). A flipped bot becomes a hostile of its
//! `look` type (`kind`) that already knows where the player is, and arms
//! itself with that type's weapon.
//!
//! The per-tick brain lives here ([`update_passive`]) and is called by
//! [`crate::systems::AISystem`] for every enemy whose state is `Passive`, so
//! no extra system needs wiring into the frame loop or the headless
//! `Simulation`.

use std::f32::consts::PI;

use crate::components::{
    AIState, Enemy, Health, PassiveAI, Player, Position, Radius, Rotation, Speed, Stunned,
    Velocity, Weapon, Zone, AI,
};
use crate::ecs::world::Wall;
use crate::ecs::{Entity, World};
use crate::game::weapon_for_enemy;
use crate::math::Vec2;
use crate::pathfinding::NavigationGrid;
use crate::scenario::{AlertTarget, SpawnDef};

/// A strolling passive moves at this fraction of a rogue's chase speed.
pub const PASSIVE_SPEED_FACTOR: f32 = 0.55;
/// A wandering (no `walk_to`) passive drifts at this fraction of chase speed.
pub const PASSIVE_WANDER_SPEED_FACTOR: f32 = 0.35;
/// How far a zone-less passive drifts from its spawn (px, per axis).
pub const PASSIVE_WANDER_LEASH: f32 = 90.0;
/// A passive is "there" once this close to its picked point in the zone.
pub const PASSIVE_ARRIVE_DIST: f32 = 6.0;
/// Inset (px) kept from the zone's edges when picking a point inside it.
const ZONE_INSET: f32 = 14.0;
/// Walls are inflated by this much when deciding whether to walk straight or
/// pathfind (same padding the chase AI uses).
const WALL_PADDING: f32 = 25.0;
/// Rotation easing rate (per second) toward the fidget heading.
const TURN_RATE: f32 = 6.0;
/// Idle fidget: max deviation (radians) from the settled heading.
const FIDGET_SWING: f32 = 0.45;

fn next_random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

fn random_range(state: &mut u32, min: f32, max: f32) -> f32 {
    let r = next_random(state) as f32 / u32::MAX as f32;
    r * (max - min) + min
}

/// Wrap an angle difference into `-PI..=PI`.
fn wrap_angle(mut a: f32) -> f32 {
    while a > PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// Spawn a passive bot from its placement: an `Enemy` (so it collides,
/// takes damage and counts) in [`AIState::Passive`], unarmed.
pub fn spawn_passive(world: &mut World, def: &SpawnDef) -> Entity {
    let entity = world.spawn();
    let pos = Position::new(def.x, def.y);
    let face = def.face.map(f32::to_radians);

    world.add_component(entity, Enemy);
    world.add_component(entity, pos);
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(100.0));
    world.add_component(entity, Health::new(50));
    world.add_component(entity, Radius::new(12.0));
    world.add_component(entity, Rotation::new(face.unwrap_or(-PI / 2.0)));

    let mut ai = AI::new_with_type(def.kind, pos);
    ai.state = AIState::Passive;
    let mut brief = PassiveAI::new(def.walk_to, face, def.group);
    brief.last_health = 50;
    brief.fidget_heading = face.unwrap_or(-PI / 2.0);
    ai.passive = Some(brief);
    world.add_component(entity, ai);
    entity
}

/// Whether the entity is a live passive bot that has not been flipped yet.
pub fn is_passive(world: &World, entity: Entity) -> bool {
    world
        .get_component::<AI>(entity)
        .is_some_and(|ai| ai.state == AIState::Passive)
        && world
            .get_component::<Health>(entity)
            .is_some_and(|h| h.is_alive())
}

/// Number of live, un-flipped passives on the floor.
pub fn count_passives(world: &World) -> usize {
    world
        .query::<Enemy>()
        .into_iter()
        .filter(|&e| is_passive(world, e))
        .count()
}

/// Flip the passives matching `target` hostile toward the player. Returns
/// how many were flipped.
pub fn alert_passives(world: &mut World, target: AlertTarget) -> usize {
    let player_pos = world
        .query::<Player>()
        .first()
        .and_then(|&p| world.get_component::<Position>(p))
        .copied();
    let zone_rect = match target {
        AlertTarget::Zone(id) => world.query::<Zone>().into_iter().find_map(|e| {
            world
                .get_component::<Zone>(e)
                .filter(|z| z.id == id)
                .copied()
        }),
        _ => None,
    };

    let mut flipped = 0;
    for entity in world.query::<Enemy>() {
        let (ai, pos) = match (
            world.get_component::<AI>(entity),
            world.get_component::<Position>(entity),
        ) {
            (Some(ai), Some(pos)) => (ai, *pos),
            _ => continue,
        };
        if ai.state != AIState::Passive {
            continue;
        }
        // A downed civilian stays down (but its brief no longer matters).
        if world
            .get_component::<Health>(entity)
            .is_some_and(|h| h.is_dead())
        {
            continue;
        }
        let matches = match target {
            AlertTarget::All => true,
            AlertTarget::Zone(_) => zone_rect.is_some_and(|z| z.contains(pos.to_vec2())),
            AlertTarget::Group(g) => ai.passive.is_some_and(|p| p.group == Some(g)),
        };
        if !matches {
            continue;
        }
        make_hostile(world, entity, player_pos);
        flipped += 1;
    }
    flipped
}

/// Turn one passive into a hostile rogue of its `kind` that already knows
/// where the player is (and arms it with that type's weapon).
pub fn make_hostile(world: &mut World, entity: Entity, player_pos: Option<Position>) {
    let kind = match world.get_component_mut::<AI>(entity) {
        Some(ai) => {
            ai.state = AIState::SurePlayerSeen;
            ai.last_known_player_position = player_pos;
            ai.check_position = None;
            ai.state_timer = ai.lost_player_duration;
            // A drifter-look civilian turns into a feral: arm its lunge.
            if ai.initial_type == crate::components::EnemyType::Wandering {
                ai.wander_state = crate::components::WanderState::Waiting;
                ai.wander_timer = 0.0;
            }
            ai.initial_type
        }
        None => return,
    };
    if !world.has_component::<Weapon>(entity) {
        world.add_component(entity, Weapon::new(weapon_for_enemy(kind)));
    }
}

/// One tick of a passive bot's brain: stroll toward its zone / drift / fidget,
/// writing `Velocity` and `Rotation`. Returns `true` when the bot took damage
/// since the previous tick (the caller then alerts the whole floor).
///
/// `rng` is the world RNG state threaded through the AI update; `nav_grid`
/// and `walls` are the AI system's cached level geometry.
pub fn update_passive(
    world: &mut World,
    entity: Entity,
    rng: &mut u32,
    nav_grid: &NavigationGrid,
    walls: &[Wall],
    dt: f32,
) -> bool {
    let (pos, speed, health, rot) = match (
        world.get_component::<Position>(entity),
        world.get_component::<Speed>(entity),
        world.get_component::<Health>(entity),
        world.get_component::<Rotation>(entity),
    ) {
        (Some(p), Some(s), Some(h), Some(r)) => (*p, s.value, *h, r.angle),
        _ => return false,
    };
    let mut ai = match world.get_component::<AI>(entity) {
        Some(ai) => *ai,
        None => return false,
    };
    let mut brief = ai
        .passive
        .unwrap_or_else(|| PassiveAI::new(None, None, None));

    // Damage since last tick? (Knockback alone does not count.) A bot downed
    // outright still reports it once — the crowd turns on a kill too.
    let hurt = health.current < brief.last_health;
    brief.last_health = health.current;
    if health.is_dead() || world.has_component::<Stunned>(entity) {
        // Down or knocked over: no brain, no motion; keep the brief current.
        ai.passive = Some(brief);
        if let Some(slot) = world.get_component_mut::<AI>(entity) {
            *slot = ai;
        }
        if let Some(v) = world.get_component_mut::<Velocity>(entity) {
            v.x = 0.0;
            v.y = 0.0;
        }
        return hurt;
    }

    let mut vx = 0.0;
    let mut vy = 0.0;
    let mut goal_heading = brief.fidget_heading;

    if let (Some(zone_id), false) = (brief.walk_to, brief.arrived) {
        // Stroll to a random point inside the zone.
        if brief.target.is_none() {
            brief.target = pick_point_in_zone(world, zone_id, rng);
        }
        match brief.target {
            Some(target) => {
                let here = pos.to_vec2();
                let goal = target.to_vec2();
                if here.distance(goal) <= PASSIVE_ARRIVE_DIST {
                    brief.arrived = true;
                    brief.target = None;
                    brief.fidget_heading = brief.face.unwrap_or(rot);
                    brief.fidget_timer = random_range(rng, 1.0, 3.0);
                    goal_heading = brief.fidget_heading;
                } else {
                    let clear = crate::collision::has_line_of_sight_with_padding(
                        here,
                        goal,
                        walls,
                        WALL_PADDING,
                    );
                    // The stroll target is a fixed point in the zone, so the
                    // cached path is recomputed even less often than a
                    // chaser's (see `ai::PASSIVE_REPATH_INTERVAL`).
                    let waypoint = if clear {
                        crate::systems::ai::clear_path_cache(world, entity);
                        goal
                    } else {
                        crate::systems::ai::throttled_path_target(
                            world,
                            entity,
                            nav_grid,
                            here,
                            goal,
                            crate::systems::ai::PASSIVE_REPATH_INTERVAL,
                            dt,
                        )
                    };
                    let d = waypoint - here;
                    let len = d.length();
                    if len > 0.0 {
                        let s = speed * PASSIVE_SPEED_FACTOR;
                        vx = d.x / len * s;
                        vy = d.y / len * s;
                        goal_heading = d.y.atan2(d.x);
                        brief.fidget_heading = goal_heading;
                    }
                }
            }
            None => {
                // Unknown zone: behave like a zone-less passive.
                brief.arrived = true;
                brief.fidget_heading = brief.face.unwrap_or(rot);
            }
        }
    } else if brief.walk_to.is_some() {
        // Arrived: stand there and fidget (small turns around `face`).
        brief.fidget_timer -= dt;
        if brief.fidget_timer <= 0.0 {
            let base = brief.face.unwrap_or(brief.fidget_heading);
            brief.fidget_heading = base + random_range(rng, -FIDGET_SWING, FIDGET_SWING);
            brief.fidget_timer = random_range(rng, 1.2, 3.5);
        }
        goal_heading = brief.fidget_heading;
    } else {
        // No zone: gentle drift around the spawn point — short slow steps
        // with pauses, turning a little between them.
        brief.fidget_timer -= dt;
        if brief.fidget_timer <= 0.0 {
            if brief.target.is_some() {
                // End of a step: pause.
                brief.target = None;
                brief.fidget_timer = random_range(rng, 1.0, 2.5);
            } else {
                // Start a step toward a nearby point inside the leash.
                let sp = ai.spawn_position;
                let leash = PASSIVE_WANDER_LEASH;
                let tx = (pos.x + random_range(rng, -60.0, 60.0)).clamp(sp.x - leash, sp.x + leash);
                let ty = (pos.y + random_range(rng, -60.0, 60.0)).clamp(sp.y - leash, sp.y + leash);
                brief.target = Some(Position::new(tx, ty));
                brief.fidget_timer = random_range(rng, 0.8, 1.6);
            }
        }
        if let Some(t) = brief.target {
            let d = t.to_vec2() - pos.to_vec2();
            let len = d.length();
            // Stop early at the point or if the next step would touch a wall.
            let ahead = Vec2::new(
                pos.x + d.x / len.max(1.0) * 14.0,
                pos.y + d.y / len.max(1.0) * 14.0,
            );
            let blocked = walls.iter().any(|w| {
                ahead.x >= w.x
                    && ahead.x <= w.x + w.width
                    && ahead.y >= w.y
                    && ahead.y <= w.y + w.height
            });
            if len > 4.0 && !blocked {
                let s = speed * PASSIVE_WANDER_SPEED_FACTOR;
                vx = d.x / len * s;
                vy = d.y / len * s;
                goal_heading = d.y.atan2(d.x);
                brief.fidget_heading = goal_heading;
            } else {
                brief.target = None;
                brief.fidget_timer = random_range(rng, 1.0, 2.5);
            }
        }
    }

    // Ease the heading toward the goal (a stroll, not a snap).
    let k = (dt * TURN_RATE).clamp(0.0, 1.0);
    let new_rot = rot + wrap_angle(goal_heading - rot) * k;

    ai.passive = Some(brief);
    if let Some(slot) = world.get_component_mut::<AI>(entity) {
        *slot = ai;
    }
    if let Some(v) = world.get_component_mut::<Velocity>(entity) {
        v.x = vx;
        v.y = vy;
    }
    if let Some(r) = world.get_component_mut::<Rotation>(entity) {
        r.angle = new_rot;
    }
    hurt
}

/// A random point inside the zone with that id (inset from its edges), or
/// `None` if the floor has no such zone.
fn pick_point_in_zone(world: &World, zone_id: &str, rng: &mut u32) -> Option<Position> {
    let z = world.query::<Zone>().into_iter().find_map(|e| {
        world
            .get_component::<Zone>(e)
            .filter(|z| z.id == zone_id)
            .copied()
    })?;
    let inset_x = ZONE_INSET.min(z.w / 2.0);
    let inset_y = ZONE_INSET.min(z.h / 2.0);
    let x = random_range(rng, z.x + inset_x, z.x + z.w - inset_x);
    let y = random_range(rng, z.y + inset_y, z.y + z.h - inset_y);
    Some(Position::new(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::EnemyType;
    use crate::game::spawn_player;
    use crate::scenario::{Rect, ZoneDef};
    use crate::sim::Simulation;

    const ZONE: ZoneDef = ZoneDef {
        id: "doors",
        rect: Rect::new(400.0, 100.0, 200.0, 80.0),
    };

    fn crowd_world(walk_to: Option<&'static str>) -> World {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(500.0, 700.0));
        let z = world.spawn();
        world.add_component(z, Zone::from_def(&ZONE));
        let a = SpawnDef {
            x: 300.0,
            y: 500.0,
            kind: EnemyType::Wandering,
            passive: true,
            walk_to,
            face: Some(-90.0),
            group: Some("crowd"),
            unarmed: false,
        };
        let b = SpawnDef {
            x: 700.0,
            y: 500.0,
            kind: EnemyType::Idle,
            passive: true,
            walk_to,
            face: None,
            group: None,
            unarmed: false,
        };
        spawn_passive(&mut world, &a);
        spawn_passive(&mut world, &b);
        world
    }

    fn enemy_positions(world: &World) -> Vec<Vec2> {
        world
            .query::<Enemy>()
            .into_iter()
            .filter_map(|e| world.get_component::<Position>(e).map(|p| p.to_vec2()))
            .collect()
    }

    #[test]
    fn passives_never_aggro_or_attack() {
        // The player stands in plain view, in front of both bots, for 20 s.
        let mut sim = Simulation::from_world(crowd_world(None));
        let hp0 = crate::game::get_player_health(&sim.world);
        sim.run_frames(1200, 1.0 / 60.0);
        assert_eq!(count_passives(&sim.world), 2, "still passive after 20 s");
        assert_eq!(
            crate::game::get_player_health(&sim.world),
            hp0,
            "passives never hurt the player"
        );
        for e in sim.world.query::<Enemy>() {
            let ai = sim.world.get_component::<AI>(e).unwrap();
            assert_eq!(ai.state, AIState::Passive);
            assert!(
                !sim.world.has_component::<Weapon>(e),
                "civilians are unarmed"
            );
        }
    }

    #[test]
    fn zoneless_passives_drift_but_stay_leashed() {
        let mut sim = Simulation::from_world(crowd_world(None));
        let start = enemy_positions(&sim.world);
        let mut moved = false;
        for _ in 0..1800 {
            sim.step(1.0 / 60.0);
            let now = enemy_positions(&sim.world);
            for (s, n) in start.iter().zip(&now) {
                assert!(
                    (n.x - s.x).abs() <= PASSIVE_WANDER_LEASH + 20.0
                        && (n.y - s.y).abs() <= PASSIVE_WANDER_LEASH + 20.0,
                    "drifted past the leash: {s:?} -> {n:?}"
                );
                if s.distance(*n) > 10.0 {
                    moved = true;
                }
            }
        }
        assert!(moved, "a zone-less passive should drift a little");
    }

    #[test]
    fn passives_walk_to_their_zone_and_settle() {
        let mut sim = Simulation::from_world(crowd_world(Some("doors")));
        sim.run_frames(1500, 1.0 / 60.0); // 25 s: 400+ px at 55 px/s
        let zone = ZONE.rect;
        for e in sim.world.query::<Enemy>() {
            let p = sim.world.get_component::<Position>(e).unwrap().to_vec2();
            assert!(
                zone.contains(p),
                "passive should be inside its zone, at {p:?}"
            );
            let ai = sim.world.get_component::<AI>(e).unwrap();
            assert!(ai.passive.unwrap().arrived);
            let v = sim.world.get_component::<Velocity>(e).unwrap();
            assert!(
                v.x.abs() < 1e-3 && v.y.abs() < 1e-3,
                "settled bots stand still"
            );
        }
        // The one with `face: -90` settles looking up (within the fidget swing).
        let facing = sim
            .world
            .query::<Enemy>()
            .into_iter()
            .find(|&e| {
                sim.world
                    .get_component::<AI>(e)
                    .and_then(|ai| ai.passive)
                    .and_then(|p| p.group)
                    == Some("crowd")
            })
            .unwrap();
        let rot = sim.world.get_component::<Rotation>(facing).unwrap().angle;
        assert!(
            wrap_angle(rot + PI / 2.0).abs() < FIDGET_SWING + 0.2,
            "faces up, got {rot}"
        );
    }

    #[test]
    fn stroll_speed_is_a_fraction_of_chase_speed() {
        let mut sim = Simulation::from_world(crowd_world(Some("doors")));
        let start = enemy_positions(&sim.world);
        sim.run_frames(60, 1.0 / 60.0);
        let now = enemy_positions(&sim.world);
        for (s, n) in start.iter().zip(&now) {
            let d = s.distance(*n);
            assert!(d > 40.0 && d < 70.0, "~55 px in 1 s, got {d}");
        }
    }

    #[test]
    fn alert_all_flips_every_passive() {
        let mut world = crowd_world(None);
        assert_eq!(alert_passives(&mut world, AlertTarget::All), 2);
        assert_eq!(count_passives(&world), 0);
        for e in world.query::<Enemy>() {
            let ai = world.get_component::<AI>(e).unwrap();
            assert_eq!(ai.state, AIState::SurePlayerSeen);
            assert!(ai.last_known_player_position.is_some());
            assert!(
                world.has_component::<Weapon>(e),
                "flipped bots arm themselves"
            );
            // The brief survives the flip so `group` stays addressable.
            assert!(ai.passive.is_some());
        }
        // A second alert finds nothing left to flip.
        assert_eq!(alert_passives(&mut world, AlertTarget::All), 0);
    }

    #[test]
    fn alert_group_and_zone_are_selective() {
        let mut world = crowd_world(None);
        assert_eq!(alert_passives(&mut world, AlertTarget::Group("nobody")), 0);
        assert_eq!(alert_passives(&mut world, AlertTarget::Group("crowd")), 1);
        assert_eq!(count_passives(&world), 1);

        // Zone alert: only the passives standing inside the zone flip.
        let mut world = crowd_world(None);
        assert_eq!(alert_passives(&mut world, AlertTarget::Zone("doors")), 0);
        // Move one bot into the zone.
        let e = world.query::<Enemy>()[1];
        *world.get_component_mut::<Position>(e).unwrap() = Position::new(500.0, 140.0);
        assert_eq!(alert_passives(&mut world, AlertTarget::Zone("doors")), 1);
        assert_eq!(alert_passives(&mut world, AlertTarget::Zone("unknown")), 0);
    }

    #[test]
    fn hurting_one_passive_flips_the_whole_floor() {
        let mut sim = Simulation::from_world(crowd_world(None));
        sim.step(1.0 / 60.0);
        let e = sim.world.query::<Enemy>()[0];
        sim.world
            .get_component_mut::<Health>(e)
            .unwrap()
            .take_damage(10);
        sim.step(1.0 / 60.0);
        assert_eq!(count_passives(&sim.world), 0, "everyone turned");
        // The flipped bots chase: after a while they close on the player.
        let before: Vec<f32> = enemy_positions(&sim.world)
            .iter()
            .map(|p| p.distance(Vec2::new(500.0, 700.0)))
            .collect();
        sim.run_frames(120, 1.0 / 60.0);
        let after: Vec<f32> = enemy_positions(&sim.world)
            .iter()
            .map(|p| p.distance(Vec2::new(500.0, 700.0)))
            .collect();
        assert!(after.iter().zip(&before).any(|(a, b)| a < b));
    }

    #[test]
    fn downing_a_passive_outright_still_turns_the_crowd() {
        let mut sim = Simulation::from_world(crowd_world(None));
        sim.step(1.0 / 60.0);
        let e = sim.world.query::<Enemy>()[0];
        sim.world
            .get_component_mut::<Health>(e)
            .unwrap()
            .take_damage(9999);
        sim.step(1.0 / 60.0);
        assert_eq!(count_passives(&sim.world), 0, "the survivor turned");
        let other = sim.world.query::<Enemy>()[1];
        assert_eq!(
            sim.world.get_component::<AI>(other).unwrap().state,
            AIState::SurePlayerSeen
        );
        // The dead one stays a corpse: no weapon, no motion.
        assert!(!sim.world.has_component::<Weapon>(e));
        let v = sim.world.get_component::<Velocity>(e).unwrap();
        assert!(v.x == 0.0 && v.y == 0.0);
    }

    #[test]
    fn passives_count_as_rogues() {
        let world = crowd_world(None);
        assert_eq!(crate::scenario::count_rogues(&world), (0, 2));
        assert_eq!(crate::game::count_alive_enemies(&world), 2);
    }
}
