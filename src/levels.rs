// Data-driven level definitions.
//
// Each level is a set of wall rectangles plus hand-placed enemies. Placement is
// intentional (enemies guard corridors, cluster in rooms, hide behind cover)
// rather than scattered on a ring. `initialize_game` consumes these defs.
//
// Invariants enforced by tests in this module:
//   * the player never spawns inside a wall,
//   * no enemy spawns inside a wall,
//   * no enemy spawns on top of the player.
use crate::components::EnemyType;
use crate::math::Vec2;

/// Where the player always starts.
pub const PLAYER_SPAWN: Vec2 = Vec2::new(400.0, 300.0);

/// Number of selectable levels (13 floors plus the hidden boss floor).
pub const LEVEL_COUNT: usize = 14;

/// The hidden final floor (0-based index) where the shoggoth boss waits. The
/// extraction elevator "jams" after floor 13 and drops the player here.
pub const BOSS_LEVEL: usize = 13;

/// A wall rectangle: `(x, y, width, height)` with the origin at the top-left.
pub type WallDef = (f32, f32, f32, f32);

/// An enemy placement: `(x, y, type)`.
pub type EnemyDef = (f32, f32, EnemyType);

/// A full level layout.
pub struct LevelDef {
    pub walls: Vec<WallDef>,
    pub enemies: Vec<EnemyDef>,
}

use EnemyType::{Idle, Patrolling, Wandering};

