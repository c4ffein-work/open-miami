//! Full-playthrough gate driven by a headless heuristic bot.
//!
//! This integration test exercises the whole gameplay engine (via the
//! [`Simulation`] harness in `src/sim.rs`) with a scripted bot and asserts a set
//! of reliability properties for every level:
//!
//!   * no panic while simulating,
//!   * all entity positions stay finite (no NaN / infinity),
//!   * the run reaches a *terminal* state (the bot wins, or the player dies)
//!     within a bounded frame budget — it must not stall forever,
//!   * structural winnability: every enemy spawn is physically reachable from
//!     the player spawn,
//!   * determinism: replaying a level from scratch yields an identical outcome
//!     and identical final player position (this is what the per-`World` RNG
//!     refactor buys us),
//!   * a hand-built guaranteed-win scenario is actually cleared by the bot.
//!
//! ## History: three bugs this gate surfaced (now fixed)
//!
//! When first written, this gate caught three genuine defects that the unit
//! validators missed. They have since been fixed, and the quarantine lists
//! below are empty — every level must now be winnable and terminate. Kept here
//! as a record of what the gate is guarding against:
//!
//!   * **Level 2** sealed the player inside a fully-closed box of walls (no
//!     enemy reachable, never terminates). Fixed: the bottom wall now has a
//!     doorway to breach.
//!   * **Level 8** placed one enemy inside a walled pocket unreachable from the
//!     player spawn. Fixed: that enemy was relocated to open ground.
//!   * **Level 7** let a patrolling enemy wander *outside the world bounds* (to
//!     a negative Y) where the player could not follow. Fixed: the movement
//!     system now clamps positions to the play field.

use open_miami::collision::circle_rect_collision;
use open_miami::components::{EnemyType, Position};
use open_miami::ecs::world::Wall;
use open_miami::ecs::World;
use open_miami::game::{spawn_enemy_with_type, spawn_player};
use open_miami::levels::{level_def, LEVEL_COUNT, PLAYER_SPAWN};
use open_miami::math::Vec2;
use open_miami::sim::Simulation;
use std::collections::HashSet;

const DT: f32 = 1.0 / 60.0;
/// 90 simulated seconds at 60 FPS.
const FRAME_BUDGET: usize = 5400;
/// How often (in frames) to sample every position for finiteness.
const FINITE_CHECK_INTERVAL: usize = 30;

/// Levels quarantined as unable to terminate within budget. Empty: every level
/// must now reach a terminal state. Left in place so a future regression can be
/// documented explicitly rather than by weakening an assertion.
const KNOWN_NON_TERMINATING: &[usize] = &[];

/// Levels quarantined as structurally unwinnable (an enemy spawn unreachable
/// from the player spawn). Empty: every enemy must now be reachable.
const STRUCTURALLY_UNWINNABLE: &[usize] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Win,
    PlayerDead,
    Timeout,
}

struct RunResult {
    outcome: Outcome,
    frames: usize,
    final_player_pos: Option<Vec2>,
}

/// Assert that every position component in the world is finite.
fn assert_all_positions_finite(world: &World, level: usize, frame: usize) {
    // Positions are attached to players, enemies, bullets, pickups, thrown
    // weapons, trails... they all use the `Position` component, so one query
    // covers the whole world.
    for entity in world.query::<Position>() {
        if let Some(pos) = world.get_component::<Position>(entity) {
            assert!(
                pos.x.is_finite() && pos.y.is_finite(),
                "level {level}: non-finite position {:?} at frame {frame}",
                (pos.x, pos.y)
            );
        }
    }
}

/// Run a `Simulation` with the bot until a terminal state or the frame budget.
/// Also continuously asserts (a) no panic and (b) finite positions.
fn run_with_bot(mut sim: Simulation, level: usize) -> RunResult {
    let mut outcome = Outcome::Timeout;
    let mut frames = 0;

    for frame in 0..FRAME_BUDGET {
        if frame % FINITE_CHECK_INTERVAL == 0 {
            assert_all_positions_finite(&sim.world, level, frame);
        }

        sim.bot_step(DT);
        frames = frame + 1;

        if !sim.player_alive() {
            outcome = Outcome::PlayerDead;
            break;
        }
        if sim.enemies_alive() == 0 {
            outcome = Outcome::Win;
            break;
        }
    }

    // Final finiteness sweep.
    assert_all_positions_finite(&sim.world, level, frames);

    RunResult {
        outcome,
        frames,
        final_player_pos: sim.player_position(),
    }
}

