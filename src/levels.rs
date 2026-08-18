// Data-driven level definitions.
//
// The floors live as JSON in `levels/*.json` (edited by the level
// editor); `tools/gen_levels.py` compiles them into `src/levels_data.rs`,
// which this module exposes. Each floor is a set of wall rectangles,
// hand-placed rogues, an ENTRY elevator (the player spawn), one or more EXIT
// elevators (extraction points) and a scenario (see `crate::scenario`).
//
// Invariants enforced by tests in this module:
//   * the player never spawns inside a wall,
//   * no enemy spawns inside a wall,
//   * no enemy spawns on top of the player,
//   * every exit leads to an existing floor (or `SURFACE_EXIT`, the surface),
//   * every zone / exit / step id a scenario references exists.
use crate::components::EnemyType;
use crate::levels_data::{FLOORS, FLOOR_COUNT};
use crate::math::Vec2;
use crate::scenario::FloorDef;

/// Number of selectable levels (the ground-level cold open, 13 floors, and
/// the hidden boss floor).
pub const LEVEL_COUNT: usize = FLOOR_COUNT;

/// The hidden final floor (0-based index) where the shoggoth boss waits. The
/// extraction elevator "jams" after floor 13 and drops the player here. It
/// is the last floor in play order.
pub const BOSS_LEVEL: usize = FLOOR_COUNT - 1;

/// Floor id of the ground-level cold open (the parking lot / main gate).
pub const GROUND_FLOOR_ID: usize = 0;

/// A wall rectangle: `(x, y, width, height)` with the origin at the top-left.
pub type WallDef = (f32, f32, f32, f32);

/// An enemy placement: `(x, y, type)`.
pub type EnemyDef = (f32, f32, EnemyType);

/// A full level layout (the legacy flat view of a [`FloorDef`]).
pub struct LevelDef {
    pub walls: Vec<WallDef>,
    pub enemies: Vec<EnemyDef>,
    /// Where the player starts: the centre of the entry elevator.
    pub player_spawn: Vec2,
}

/// The full floor definition for a level index (`0`-based). Out-of-range
/// indices fall back to the last floor.
pub fn floor_def(level: usize) -> &'static FloorDef {
    FLOORS[level.min(LEVEL_COUNT - 1)]
}

/// The level index (0-based) a floor *id* maps to, if it exists.
pub fn level_index_for_floor_id(id: usize) -> Option<usize> {
    FLOORS.iter().position(|f| f.id == id)
}

/// Build the (legacy, flat) layout for a level index (`0`-based). Out-of-range
/// indices fall back to the last level.
pub fn level_def(level: usize) -> LevelDef {
    let floor = floor_def(level);
    LevelDef {
        walls: floor.walls.iter().map(|r| (r.x, r.y, r.w, r.h)).collect(),
        enemies: floor.spawns.iter().map(|s| (s.x, s.y, s.kind)).collect(),
        player_spawn: floor.player_spawn(),
    }
}

/// Where the player starts on a level.
pub fn player_spawn(level: usize) -> Vec2 {
    floor_def(level).player_spawn()
}