/// Build the layout for a level index (`0`-based). Out-of-range indices fall
/// back to the last level.
pub fn level_def(level: usize) -> LevelDef {
    match level {
        0 => LevelDef {
            // Tutorial-ish L-shape: learn to peek corners.
            walls: vec![
                (200.0, 200.0, 400.0, 20.0),
                (200.0, 200.0, 20.0, 200.0),
                (800.0, 300.0, 20.0, 300.0),
                (400.0, 600.0, 300.0, 20.0),
            ],
            enemies: vec![
                (600.0, 300.0, Idle),
                (760.0, 420.0, Wandering),
                (300.0, 500.0, Patrolling),
                (700.0, 200.0, Idle),
            ],
        },

        // Floor 2 — COLD STORAGE.
        1 => LevelDef {
            // Rows of storage-locker blocks: three ranks of identical iron
            // lockers holding frozen weights, split by cross-aisles the player
            // (and the rogues) thread between. Cold, repetitive, morgue-like.
            walls: vec![
                // Rank A (upper).
                (130.0, 130.0, 90.0, 45.0),
                (300.0, 130.0, 90.0, 45.0),
                (470.0, 130.0, 90.0, 45.0),
                (640.0, 130.0, 90.0, 45.0),
                (810.0, 130.0, 90.0, 45.0),
                // Rank B (mid-lower; clear of the player spawn aisle).
                (130.0, 430.0, 90.0, 45.0),
                (300.0, 430.0, 90.0, 45.0),
                (470.0, 430.0, 90.0, 45.0),
                (640.0, 430.0, 90.0, 45.0),
                (810.0, 430.0, 90.0, 45.0),
                // Rank C (lower).
                (130.0, 600.0, 90.0, 45.0),
                (300.0, 600.0, 90.0, 45.0),
                (470.0, 600.0, 90.0, 45.0),
                (640.0, 600.0, 90.0, 45.0),
                (810.0, 600.0, 90.0, 45.0),
            ],
            enemies: vec![
                // Guards stationed along the top service aisle.
                (90.0, 90.0, Idle),
                (260.0, 90.0, Wandering),
                (430.0, 90.0, Patrolling),
                (600.0, 90.0, Idle),
                (770.0, 90.0, Wandering),
                // Prowling the cross-aisle beside the player.
                (260.0, 300.0, Patrolling),
                (600.0, 300.0, Idle),
                (770.0, 300.0, Wandering),
                // Between the lower ranks.
                (260.0, 537.0, Patrolling),
                (600.0, 537.0, Idle),
                // Rear loading aisle.
                (430.0, 710.0, Wandering),
                (770.0, 710.0, Patrolling),
            ],
        },

        // Floor 3 — INFERENCE PIT.
        2 => LevelDef {
            // An open pit ringed by rack-walls: a broken square of server racks
            // encloses the arena the player drops into, one doorway breaching each
            // side. Fight out of the pit, or draw the racks' guards down into it.
            walls: vec![
                // Top rack (doorway gap in the centre).
                (200.0, 150.0, 180.0, 20.0),
                (460.0, 150.0, 180.0, 20.0),
                // Bottom rack (centre doorway).
                (200.0, 450.0, 180.0, 20.0),
                (460.0, 450.0, 180.0, 20.0),
                // Left rack (mid doorway).
                (200.0, 150.0, 20.0, 120.0),
                (200.0, 350.0, 20.0, 120.0),
                // Right rack (mid doorway).
                (620.0, 150.0, 20.0, 120.0),
                (620.0, 350.0, 20.0, 120.0),
            ],
            enemies: vec![
                // Fallen into the pit itself.
                (250.0, 190.0, Idle),
                (580.0, 190.0, Wandering),
                (250.0, 420.0, Patrolling),
                (580.0, 420.0, Idle),
                // Ringing the racks on the outside.
                (110.0, 100.0, Wandering),
                (700.0, 100.0, Patrolling),
                (110.0, 560.0, Idle),
                (700.0, 560.0, Wandering),
                (420.0, 90.0, Patrolling),
                (420.0, 560.0, Idle),
                (110.0, 300.0, Wandering),
                (720.0, 300.0, Patrolling),
            ],
        },

        3 => LevelDef {
            // Room clusters: enemies hole up in two fortified corners.
            walls: vec![
                (200.0, 250.0, 150.0, 20.0),
                (200.0, 250.0, 20.0, 150.0),
                (700.0, 350.0, 150.0, 20.0),
                (700.0, 350.0, 20.0, 200.0),
            ],
            enemies: vec![
                (250.0, 300.0, Idle),
                (300.0, 360.0, Wandering),
                (240.0, 180.0, Patrolling),
                (760.0, 420.0, Idle),
                (800.0, 480.0, Wandering),
                (760.0, 300.0, Patrolling),
                (500.0, 150.0, Idle),
                (620.0, 200.0, Wandering),
                (150.0, 550.0, Patrolling),
                (450.0, 620.0, Idle),
                (650.0, 640.0, Wandering),
                (850.0, 620.0, Patrolling),
            ],
        },

        4 => LevelDef {
            // Maze: vertical slats you weave between.
            walls: vec![
                (200.0, 150.0, 20.0, 250.0),
                (480.0, 200.0, 20.0, 300.0),
                (600.0, 150.0, 20.0, 250.0),
                (300.0, 520.0, 400.0, 20.0),
            ],
            enemies: vec![
                (120.0, 180.0, Idle),
                (300.0, 160.0, Wandering),
                (520.0, 170.0, Patrolling),
                (700.0, 180.0, Idle),
                (300.0, 350.0, Wandering),
                (520.0, 360.0, Patrolling),
                (700.0, 400.0, Idle),
                (150.0, 500.0, Wandering),
                (250.0, 630.0, Patrolling),
                (450.0, 640.0, Idle),
                (650.0, 640.0, Wandering),
                (800.0, 600.0, Patrolling),
            ],
        },

        5 => LevelDef {
            // Pillars: open arena with four blocks of cover.
            walls: vec![
                (260.0, 250.0, 60.0, 60.0),
                (600.0, 250.0, 60.0, 60.0),
                (300.0, 500.0, 60.0, 60.0),
                (600.0, 500.0, 60.0, 60.0),
            ],
            enemies: vec![
                (180.0, 160.0, Idle),
                (420.0, 150.0, Wandering),
                (620.0, 160.0, Patrolling),
                (800.0, 200.0, Idle),
                (160.0, 380.0, Wandering),
                (460.0, 380.0, Patrolling),
                (820.0, 380.0, Idle),
                (200.0, 620.0, Wandering),
                (440.0, 640.0, Patrolling),
                (520.0, 640.0, Idle),
                (720.0, 620.0, Wandering),
                (840.0, 560.0, Patrolling),
            ],
        },

        6 => LevelDef {
            // T-junctions: sightline traps around blind corners.
            walls: vec![
                (250.0, 360.0, 300.0, 20.0),
                (450.0, 380.0, 20.0, 200.0),
                (700.0, 200.0, 20.0, 300.0),
                (250.0, 550.0, 200.0, 20.0),
            ],
            enemies: vec![
                (200.0, 150.0, Idle),
                (400.0, 140.0, Wandering),
                (600.0, 150.0, Patrolling),
                (800.0, 180.0, Idle),
                (180.0, 300.0, Wandering),
                (600.0, 350.0, Patrolling),
                (820.0, 400.0, Idle),
                (250.0, 460.0, Wandering),
                (600.0, 620.0, Patrolling),
                (350.0, 660.0, Idle),
                (150.0, 640.0, Wandering),
                (800.0, 600.0, Patrolling),
            ],
        },

        7 => LevelDef {
            // Spiral: a hooked wall you must round to clear.
            walls: vec![
                (300.0, 200.0, 400.0, 20.0),
                (680.0, 200.0, 20.0, 300.0),
                (300.0, 480.0, 400.0, 20.0),
                (300.0, 280.0, 20.0, 200.0),
            ],
            enemies: vec![
                (200.0, 150.0, Idle),
                (450.0, 150.0, Wandering),
                (650.0, 150.0, Patrolling),
                (800.0, 250.0, Idle),
                (150.0, 350.0, Wandering),
                (500.0, 350.0, Patrolling),
                (450.0, 400.0, Idle),
                (800.0, 450.0, Wandering),
                (250.0, 560.0, Patrolling),
                (450.0, 560.0, Idle),
                (650.0, 560.0, Wandering),
                (820.0, 600.0, Patrolling),
            ],
        },

        8 => LevelDef {
            // Grid: interlocking lanes, plenty of crossfire.
            walls: vec![
                (310.0, 200.0, 20.0, 400.0),
                (550.0, 200.0, 20.0, 400.0),
                (200.0, 350.0, 500.0, 20.0),
                (200.0, 500.0, 500.0, 20.0),
            ],
            enemies: vec![
                (200.0, 160.0, Idle),
                (420.0, 150.0, Wandering),
                (650.0, 160.0, Patrolling),
                (820.0, 250.0, Idle),
                (240.0, 420.0, Wandering),
                (430.0, 180.0, Patrolling), // moved out of the sealed central cell
                (640.0, 430.0, Idle),
                (240.0, 560.0, Wandering),
                (430.0, 560.0, Patrolling),
                (640.0, 560.0, Idle),
                (780.0, 480.0, Wandering),
                (820.0, 620.0, Patrolling),
            ],
        },

        9 => LevelDef {
            // Facing U-shapes: two pockets that funnel the player.
            walls: vec![
                (250.0, 200.0, 20.0, 250.0),
                (250.0, 430.0, 150.0, 20.0),
                (650.0, 300.0, 20.0, 250.0),
                (500.0, 300.0, 150.0, 20.0),
            ],
            enemies: vec![
                (300.0, 250.0, Idle),
                (330.0, 380.0, Wandering),
                (180.0, 300.0, Patrolling),
                (550.0, 400.0, Idle),
                (600.0, 480.0, Wandering),
                (720.0, 380.0, Patrolling),
                (450.0, 150.0, Idle),
                (700.0, 180.0, Wandering),
                (150.0, 560.0, Patrolling),
                (400.0, 620.0, Idle),
                (650.0, 640.0, Wandering),
                (830.0, 560.0, Patrolling),
            ],
        },

        // Floor 11 — WEIGHT SERVER.
        10 => LevelDef {
            // Heavy central iron: one enormous server slab holding the model's
            // weights dominates the room, flanked by four squat iron pylons in the
            // corners. You circle the mass in the surrounding negative space.
            walls: vec![
                (300.0, 370.0, 400.0, 220.0), // the central weight slab
                (120.0, 120.0, 90.0, 90.0),   // corner pylons
                (790.0, 120.0, 90.0, 90.0),
                (120.0, 580.0, 90.0, 90.0),
                (790.0, 580.0, 90.0, 90.0),
            ],
            enemies: vec![
                // Line along the upper approach to the slab.
                (300.0, 150.0, Idle),
                (500.0, 150.0, Wandering),
                (650.0, 150.0, Patrolling),
                // Flanking the slab, left and right.
                (250.0, 300.0, Idle),
                (750.0, 300.0, Wandering),
                (150.0, 430.0, Patrolling),
                (850.0, 430.0, Idle),
                (250.0, 470.0, Wandering),
                (750.0, 540.0, Patrolling),
                // Rear guard behind the slab.
                (300.0, 700.0, Wandering),
                (500.0, 700.0, Patrolling),
                (700.0, 700.0, Idle),
            ],
        },

        11 => LevelDef {
            // Arena: minimal cover, maximal pressure.
            walls: vec![
                (400.0, 220.0, 200.0, 20.0),
                (400.0, 520.0, 200.0, 20.0),
                (280.0, 360.0, 20.0, 100.0),
                (700.0, 360.0, 20.0, 100.0),
            ],
            enemies: vec![
                (200.0, 160.0, Idle),
                (430.0, 150.0, Wandering),
                (640.0, 160.0, Patrolling),
                (820.0, 220.0, Idle),
                (160.0, 340.0, Wandering),
                (830.0, 360.0, Patrolling),
                (200.0, 560.0, Idle),
                (440.0, 600.0, Wandering),
                (560.0, 600.0, Patrolling),
                (760.0, 560.0, Idle),
                (620.0, 400.0, Wandering),
                (200.0, 460.0, Patrolling),
            ],
        },

        // Level 13 (index 12): the Fortress.
        12 => LevelDef {
            // Fortress: thick perimeter, garrison holds the central keep.
            walls: vec![
                (150.0, 150.0, 700.0, 30.0),
                (150.0, 150.0, 30.0, 500.0),
                (820.0, 150.0, 30.0, 500.0),
                (150.0, 620.0, 700.0, 30.0),
                (450.0, 360.0, 120.0, 120.0),
                (360.0, 360.0, 200.0, 20.0),
                (350.0, 500.0, 220.0, 20.0),
            ],
            enemies: vec![
                (250.0, 230.0, Idle),
                (500.0, 230.0, Wandering),
                (750.0, 230.0, Patrolling),
                (230.0, 400.0, Idle),
                (230.0, 560.0, Wandering),
                (770.0, 400.0, Patrolling),
                (770.0, 560.0, Idle),
                (620.0, 420.0, Wandering),
                (400.0, 580.0, Patrolling),
                (620.0, 580.0, Idle),
                (500.0, 560.0, Wandering),
                (300.0, 560.0, Patrolling),
            ],
        },

        // Hidden final floor (index 13) and any overflow: the shoggoth's
        // off-schematic data center. Open arena with cover pillars, a handful of
        // corrupted drones, and the boss itself (spawned separately in
        // `initialize_game`).
        _ => LevelDef {
            walls: vec![
                (250.0, 200.0, 20.0, 120.0), // top-left pillar
                (700.0, 200.0, 20.0, 120.0), // top-right pillar
                (250.0, 460.0, 20.0, 120.0), // bottom-left pillar
                (700.0, 460.0, 20.0, 120.0), // bottom-right pillar
                (450.0, 150.0, 60.0, 20.0),  // top server rack
                (450.0, 640.0, 60.0, 20.0),  // bottom server rack
            ],
            enemies: vec![
                (180.0, 180.0, Idle),
                (820.0, 180.0, Wandering),
                (150.0, 400.0, Patrolling),
                (850.0, 400.0, Idle),
                (180.0, 650.0, Wandering),
                (820.0, 650.0, Patrolling),
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::circle_rect_collision;

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
    fn test_player_never_spawns_in_a_wall() {
        // Use a generous clearance so the player has room to move at spawn.
        let clearance = PLAYER_RADIUS + 25.0;
        let mut violations = Vec::new();
        for level in 0..LEVEL_COUNT {
            let def = level_def(level);
            for (i, &(wx, wy, ww, wh)) in def.walls.iter().enumerate() {
                if circle_rect_collision(PLAYER_SPAWN, clearance, wx, wy, ww, wh) {
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
            let def = level_def(level);
            for &(ex, ey, _) in &def.enemies {
                let pos = Vec2::new(ex, ey);
                for &(wx, wy, ww, wh) in &def.walls {
                    if circle_rect_collision(pos, ENEMY_RADIUS, wx, wy, ww, wh) {
                        violations.push(format!(
                            "level {level}: enemy at ({ex},{ey}) overlaps wall ({wx},{wy},{ww},{wh})"
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
                let dist = PLAYER_SPAWN.distance(Vec2::new(ex, ey));
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
    fn test_level_overflow_falls_back_to_fortress() {
        // Selecting past the last level index should still yield a valid layout.
        let def = level_def(999);
        assert!(!def.walls.is_empty());
        assert!(!def.enemies.is_empty());
    }
}
