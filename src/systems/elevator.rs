//! Elevator extraction: the player leaves a floor by standing inside an OPEN
//! exit elevator for [`EXTRACT_DWELL_SECS`]. Kill-all is no longer the win
//! condition; scenarios (or the legacy all-dead fallback) open the doors.

use crate::components::{Elevator, Health, Player, Position};
use crate::ecs::{System, World};

/// Seconds the player must stand inside an open exit to extract.
pub const EXTRACT_DWELL_SECS: f32 = 0.6;

/// Tracks how long the (alive) player has been standing in each open exit.
pub struct ElevatorSystem;

impl System for ElevatorSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let player_pos = world.query::<Player>().first().and_then(|&p| {
            let alive = world
                .get_component::<Health>(p)
                .map(|h| h.is_alive())
                .unwrap_or(false);
            if alive {
                world.get_component::<Position>(p).map(|pos| pos.to_vec2())
            } else {
                None
            }
        });

        for entity in world.query::<Elevator>() {
            if let Some(elev) = world.get_component_mut::<Elevator>(entity) {
                let inside = elev.is_exit
                    && elev.open
                    && player_pos.map(|p| elev.contains(p)).unwrap_or(false);
                if inside {
                    elev.dwell += dt;
                } else {
                    elev.dwell = 0.0;
                }
            }
        }
    }
}

impl ElevatorSystem {
    /// The exit the player has fully extracted through, if any: returns its
    /// `to` floor id (`0` = surface).
    pub fn extraction(world: &World) -> Option<usize> {
        world
            .query::<Elevator>()
            .into_iter()
            .filter_map(|e| world.get_component::<Elevator>(e))
            .find(|e| e.is_exit && e.open && e.dwell >= EXTRACT_DWELL_SECS)
            .map(|e| e.to)
    }

    /// The exit the player is currently standing in (open or not).
    pub fn player_exit(world: &World) -> Option<Elevator> {
        let p = world.query::<Player>().into_iter().next()?;
        let pos = world.get_component::<Position>(p)?.to_vec2();
        world
            .query::<Elevator>()
            .into_iter()
            .filter_map(|e| world.get_component::<Elevator>(e))
            .find(|e| e.is_exit && e.contains(pos))
            .copied()
    }

    /// Whether any exit on the floor is open.
    pub fn any_exit_open(world: &World) -> bool {
        world
            .query::<Elevator>()
            .into_iter()
            .filter_map(|e| world.get_component::<Elevator>(e))
            .any(|e| e.is_exit && e.open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::spawn_player;
    use crate::math::Vec2;

    fn exit(world: &mut World, open: bool) -> crate::ecs::Entity {
        let e = world.spawn();
        world.add_component(
            e,
            Elevator {
                id: "lift",
                label: "LIFT",
                x: 455.0,
                y: 20.0,
                w: 90.0,
                h: 60.0,
                is_exit: true,
                open,
                to: 2,
                dwell: 0.0,
            },
        );
        e
    }

    #[test]
    fn closed_exit_never_extracts() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(500.0, 50.0));
        exit(&mut world, false);
        let mut sys = ElevatorSystem;
        for _ in 0..120 {
            sys.run(&mut world, 1.0 / 60.0);
        }
        assert_eq!(ElevatorSystem::extraction(&world), None);
        assert!(ElevatorSystem::player_exit(&world).is_some());
        assert!(!ElevatorSystem::any_exit_open(&world));
    }

    #[test]
    fn open_exit_extracts_after_dwell() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(500.0, 50.0));
        exit(&mut world, true);
        let mut sys = ElevatorSystem;
        for _ in 0..30 {
            sys.run(&mut world, 1.0 / 60.0); // 0.5s
        }
        assert_eq!(ElevatorSystem::extraction(&world), None);
        for _ in 0..8 {
            sys.run(&mut world, 1.0 / 60.0); // > 0.6s
        }
        assert_eq!(ElevatorSystem::extraction(&world), Some(2));
    }

    #[test]
    fn leaving_the_car_resets_the_dwell() {
        let mut world = World::new();
        let p = spawn_player(&mut world, Vec2::new(500.0, 50.0));
        exit(&mut world, true);
        let mut sys = ElevatorSystem;
        for _ in 0..30 {
            sys.run(&mut world, 1.0 / 60.0);
        }
        *world.get_component_mut::<Position>(p).unwrap() =
            crate::components::Position::new(500.0, 300.0);
        sys.run(&mut world, 1.0 / 60.0);
        for _ in 0..30 {
            sys.run(&mut world, 1.0 / 60.0);
        }
        assert_eq!(ElevatorSystem::extraction(&world), None);
    }

    #[test]
    fn dead_player_does_not_extract() {
        let mut world = World::new();
        let p = spawn_player(&mut world, Vec2::new(500.0, 50.0));
        exit(&mut world, true);
        world
            .get_component_mut::<Health>(p)
            .unwrap()
            .take_damage(1000);
        let mut sys = ElevatorSystem;
        for _ in 0..120 {
            sys.run(&mut world, 1.0 / 60.0);
        }
        assert_eq!(ElevatorSystem::extraction(&world), None);
    }
}