/// Display name of a level ("FLOOR 13½" for the boss floor, "FLOOR 00" for
/// the ground-level cold open).
pub fn floor_title(level: usize) -> String {
    let id = floor_def(level).id;
    if level == BOSS_LEVEL {
        "FLOOR 13\u{00BD}".to_string()
    } else if id == GROUND_FLOOR_ID {
        "FLOOR 00".to_string()
    } else {
        format!("FLOOR {id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::circle_rect_collision;
    use crate::scenario::{Action, Trigger, SURFACE_EXIT};

    const PLAYER_RADIUS: f32 = 15.0;
    const ENEMY_RADIUS: f32 = 12.0;

    #[test]
    fn test_all_levels_have_enemies() {
        for level in 0..LEVEL_COUNT {
            let def = level_def(level);
            assert!(!def.enemies.is_empty(), "level {level} has no enemies");
        }
    }

    #[test]
    fn test_floor_ids_are_play_order() {
        // Floor 0 (the parking lot) first, then 1..13, then 13½ (id 14).
        for (i, f) in FLOORS.iter().enumerate() {
            assert_eq!(f.id, i, "floor at index {i} has id {}", f.id);
        }
        assert_eq!(FLOORS[0].id, GROUND_FLOOR_ID);
        assert_eq!(FLOORS[BOSS_LEVEL].id, 14);
        assert_eq!(level_index_for_floor_id(0), Some(0));
        assert_eq!(level_index_for_floor_id(14), Some(BOSS_LEVEL));
        assert_eq!(level_index_for_floor_id(99), None);
        assert_eq!(level_index_for_floor_id(SURFACE_EXIT), None);
    }

    #[test]
    fn test_floor_titles() {
        assert_eq!(floor_title(0), "FLOOR 00");
        assert_eq!(floor_title(level_index_for_floor_id(1).unwrap()), "FLOOR 1");
        assert_eq!(
            floor_title(level_index_for_floor_id(13).unwrap()),
            "FLOOR 13"
        );
        assert_eq!(floor_title(BOSS_LEVEL), "FLOOR 13\u{00BD}");
    }

    #[test]
    fn test_ground_floor_is_the_cold_open() {
        // Asphalt lot, a gate to arrive through, a door into the lobby, and a
        // passive crowd strolling to a zone near the doors.
        use crate::scenario::{ElevatorKind, Surface};
        let f = floor_def(0);
        assert_eq!(f.surface, Surface::Asphalt);
        assert_eq!(f.entry.kind, ElevatorKind::Gate);
        assert!(f
            .exits
            .iter()
            .all(|e| e.kind == ElevatorKind::Door && e.to == 1));
        assert!(f.spawns.iter().all(|s| s.passive));
        // Every stroll target is a real zone, and most of the crowd strolls.
        assert!(f
            .spawns
            .iter()
            .filter_map(|s| s.walk_to)
            .all(|z| f.zone(z).is_some()));
        assert!(f.spawns.iter().filter(|s| s.walk_to.is_some()).count() >= 2);
        // The gate-scan and forecourt beats are `talk` conversations now
        // (dialogue mode, player-paced) rather than timed holds.
        assert!(
            f.scenario
                .iter()
                .filter(|s| s.actions.iter().any(|a| matches!(a, Action::Talk(_))))
                .count()
                >= 2
        );
    }

    #[test]
    fn test_player_never_spawns_in_a_wall() {
        // The player must have room to step out of the entry car.
        let clearance = PLAYER_RADIUS + 10.0;
        let mut violations = Vec::new();
        for level in 0..LEVEL_COUNT {
            let def = level_def(level);
            for (i, &(wx, wy, ww, wh)) in def.walls.iter().enumerate() {
                if circle_rect_collision(def.player_spawn, clearance, wx, wy, ww, wh) {
                    violations.push(format!(
                        "level {level}: player spawn overlaps wall {i} ({wx},{wy},{ww},{wh})"
                    ));
                }
            }
        }
        assert!(violations.is_empty(), "{}", violations.join("\n"));
    }

    #[test]
    fn test_no_enemy_spawns_in_a_wall() {
        let mut violations = Vec::new();
        for level in 0..LEVEL_COUNT {
            let floor = floor_def(level);
            let mut spawns: Vec<(f32, f32)> = floor.spawns.iter().map(|s| (s.x, s.y)).collect();
            for step in floor.scenario {
                for action in step.actions {
                    if let Action::Spawn(wave) = action {
                        spawns.extend(wave.iter().map(|s| (s.x, s.y)));
                    }
                }
            }
            for (ex, ey) in spawns {
                let pos = Vec2::new(ex, ey);
                for w in floor.walls {
                    if circle_rect_collision(pos, ENEMY_RADIUS, w.x, w.y, w.w, w.h) {
                        violations.push(format!(
                            "level {level}: enemy at ({ex},{ey}) overlaps wall {w:?}"
                        ));
                    }
                }
            }
        }
        assert!(violations.is_empty(), "{}", violations.join("\n"));
    }

    #[test]
    fn test_no_enemy_spawns_on_the_player() {
        // Enemies should not start close enough to hit the player instantly.
        let min_distance = 100.0;
        let mut violations = Vec::new();
        for level in 0..LEVEL_COUNT {
            let def = level_def(level);
            for &(ex, ey, _) in &def.enemies {
                let dist = def.player_spawn.distance(Vec2::new(ex, ey));
                if dist < min_distance {
                    violations.push(format!(
                        "level {level}: enemy at ({ex},{ey}) is only {dist:.0}px from player spawn"
                    ));
                }
            }
        }
        assert!(violations.is_empty(), "{}", violations.join("\n"));
    }

    #[test]
    fn test_level_overflow_falls_back_to_last_floor() {
        // Selecting past the last level index should still yield a valid layout.
        let def = level_def(999);
        assert!(!def.walls.is_empty());
        assert!(!def.enemies.is_empty());
        assert_eq!(floor_def(999).id, floor_def(LEVEL_COUNT - 1).id);
    }

    #[test]
    fn test_every_floor_has_entry_and_exits_inside_the_floor() {
        for (i, f) in FLOORS.iter().enumerate() {
            assert!(!f.exits.is_empty(), "floor {i} has no exit");
            let inside = |r: &crate::scenario::Rect| {
                r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= f.width && r.y + r.h <= f.height
            };
            assert!(inside(&f.entry.rect), "floor {i}: entry outside the floor");
            for e in f.exits {
                assert!(
                    inside(&e.rect),
                    "floor {i}: exit {} outside the floor",
                    e.id
                );
            }
        }
    }

    #[test]
    fn test_levels_data_is_consistent() {
        // Mirrors tools/gen_levels.py's validation on the compiled data: every
        // exit leads somewhere real and every id a scenario references exists.
        let mut problems = Vec::new();
        for (i, f) in FLOORS.iter().enumerate() {
            for e in f.exits {
                if e.to != SURFACE_EXIT && level_index_for_floor_id(e.to).is_none() {
                    problems.push(format!(
                        "floor {i}: exit {} -> unknown floor {}",
                        e.id, e.to
                    ));
                }
            }
            let mut exit_ids: Vec<&str> = f.exits.iter().map(|e| e.id).collect();
            exit_ids.sort_unstable();
            exit_ids.dedup();
            if exit_ids.len() != f.exits.len() {
                problems.push(format!("floor {i}: duplicate exit ids"));
            }
            let mut step_ids: Vec<&str> = f.scenario.iter().map(|s| s.id).collect();
            step_ids.sort_unstable();
            step_ids.dedup();
            if step_ids.len() != f.scenario.len() {
                problems.push(format!("floor {i}: duplicate step ids"));
            }
            for s in f.scenario {
                match s.trigger {
                    Trigger::EnterZone { zone: z, .. } if f.zone(z).is_none() => {
                        problems.push(format!("floor {i}/{}: unknown zone {z}", s.id))
                    }
                    Trigger::Timer { after: Some(a), .. } | Trigger::StepDone(a)
                        if !f.scenario.iter().any(|o| o.id == a) =>
                    {
                        problems.push(format!("floor {i}/{}: unknown step {a}", s.id))
                    }
                    Trigger::ExitOpen(Some(e)) if f.exit(e).is_none() => {
                        problems.push(format!("floor {i}/{}: unknown exit {e}", s.id))
                    }
                    Trigger::Kills(0) => {
                        problems.push(format!("floor {i}/{}: kills 0", s.id));
                    }
                    _ => {}
                }
                for a in s.actions {
                    match a {
                        Action::OpenExit(e) | Action::CloseExit(e) if f.exit(e).is_none() => {
                            problems.push(format!("floor {i}/{}: unknown exit {e}", s.id))
                        }
                        Action::Say(say)
                            if crate::scenario::speaker_rgb(say.who) == (255, 255, 255) =>
                        {
                            problems
                                .push(format!("floor {i}/{}: unknown speaker {}", s.id, say.who))
                        }
                        _ => {}
                    }
                }
            }
            // Every floor gets at least an intro line and a way to open an exit
            // (an explicit opener or the all-dead fallback, which always works).
            let has_start = f.scenario.iter().any(|s| s.trigger == Trigger::Start);
            if !has_start {
                problems.push(format!("floor {i}: no start step"));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn test_floor_13_jams_into_the_boss_floor() {
        let f13 = floor_def(level_index_for_floor_id(13).unwrap());
        assert_eq!(f13.id, 13);
        assert!(f13.exits.iter().all(|e| e.to == 14));
        // The boss floor's exit is the surface (`"to": "surface"`), and it is
        // the only one.
        assert!(floor_def(BOSS_LEVEL)
            .exits
            .iter()
            .all(|e| e.to == SURFACE_EXIT));
        for (i, f) in FLOORS.iter().enumerate() {
            if i != BOSS_LEVEL {
                assert!(
                    f.exits.iter().all(|e| e.to != SURFACE_EXIT),
                    "floor {i} exits to the surface"
                );
            }
        }
    }
}