/// The set of enemy spawns in a level that are physically UNREACHABLE from the
/// player spawn, computed with a fine flood fill over the actual collision model
/// (a player-radius circle sweeping a 4px grid). This is the accurate notion of
/// reachability: the AI's coarse `NavigationGrid` blocks whole 50px cells near
/// any wall and so reports many physically-reachable enemies as unreachable,
/// which would make a structural-winnability check based on it meaningless.
fn unreachable_enemies(level: usize) -> Vec<(f32, f32)> {
    const STEP: f32 = 4.0;
    const PLAYER_RADIUS: f32 = 15.0;

    let def = level_def(level);
    let walls: Vec<Wall> = def
        .walls
        .iter()
        .map(|&(x, y, w, h)| Wall::new(x, y, w, h))
        .collect();

    let free = |p: Vec2| -> bool {
        if p.x < 0.0 || p.y < 0.0 || p.x > 2000.0 || p.y > 2000.0 {
            return false;
        }
        !walls
            .iter()
            .any(|w| circle_rect_collision(p, PLAYER_RADIUS, w.x, w.y, w.width, w.height))
    };
    let key = |p: Vec2| ((p.x / STEP).round() as i32, (p.y / STEP).round() as i32);

    // Flood fill the walkable region reachable from the player spawn.
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut stack = vec![PLAYER_SPAWN];
    seen.insert(key(PLAYER_SPAWN));
    while let Some(p) = stack.pop() {
        for (dx, dy) in [(STEP, 0.0), (-STEP, 0.0), (0.0, STEP), (0.0, -STEP)] {
            let np = Vec2::new(p.x + dx, p.y + dy);
            // Bound the search to the playable area for speed.
            if np.x < 50.0 || np.x > 950.0 || np.y < 20.0 || np.y > 780.0 {
                continue;
            }
            if seen.contains(&key(np)) || !free(np) {
                continue;
            }
            seen.insert(key(np));
            stack.push(np);
        }
    }

    // An enemy is reachable if any player-standable point next to it was seen.
    let mut unreachable = Vec::new();
    for &(ex, ey, _) in &def.enemies {
        let near = [
            (0.0, 0.0),
            (20.0, 0.0),
            (-20.0, 0.0),
            (0.0, 20.0),
            (0.0, -20.0),
            (28.0, 0.0),
            (-28.0, 0.0),
            (0.0, 28.0),
            (0.0, -28.0),
        ]
        .iter()
        .any(|(dx, dy)| seen.contains(&key(Vec2::new(ex + dx, ey + dy))));
        if !near {
            unreachable.push((ex, ey));
        }
    }
    unreachable
}

#[test]
fn structural_winnability() {
    for level in 0..LEVEL_COUNT {
        let unreachable = unreachable_enemies(level);
        if STRUCTURALLY_UNWINNABLE.contains(&level) {
            assert!(
                !unreachable.is_empty(),
                "level {level} is listed as structurally unwinnable but all enemies are now \
                 reachable — the level was fixed; update STRUCTURALLY_UNWINNABLE.",
            );
        } else {
            assert!(
                unreachable.is_empty(),
                "level {level}: enemies unreachable from player spawn {:?} — level is not \
                 structurally winnable",
                unreachable
            );
        }
    }
}

#[test]
fn every_level_reaches_a_terminal_state() {
    for level in 0..LEVEL_COUNT {
        // (a) no panic, (b) finite positions asserted inside run_with_bot.
        let result = run_with_bot(Simulation::new(level), level);

        if KNOWN_NON_TERMINATING.contains(&level) {
            // Quarantined: assert the (documented) stall really happens, so the
            // gate stays honest and flags a fix or a regression.
            assert_eq!(
                result.outcome,
                Outcome::Timeout,
                "level {level} is listed as non-terminating but reached {:?} — it appears fixed; \
                 update KNOWN_NON_TERMINATING.",
                result.outcome
            );
        } else {
            assert_ne!(
                result.outcome,
                Outcome::Timeout,
                "level {level}: bot failed to reach a terminal state within {FRAME_BUDGET} frames \
                 (stalled). This is a stuck bot or a soft-locked level.",
            );
            assert!(result.frames >= 1, "level {level}: no frames advanced");
        }
    }
}

#[test]
fn playthrough_is_deterministic() {
    // Same level, two fresh simulations, must match exactly. This is the proof
    // that the RNG now lives in the per-`World` state rather than a global (and
    // that entity iteration order is deterministic).
    let level = 0;
    let a = run_with_bot(Simulation::new(level), level);
    let b = run_with_bot(Simulation::new(level), level);

    assert_eq!(
        a.outcome, b.outcome,
        "outcome should be deterministic across runs"
    );
    assert_eq!(
        a.frames, b.frames,
        "termination frame should be deterministic across runs"
    );
    match (a.final_player_pos, b.final_player_pos) {
        (Some(pa), Some(pb)) => assert_eq!(
            (pa.x, pa.y),
            (pb.x, pb.y),
            "final player position should be identical across runs"
        ),
        (None, None) => {}
        _ => panic!("player presence differed across runs"),
    }
}

#[test]
fn guaranteed_win_scenario_is_cleared() {
    // A tiny open arena: the player plus a few *idle* enemies placed in front of
    // the player with clear line of sight. The bot should mow them down without
    // dying, well within a small budget.
    let mut world = World::new();
    spawn_player(&mut world, Vec2::new(500.0, 500.0));
    spawn_enemy_with_type(&mut world, Vec2::new(500.0, 300.0), EnemyType::Idle);
    spawn_enemy_with_type(&mut world, Vec2::new(400.0, 320.0), EnemyType::Idle);
    spawn_enemy_with_type(&mut world, Vec2::new(600.0, 320.0), EnemyType::Idle);

    let mut sim = Simulation::from_world(world);
    assert_eq!(sim.enemies_alive(), 3);

    let mut cleared = false;
    for _ in 0..1200 {
        sim.bot_step(DT);
        if sim.enemies_alive() == 0 {
            cleared = true;
            break;
        }
        if !sim.player_alive() {
            break;
        }
    }

    assert!(
        cleared,
        "bot should clear the guaranteed-win arena (enemies alive: {}, player alive: {})",
        sim.enemies_alive(),
        sim.player_alive()
    );
    assert!(sim.player_alive(), "player should survive the easy arena");
}
