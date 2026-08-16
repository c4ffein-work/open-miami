//! Headless simulation harness.
//!
//! The browser build drives the game from a `requestAnimationFrame` loop that
//! reads input, ticks every system, then renders. Rendering and input are the
//! only browser-specific parts — all the *gameplay* lives in plain systems that
//! operate on a [`World`]. [`Simulation`] wires those systems together in the
//! exact same order as the real loop, with no rendering and no browser, so the
//! true game logic can be run:
//!
//! * deterministically in tests (fixed `dt`, fixed initial state),
//! * "fast-forwarded" at any rate (call [`Simulation::run_frames`] with a large
//!   frame count, or a bigger `dt`, to simulate minutes of play instantly),
//! * with scripted player actions ([`Simulation::player_fire`],
//!   [`Simulation::player_throw`], [`Simulation::player_pickup`],
//!   [`Simulation::set_player_velocity`]).
//!
//! This is the "simulated e2e" layer: it exercises the whole engine end to end
//! without a browser, complementing the Playwright tests that drive the real
//! rendered game.

use crate::collision::has_line_of_sight;
use crate::components::{Enemy, Health, Position, Speed, Weapon, WeaponPickup, WeaponType};
use crate::components::{Player, Velocity};
use crate::ecs::{Entity, System, World};
use crate::game::{
    count_alive_enemies, fire_player_weapon, get_player_position, initialize_game, is_player_alive,
};
use crate::math::Vec2;
use crate::pathfinding::NavigationGrid;
use crate::systems::{
    AISystem, BossSystem, BulletSystem, CombatSystem, MovementSystem, PickupSystem,
    ProjectileTrailSystem, StunSystem, ThrownWeaponSystem, WeaponUpdateSystem,
};

/// A headless instance of the full game engine.
pub struct Simulation {
    pub world: World,
    stun: StunSystem,
    weapon: WeaponUpdateSystem,
    ai: AISystem,
    boss: BossSystem,
    movement: MovementSystem,
    combat: CombatSystem,
    bullet: BulletSystem,
    thrown: ThrownWeaponSystem,
    projectile: ProjectileTrailSystem,
    pickup: PickupSystem,
    /// Bot navigation state: the player position at the previous `bot_step`,
    /// used to detect when the bot is wedged against geometry.
    bot_prev_pos: Option<Vec2>,
    /// While positive, the bot is executing an "unstick" maneuver: it strafes
    /// along `bot_unstick_dir` instead of steering straight at its target.
    bot_unstick_timer: f32,
    bot_unstick_dir: Vec2,
    /// Lazily-built fine navigation grid for the bot (walkable flags on a grid
    /// finer than the AI's `NavigationGrid`, so it can path through the sub-cell
    /// gaps a 15px-radius player physically fits through). Cached because the
    /// walls never change during a run.
    bot_fine_grid: Option<FineGrid>,
}

/// A fine walkability grid used only by the headless bot's navigation. Cells are
/// [`FineGrid::CELL`] wide and a cell is walkable if a player-radius circle at
/// its centre clears every wall.
struct FineGrid {
    cols: usize,
    rows: usize,
    walkable: Vec<bool>,
}

impl FineGrid {
    const CELL: f32 = 20.0;
    const W: f32 = 2000.0;
    const H: f32 = 2000.0;
    /// A slightly-reduced player radius so the bot is willing to graze corners
    /// it can physically pass (avoids being overly conservative at cell centres).
    const CLEARANCE: f32 = 13.0;

    fn build(walls: &[crate::ecs::world::Wall]) -> Self {
        let cols = (Self::W / Self::CELL) as usize;
        let rows = (Self::H / Self::CELL) as usize;
        let mut walkable = vec![false; cols * rows];
        for j in 0..rows {
            for i in 0..cols {
                let c = Vec2::new(
                    i as f32 * Self::CELL + Self::CELL / 2.0,
                    j as f32 * Self::CELL + Self::CELL / 2.0,
                );
                let blocked = walls.iter().any(|w| {
                    crate::collision::circle_rect_collision(
                        c,
                        Self::CLEARANCE,
                        w.x,
                        w.y,
                        w.width,
                        w.height,
                    )
                });
                walkable[j * cols + i] = !blocked;
            }
        }
        FineGrid {
            cols,
            rows,
            walkable,
        }
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        (
            (p.x / Self::CELL).floor() as i32,
            (p.y / Self::CELL).floor() as i32,
        )
    }

