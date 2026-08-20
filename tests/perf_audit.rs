//! Perf-audit instrumentation: prints ms/tick for representative floors and
//! micro-costs of the ECS primitives. Run with:
//!   cargo test --release --test perf_audit -- --nocapture
//! Asserts nothing about time (never flaky); only that the sim stays sane.

use open_miami::components::{AIState, Enemy, Player, Position, AI};
use open_miami::ecs::World;
use open_miami::sim::Simulation;
use std::time::Instant;

const DT: f32 = 1.0 / 60.0;

/// Meaningful timings need --release; under a debug build the loops shrink so
/// `make verify` stays fast (the printed numbers are then only smoke output).
#[cfg(debug_assertions)]
const SCALE: usize = 20;
#[cfg(not(debug_assertions))]
const SCALE: usize = 1;

fn alert_all(sim: &mut Simulation) {
    let player_pos = sim.player_position().unwrap();
    for e in sim.world.query::<Enemy>() {
        if let Some(ai) = sim.world.get_component_mut::<AI>(e) {
            ai.state = AIState::SurePlayerSeen;
            ai.last_known_player_position = Some(Position::new(player_pos.x, player_pos.y));
            ai.state_timer = 1e9; // never lose the player
        }
    }
}

fn bench_floor(level: usize, label: &str, alerted: bool, ticks: usize) {
    let ticks = ticks / SCALE;
    let mut sim = Simulation::new(level);
    if alerted {
        alert_all(&mut sim);
    }
    // Warm the nav cache and any lazy state.
    sim.run_frames(30, DT);
    if alerted {
        alert_all(&mut sim); // re-pin in case states drifted during warmup
    }
    let t0 = Instant::now();
    for _ in 0..ticks {
        if alerted {
            // Keep every enemy chasing the whole run (the interesting path).
            alert_all(&mut sim);
        }
        sim.step(DT);
    }
    let el = t0.elapsed();
    println!(
        "PERF {label}: {ticks} ticks in {:.1} ms  ->  {:.3} ms/tick ({} enemies alive)",
        el.as_secs_f64() * 1000.0,
        el.as_secs_f64() * 1000.0 / ticks as f64,
        sim.enemies_alive()
    );
    assert!(sim.enemy_count() > 0);
}

#[test]
fn perf_floor_ticks() {
    // Idle floors (nobody alerted): the wander/patrol path.
    bench_floor(1, "floor idx 1 (lobby, passive crowd) idle", false, 1000);
    bench_floor(2, "floor idx 2 (12 rogues, 27 walls) idle", false, 1000);
    // Everyone alerted and chasing: LOS checks + A* pathfinding path.
    bench_floor(2, "floor idx 2 ALL ALERTED (chase/A*)", true, 1000);
    bench_floor(11, "floor idx 11 ALL ALERTED", true, 1000);
    // The boss floor.
    bench_floor(
        open_miami::levels::BOSS_LEVEL,
        "floor 13.5 (boss)",
        true,
        1000,
    );
}

/// Worst realistic AI case: the player tucked into floor 2's central pocket so
/// (almost) no enemy has padded LOS — every alerted enemy runs A* every tick.
#[test]
fn perf_all_enemies_pathfinding() {
    let mut sim = Simulation::new(2);
    let player = sim.player().unwrap();
    if let Some(p) = sim.world.get_component_mut::<Position>(player) {
        p.x = 500.0;
        p.y = 300.0;
    }
    alert_all(&mut sim);
    sim.run_frames(5, DT);
    let t0 = Instant::now();
    let ticks = 1000 / SCALE;
    for _ in 0..ticks {
        alert_all(&mut sim);
        sim.step(DT);
    }
    let el = t0.elapsed();
    println!(
        "PERF floor idx 2, player pocketed, 12 chasers all pathfinding: {:.3} ms/tick",
        el.as_secs_f64() * 1000.0 / ticks as f64
    );
    assert!(sim.enemy_count() > 0);
}

#[test]
fn perf_ecs_primitives() {
    let mut world = World::new();
    open_miami::game::initialize_game(&mut world, 2);
    let n_enemies = world.query::<Enemy>().len();

    let t0 = Instant::now();
    let mut acc = 0usize;
    for _ in 0..(100_000 / SCALE) {
        acc += world.query::<Enemy>().len();
    }
    let q = t0.elapsed();

    let player = world.query::<Player>()[0];
    let t0 = Instant::now();
    let mut sum = 0.0f32;
    for _ in 0..(1_000_000 / SCALE) {
        sum += world.get_component::<Position>(player).unwrap().x;
    }
    let g = t0.elapsed();

    println!(
        "PERF ecs: query::<Enemy>() ({n_enemies} hits) = {:.0} ns/call ; get_component::<Position>() = {:.0} ns/call (acc {acc}, sum {sum})",
        q.as_secs_f64() * 1e9 / (100_000 / SCALE) as f64,
        g.as_secs_f64() * 1e9 / (1_000_000 / SCALE) as f64,
    );
    assert!(acc > 0);
}

#[test]
fn perf_pathfinding() {
    use open_miami::pathfinding::NavigationGrid;
    let mut world = World::new();
    open_miami::game::initialize_game(&mut world, 2);
    let walls = world.walls().to_vec();

    let t0 = Instant::now();
    let grid = NavigationGrid::new(&walls);
    let build = t0.elapsed();

    // A representative cross-room path (both endpoints inside floor 2's rooms).
    let a = open_miami::math::Vec2::new(100.0, 300.0);
    let b = open_miami::math::Vec2::new(900.0, 630.0);
    let t0 = Instant::now();
    let mut len = 0usize;
    for _ in 0..(1000 / SCALE) {
        len += grid.find_path(a, b).map(|p| p.len()).unwrap_or(0);
    }
    let fp = t0.elapsed();

    // Unreachable goal: A* floods every reachable cell before giving up —
    // the worst case, and what a chase toward a walled-off target pays
    // EVERY tick per enemy.
    let c = open_miami::math::Vec2::new(1850.0, 1850.0);
    let t0 = Instant::now();
    let mut fails = 0usize;
    for _ in 0..(1000 / SCALE) {
        if grid.find_path(a, c).is_none() {
            fails += 1;
        }
    }
    let ff = t0.elapsed();
    println!(
        "PERF pathfinding: grid build = {:.2} ms ; find_path(cross-room) = {:.1} us/call ({} waypoints) ; find_path(unreachable, full flood) = {:.1} us/call ({} fails)",
        build.as_secs_f64() * 1000.0,
        fp.as_secs_f64() * 1e6 / (1000 / SCALE) as f64,
        len / (1000 / SCALE),
        ff.as_secs_f64() * 1e6 / (1000 / SCALE) as f64,
        fails
    );
    assert!(len > 0);
}