    fn in_bounds(&self, i: i32, j: i32) -> bool {
        i >= 0 && j >= 0 && (i as usize) < self.cols && (j as usize) < self.rows
    }

    fn is_walkable(&self, i: i32, j: i32) -> bool {
        self.in_bounds(i, j) && self.walkable[j as usize * self.cols + i as usize]
    }

    fn center(&self, i: i32, j: i32) -> Vec2 {
        Vec2::new(
            i as f32 * Self::CELL + Self::CELL / 2.0,
            j as f32 * Self::CELL + Self::CELL / 2.0,
        )
    }

    /// Breadth-first search from `start` toward `goal`; returns the world point
    /// of the first step to take. Deterministic (fixed neighbour order, indices
    /// keyed by grid position). If start/goal cells are blocked they are snapped
    /// to the nearest walkable cell first.
    fn next_step(&self, start: Vec2, goal: Vec2) -> Option<Vec2> {
        let s = self.nearest_walkable(self.cell_of(start))?;
        let g = self.nearest_walkable(self.cell_of(goal))?;
        if s == g {
            return Some(self.center(g.0, g.1));
        }

        use std::collections::VecDeque;
        let idx = |i: i32, j: i32| j as usize * self.cols + i as usize;
        let mut came: Vec<i32> = vec![-1; self.cols * self.rows];
        let mut visited = vec![false; self.cols * self.rows];
        let mut q = VecDeque::new();
        visited[idx(s.0, s.1)] = true;
        q.push_back(s);
        // 4-connected, fixed order for determinism.
        const NB: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let mut found = false;
        while let Some((ci, cj)) = q.pop_front() {
            if (ci, cj) == g {
                found = true;
                break;
            }
            for (di, dj) in NB {
                let (ni, nj) = (ci + di, cj + dj);
                if self.is_walkable(ni, nj) && !visited[idx(ni, nj)] {
                    visited[idx(ni, nj)] = true;
                    came[idx(ni, nj)] = idx(ci, cj) as i32;
                    q.push_back((ni, nj));
                }
            }
        }
        if !found {
            return None;
        }
        // Walk back from goal to the cell right after start.
        let mut cur = idx(g.0, g.1);
        let start_idx = idx(s.0, s.1);
        let mut prev = cur;
        while came[cur] != -1 {
            prev = cur;
            if came[cur] as usize == start_idx {
                break;
            }
            cur = came[cur] as usize;
        }
        let step_cell = prev;
        let si = (step_cell % self.cols) as i32;
        let sj = (step_cell / self.cols) as i32;
        Some(self.center(si, sj))
    }

    /// Nearest walkable cell to `(i,j)` via an expanding ring search.
    fn nearest_walkable(&self, (i, j): (i32, i32)) -> Option<(i32, i32)> {
        if self.is_walkable(i, j) {
            return Some((i, j));
        }
        for r in 1i32..20 {
            let mut best: Option<(i32, i32)> = None;
            for dj in -r..=r {
                for di in -r..=r {
                    if di.abs() != r && dj.abs() != r {
                        continue; // ring only
                    }
                    if self.is_walkable(i + di, j + dj) {
                        // Keep the first in a fixed scan order → deterministic.
                        if best.is_none() {
                            best = Some((i + di, j + dj));
                        }
                    }
                }
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
}

impl Simulation {
    /// Start a simulation on the given level (same layout as the real game).
    pub fn new(level: usize) -> Self {
        let mut world = World::new();
        initialize_game(&mut world, level);
        Self::from_world(world)
    }

    /// Wrap an already-populated world (useful for bespoke test scenarios).
    pub fn from_world(world: World) -> Self {
        Simulation {
            world,
            stun: StunSystem,
            weapon: WeaponUpdateSystem,
            ai: AISystem::default(),
            boss: BossSystem,
            movement: MovementSystem,
            combat: CombatSystem,
            bullet: BulletSystem,
            thrown: ThrownWeaponSystem,
            projectile: ProjectileTrailSystem,
            pickup: PickupSystem,
            bot_prev_pos: None,
            bot_unstick_timer: 0.0,
            bot_unstick_dir: Vec2::zero(),
            bot_fine_grid: None,
        }
    }

    /// Advance the whole simulation by `dt` seconds — one engine tick, running
    /// every gameplay system in the same order as the browser loop.
    pub fn step(&mut self, dt: f32) {
        self.stun.run(&mut self.world, dt);
        self.weapon.run(&mut self.world, dt);
        self.ai.run(&mut self.world, dt);
        self.boss.run(&mut self.world, dt);
        self.movement.run(&mut self.world, dt);
        self.combat.run(&mut self.world, dt);
        self.bullet.run(&mut self.world, dt);
        self.thrown.run(&mut self.world, dt);
        self.projectile.run(&mut self.world, dt);
        self.pickup.run(&mut self.world, dt);
    }

    /// Run `frames` ticks of `dt` seconds each (fast-forward).
    pub fn run_frames(&mut self, frames: usize, dt: f32) {
        for _ in 0..frames {
            self.step(dt);
        }
    }

    // --- Scripted player actions (stand-ins for keyboard/mouse input) ---

    /// The player entity, if one exists.
    pub fn player(&self) -> Option<Entity> {
        self.world.query::<Player>().into_iter().next()
    }

    /// Set the player's velocity directly (as WASD movement would).
    pub fn set_player_velocity(&mut self, velocity: Vec2) {
        if let Some(player) = self.player() {
            if let Some(v) = self.world.get_component_mut::<Velocity>(player) {
                v.x = velocity.x;
                v.y = velocity.y;
            }
        }
    }

    /// Fire the player's weapon toward a world point (as left-click would).
    pub fn player_fire(&mut self, target: Vec2) -> bool {
        fire_player_weapon(&mut self.world, target)
    }

    /// Throw the player's held weapon toward a world point (as right-click would).
    pub fn player_throw(&mut self, target: Vec2) -> bool {
        let from = match get_player_position(&self.world) {
            Some(p) => p,
            None => return false,
        };
        ThrownWeaponSystem::throw_from_player(&mut self.world, target - from)
    }

    /// Pick up / swap the weapon under the player (as the E key would).
    pub fn player_pickup(&mut self) -> Option<WeaponType> {
        PickupSystem::swap_for_player(&mut self.world)
    }

    // --- Queries ---

    pub fn player_alive(&self) -> bool {
        is_player_alive(&self.world)
    }

    pub fn enemies_alive(&self) -> usize {
        count_alive_enemies(&self.world)
    }

    pub fn player_position(&self) -> Option<Vec2> {
        get_player_position(&self.world)
    }

    /// Total enemy entities (alive or downed).
    pub fn enemy_count(&self) -> usize {
        self.world.query::<Enemy>().len()
    }

    // --- Query helpers used by the headless bot / gate tests ---

    /// Position of the nearest *alive* enemy to the given point, if any.
    pub fn nearest_alive_enemy(&self, from: Vec2) -> Option<(Entity, Vec2)> {
        self.world
            .query::<Enemy>()
            .into_iter()
            .filter(|&e| {
                self.world
                    .get_component::<Health>(e)
                    .map(|h| h.is_alive())
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                self.world
                    .get_component::<Position>(e)
                    .map(|p| (e, p.to_vec2()))
            })
            .min_by(|(_, a), (_, b)| {
                from.distance(*a)
                    .partial_cmp(&from.distance(*b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// All alive enemies as `(entity, position)`, sorted nearest-first from
    /// `from`. Deterministic (query is id-sorted; distance ties keep that order).
    fn alive_enemies_by_distance(&self, from: Vec2) -> Vec<(Entity, Vec2)> {
        let mut list: Vec<(Entity, Vec2)> = self
            .world
            .query::<Enemy>()
            .into_iter()
            .filter(|&e| {
                self.world
                    .get_component::<Health>(e)
                    .map(|h| h.is_alive())
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                self.world
                    .get_component::<Position>(e)
                    .map(|p| (e, p.to_vec2()))
            })
            .collect();
        list.sort_by(|(_, a), (_, b)| {
            from.distance(*a)
                .partial_cmp(&from.distance(*b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        list
    }

    /// Position of the nearest weapon pickup to the given point, if any.
    pub fn nearest_pickup(&self, from: Vec2) -> Option<Vec2> {
        self.world
            .query::<WeaponPickup>()
            .into_iter()
            .filter_map(|e| self.world.get_component::<Position>(e).map(|p| p.to_vec2()))
            .min_by(|a, b| {
                from.distance(*a)
                    .partial_cmp(&from.distance(*b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// The player's current movement speed (defaults to a sane value if missing).
    fn player_speed(&self) -> f32 {
        self.player()
            .and_then(|p| self.world.get_component::<Speed>(p))
            .map(|s| s.value)
            .unwrap_or(200.0)
    }

    /// Whether the player currently holds a weapon that can actually deal damage
    /// (a weapon component that still has ammo, or a melee weapon).
    fn player_has_usable_weapon(&self) -> bool {
        match self
            .player()
            .and_then(|p| self.world.get_component::<Weapon>(p))
        {
            Some(w) => w.weapon_type == WeaponType::Melee || w.ammo > 0,
            None => false,
        }
    }

    /// The direction the bot should move to head toward `target`. Returns a unit
    /// vector (or zero if already there). Uses a clear line-of-sight shortcut,
    /// then a fine BFS grid (accurate to the player's physical radius), then the
    /// AI's coarse grid, then a straight line as last resort.
    fn nav_direction(
        &mut self,
        from: Vec2,
        target: Vec2,
        walls: &[crate::ecs::world::Wall],
    ) -> Vec2 {
        // Clear shot: just go straight.
        if has_line_of_sight(from, target, walls) {
            return (target - from).normalize();
        }

        // Fine BFS grid (built once, cached) — matches player physics closely.
        if self.bot_fine_grid.is_none() {
            self.bot_fine_grid = Some(FineGrid::build(walls));
        }
        if let Some(grid) = &self.bot_fine_grid {
            if let Some(wp) = grid.next_step(from, target) {
                let d = wp - from;
                if d.length() > 1.0 {
                    return d.normalize();
                }
            }
        }

        // Coarse grid fallback.
        let grid = NavigationGrid::new(walls);
        if let Some(wp) = grid.get_next_waypoint(from, target) {
            return (wp - from).normalize();
        }
        (target - from).normalize()
    }

    /// Drive the player toward a world point, with anti-stuck handling.
    fn navigate_toward(&mut self, target: Vec2) {
        let from = match self.player_position() {
            Some(p) => p,
            None => return,
        };
        let speed = self.player_speed();
        let walls = self.world.walls().to_vec();

        // If currently unsticking, strafe along the chosen direction.
        if self.bot_unstick_timer > 0.0 {
            let dir = self.bot_unstick_dir;
            if dir.length() > 0.0 {
                self.set_player_velocity(dir * speed);
                return;
            }
        }

        let dir = self.nav_direction(from, target, &walls);
        if dir.length() > 0.0 {
            self.set_player_velocity(dir * speed);
        } else {
            self.set_player_velocity(Vec2::zero());
        }
    }

    /// Detect whether the bot barely moved since the last tick while it *wanted*
    /// to move, and if so kick off an unstick strafe perpendicular to its goal.
    fn update_stuck_state(&mut self, want_move_toward: Option<Vec2>, dt: f32) {
        if self.bot_unstick_timer > 0.0 {
            self.bot_unstick_timer -= dt;
        }
        let cur = match self.player_position() {
            Some(p) => p,
            None => return,
        };
        if let (Some(prev), Some(goal)) = (self.bot_prev_pos, want_move_toward) {
            let moved = cur.distance(prev);
            // Wanted to move but barely did => wedged. Start/refresh a strafe.
            if moved < 0.5 && self.bot_unstick_timer <= 0.0 {
                let to_goal = (goal - cur).normalize();
                // Perpendicular strafe; pick the side using a cheap deterministic
                // hash of the position so it varies but replays identically.
                let perp = Vec2::new(-to_goal.y, to_goal.x);
                let sign = if ((cur.x as i64) ^ (cur.y as i64)) & 1 == 0 {
                    1.0
                } else {
                    -1.0
                };
                self.bot_unstick_dir = (perp * sign + to_goal * 0.3).normalize();
                self.bot_unstick_timer = 0.3;
            }
        }
        self.bot_prev_pos = Some(cur);
    }

    /// One tick of a simple, robust heuristic bot, then advance the simulation
    /// by `dt`. One call == one advanced frame.
    ///
    /// Priorities each tick:
    /// 1. If there are no alive enemies, stop and just tick.
    /// 2. If the player has no usable weapon, head for the nearest weapon pickup
    ///    (picking it up when close); if there are none, advance on the enemy to
    ///    melee / recover a dropped weapon.
    /// 3. If there's a clear line of sight to the nearest enemy, aim and fire
    ///    (closing the distance a little if far away).
    /// 4. Otherwise navigate toward the enemy via the pathfinder.
    pub fn bot_step(&mut self, dt: f32) {
        let player_pos = match self.player_position() {
            Some(p) => p,
            None => {
                self.step(dt);
                return;
            }
        };

        let candidates = self.alive_enemies_by_distance(player_pos);
        if candidates.is_empty() {
            // Nothing to fight; stand still.
            self.set_player_velocity(Vec2::zero());
            self.bot_prev_pos = Some(player_pos);
            self.step(dt);
            return;
        }

        // Prefer the nearest enemy we can actually engage: one with a clear shot,
        // or one the fine navigation grid can path to. This lets the bot skip
        // over an enemy it currently can't reach (e.g. one that wandered into an
        // awkward spot) and clear the rest, rather than fixating and stalling.
        let walls = self.world.walls().to_vec();
        if self.bot_fine_grid.is_none() {
            self.bot_fine_grid = Some(FineGrid::build(&walls));
        }
        let mut chosen: Option<Vec2> = None;
        for &(_e, pos) in &candidates {
            let reachable = has_line_of_sight(player_pos, pos, &walls)
                || self
                    .bot_fine_grid
                    .as_ref()
                    .map(|g| g.next_step(player_pos, pos).is_some())
                    .unwrap_or(false);
            if reachable {
                chosen = Some(pos);
                break;
            }
        }
        // If none look reachable, just target the nearest and try anyway.
        let enemy_pos = chosen.unwrap_or(candidates[0].1);

        // The point (if any) the bot is actively trying to walk toward this tick;
        // used for stuck detection.
        let mut move_goal: Option<Vec2> = None;

        if !self.player_has_usable_weapon() {
            // Need a weapon: grab the nearest pickup, or advance to melee range.
            if let Some(pickup_pos) = self.nearest_pickup(player_pos) {
                // Try to grab it every tick; the swap only succeeds once we're
                // physically overlapping it. Keep closing the distance until it
                // does (a fixed "close enough" threshold can leave the bot parked
                // just outside the real pickup radius, deadlocked).
                if self.player_pickup().is_some() {
                    self.set_player_velocity(Vec2::zero());
                } else {
                    move_goal = Some(pickup_pos);
                }
            } else {
                // No pickups exist: close on the enemy (a dropped weapon may
                // appear, or a melee weapon may still connect).
                move_goal = Some(enemy_pos);
            }
        } else {
            // Armed: engage the enemy.
            if has_line_of_sight(player_pos, enemy_pos, &walls) {
                // Fire at the target.
                let _ = self.player_fire(enemy_pos);
                // Close in if far, otherwise hold position and keep firing.
                if player_pos.distance(enemy_pos) > 250.0 {
                    move_goal = Some(enemy_pos);
                } else {
                    self.set_player_velocity(Vec2::zero());
                }
            } else {
                // No line of sight: path toward the enemy.
                move_goal = Some(enemy_pos);
            }
        }

        if let Some(goal) = move_goal {
            self.navigate_toward(goal);
        }

        // Detect wedging and (next tick) strafe out of it.
        self.update_stuck_state(move_goal, dt);

        self.step(dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Health, Position, Radius, Rotation, Speed, Weapon};
    use crate::game::{spawn_enemy, spawn_player};

    const DT: f32 = 0.016; // 60 FPS

    #[test]
    fn test_sim_runs_many_frames_without_panicking() {
        // Fast-forward a full level for ~17 seconds of game time.
        let mut sim = Simulation::new(0);
        sim.run_frames(1000, DT);
        // Still structurally valid.
        assert!(sim.player().is_some());
    }

    #[test]
    fn test_sim_enemies_converge_on_player() {
        // With the player standing still, an enemy that can see them should
        // close the distance.
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(500.0, 500.0));
        let enemy = spawn_enemy(&mut world, Vec2::new(500.0, 100.0)); // clear line of sight
                                                                      // Face the enemy toward the player (downward) so it's in its vision cone.
        world.get_component_mut::<Rotation>(enemy).unwrap().angle = std::f32::consts::FRAC_PI_2;
        let mut sim = Simulation::from_world(world);

        let start = sim
            .world
            .get_component::<Position>(enemy)
            .unwrap()
            .distance_to(&Position::new(500.0, 500.0));

        sim.run_frames(120, DT); // ~2 seconds

        let end = sim
            .world
            .get_component::<Position>(enemy)
            .unwrap()
            .distance_to(&Position::new(500.0, 500.0));

        assert!(
            end < start,
            "enemy should have moved closer: {start} -> {end}"
        );
    }

    #[test]
    fn test_sim_scripted_player_clears_a_small_room() {
        // Build a tiny arena: player plus three enemies in a line, no walls.
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));
        let e1 = spawn_enemy(&mut world, Vec2::new(0.0, -60.0));
        let e2 = spawn_enemy(&mut world, Vec2::new(0.0, -120.0));
        let e3 = spawn_enemy(&mut world, Vec2::new(0.0, -180.0));
        let mut sim = Simulation::from_world(world);

        assert_eq!(sim.enemies_alive(), 3);

        // Fire straight up repeatedly; bullets fly and cull enemies over time.
        for target in [e1, e2, e3] {
            // Aim at each enemy in turn and shoot until it dies.
            for _ in 0..200 {
                if sim
                    .world
                    .get_component::<Health>(target)
                    .map(|h| h.is_dead())
                    .unwrap_or(true)
                {
                    break;
                }
                let tpos = *sim.world.get_component::<Position>(target).unwrap();
                sim.player_fire(Vec2::new(tpos.x, tpos.y));
                sim.step(DT);
            }
        }

        assert_eq!(
            sim.enemies_alive(),
            0,
            "player should have cleared the room"
        );
        assert!(sim.player_alive());
    }

    #[test]
    fn test_sim_throw_knocks_down_then_enemy_recovers() {
        // Player holding a weapon, one enemy directly to the right.
        let mut world = World::new();
        let player = world.spawn();
        world.add_component(player, Player);
        world.add_component(player, Position::new(0.0, 0.0));
        world.add_component(player, Velocity::zero());
        world.add_component(player, Speed::new(200.0));
        world.add_component(player, Radius::new(15.0));
        world.add_component(player, Rotation::new(0.0));
        world.add_component(player, Weapon::new(WeaponType::Pistol));

        let enemy = world.spawn();
        world.add_component(enemy, Enemy);
        world.add_component(enemy, Position::new(60.0, 0.0));
        world.add_component(enemy, Velocity::zero());
        world.add_component(enemy, Speed::new(100.0));
        world.add_component(enemy, Health::new(50));
        world.add_component(enemy, Radius::new(12.0));
        world.add_component(enemy, Rotation::new(0.0));
        world.add_component(enemy, crate::components::AI::new());

        let mut sim = Simulation::from_world(world);

        // Throw the pistol at the enemy.
        assert!(sim.player_throw(Vec2::new(60.0, 0.0)));
        assert!(sim.world.get_component::<Weapon>(player).is_none()); // disarmed

        // Within a few frames it connects and stuns the enemy.
        let mut stunned = false;
        for _ in 0..15 {
            sim.step(DT);
            if sim.world.has_component::<crate::components::Stunned>(enemy) {
                stunned = true;
                break;
            }
        }
        assert!(stunned, "thrown weapon should knock the enemy down");

        // After the stun duration elapses, the enemy recovers.
        sim.run_frames(240, DT); // ~4 seconds > 3s stun
        assert!(!sim.world.has_component::<crate::components::Stunned>(enemy));

        // The thrown pistol is now a pickup the player can retrieve.
        assert!(!sim
            .world
            .query::<crate::components::WeaponPickup>()
            .is_empty());
    }
}
