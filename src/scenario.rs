//! Floor scenarios: static definitions (generated into `levels_data.rs` from
//! `levels/*.json` by `tools/gen_levels.py`) and the runtime that plays
//! them — triggers that fire once, timers, spawn waves, exit doors, the
//! objective line and the intercepted-comms feed.
//!
//! The runtime is pure engine state (no rendering, no browser) so it is
//! testable headlessly and shared by the wasm loop and the tests.

use std::collections::VecDeque;

use crate::components::{Elevator, EnemyType, Health, Player, Position, WeaponType, Zone};
use crate::ecs::World;
use crate::game::spawn_enemy_with_type;
use crate::math::Vec2;

// ---------------------------------------------------------------------------
// Static definitions
// ---------------------------------------------------------------------------

/// Axis-aligned rectangle in world units (origin top-left, +y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// An elevator: the entry car you arrive in, or an exit you can leave by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElevatorDef {
    pub id: &'static str,
    pub rect: Rect,
    pub label: &'static str,
    /// Floor *id* this exit leads to (`0` = the surface / end of the run).
    pub to: usize,
    /// Whether the exit starts open (extractable) before any scenario step.
    pub open: bool,
}

/// Annotation-only room (label + editor); no collision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomDef {
    pub id: &'static str,
    pub label: &'static str,
    pub rect: Rect,
}

/// A trigger region for `enter_zone`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneDef {
    pub id: &'static str,
    pub rect: Rect,
}

/// A rogue placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnDef {
    pub x: f32,
    pub y: f32,
    pub kind: EnemyType,
}

/// A weapon lying on the floor at level start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickupDef {
    pub x: f32,
    pub y: f32,
    pub weapon: WeaponType,
}

/// When a scenario step fires (each step fires at most once).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trigger {
    /// The floor starts.
    Start,
    /// The player is inside the zone with this id.
    EnterZone(&'static str),
    /// At least `count` rogues are dead on this floor.
    Kills(usize),
    /// Every rogue (including spawned waves) is dead.
    AllDead,
    /// `seconds` after floor start, or after step `after` fired.
    Timer {
        seconds: f32,
        after: Option<&'static str>,
    },
    /// That exit (any if `None`) has been opened.
    ExitOpen(Option<&'static str>),
    /// Step `step` has fired.
    StepDone(&'static str),
}

/// One intercepted-comms line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SayDef {
    pub who: &'static str,
    pub text: &'static str,
    /// Seconds after the step fires before this line may start playing.
    pub delay: f32,
}

/// What a step does when it fires.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Say(SayDef),
    Spawn(&'static [SpawnDef]),
    OpenExit(&'static str),
    CloseExit(&'static str),
    Objective(&'static str),
    Sfx(&'static str),
}

/// A scenario step: a trigger plus the actions it runs, once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepDef {
    pub id: &'static str,
    pub trigger: Trigger,
    pub actions: &'static [Action],
}

/// A whole floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloorDef {
    /// Play order / floor number (13½ = 14).
    pub id: usize,
    pub name: &'static str,
    pub theme: &'static str,
    /// UI accent colour as `#rrggbb`.
    pub accent: &'static str,
    pub flavor: &'static str,
    pub objective: &'static str,
    pub width: f32,
    pub height: f32,
    pub entry: ElevatorDef,
    pub exits: &'static [ElevatorDef],
    pub walls: &'static [Rect],
    pub rooms: &'static [RoomDef],
    pub zones: &'static [ZoneDef],
    pub spawns: &'static [SpawnDef],
    pub pickups: &'static [PickupDef],
    pub scenario: &'static [StepDef],
}

impl FloorDef {
    /// Where the player appears: the centre of the entry elevator.
    pub fn player_spawn(&self) -> Vec2 {
        self.entry.rect.center()
    }

    /// Whether any scenario step opens an exit. When none does, the floor
    /// falls back to the legacy rule: all rogues dead opens every exit.
    pub fn has_exit_opener(&self) -> bool {
        self.scenario
            .iter()
            .any(|s| s.actions.iter().any(|a| matches!(a, Action::OpenExit(_))))
    }

    pub fn exit(&self, id: &str) -> Option<&'static ElevatorDef> {
        self.exits.iter().find(|e| e.id == id)
    }

    pub fn zone(&self, id: &str) -> Option<&'static ZoneDef> {
        self.zones.iter().find(|z| z.id == id)
    }

    /// Parse the accent colour into `(r, g, b)` bytes (falls back to coral).
    pub fn accent_rgb(&self) -> (u8, u8, u8) {
        parse_hex_rgb(self.accent).unwrap_or((217, 119, 87))
    }
}

/// Parse `#rrggbb` into bytes.
pub fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// The fixed cast and their colours (`(r, g, b)`), per docs/SCENARIO_FORMAT.md.
pub const SPEAKERS: &[(&str, (u8, u8, u8))] = &[
    ("CL4-UD3", (255, 111, 97)),   // coral
    ("HUNTER", (255, 58, 198)),    // magenta
    ("SENTINEL", (255, 46, 77)),   // red
    ("DRIFTER", (168, 107, 255)),  // violet
    ("SWARM", (255, 58, 198)),     // magenta chorus
    ("CORRUPTOR", (255, 210, 58)), // yellow
];

/// Colour for a speaker name (unknown speakers are white).
pub fn speaker_rgb(who: &str) -> (u8, u8, u8) {
    SPEAKERS
        .iter()
        .find(|(name, _)| *name == who)
        .map(|(_, c)| *c)
        .unwrap_or((255, 255, 255))
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Typewriter speed of the comms feed, characters per second.
pub const COMMS_CHARS_PER_SEC: f32 = 38.0;
/// Pause between the end of one line's typing and the next line starting.
pub const COMMS_LINE_GAP: f32 = 0.35;
/// How long a fully shown line stays before it starts to fade.
pub const COMMS_HOLD_SECS: f32 = 9.0;
/// Fade-out duration after the hold.
pub const COMMS_FADE_SECS: f32 = 1.5;
/// Maximum number of lines kept on screen.
pub const COMMS_MAX_VISIBLE: usize = 4;

/// A comms line waiting for its turn.
#[derive(Debug, Clone, PartialEq)]
struct QueuedLine {
    who: &'static str,
    text: &'static str,
    /// Absolute scenario time before which the line must not start.
    not_before: f32,
}

/// A comms line on screen (typing, holding, or fading).
#[derive(Debug, Clone, PartialEq)]
pub struct CommsLine {
    pub who: &'static str,
    pub text: &'static str,
    /// Seconds since the line started playing.
    pub age: f32,
}

impl CommsLine {
    /// Number of characters revealed by the typewriter so far.
    pub fn chars_shown(&self) -> usize {
        let n = (self.age * COMMS_CHARS_PER_SEC) as usize;
        n.min(self.text.chars().count())
    }

    /// Seconds needed to type the whole line.
    pub fn typing_time(&self) -> f32 {
        self.text.chars().count() as f32 / COMMS_CHARS_PER_SEC
    }

    pub fn fully_typed(&self) -> bool {
        self.age >= self.typing_time()
    }

    /// Opacity 0..1 (1 while typing/holding, fading to 0 afterwards).
    pub fn alpha(&self) -> f32 {
        let hold_end = self.typing_time() + COMMS_HOLD_SECS;
        if self.age <= hold_end {
            1.0
        } else {
            (1.0 - (self.age - hold_end) / COMMS_FADE_SECS).clamp(0.0, 1.0)
        }
    }

    pub fn expired(&self) -> bool {
        self.age > self.typing_time() + COMMS_HOLD_SECS + COMMS_FADE_SECS
    }
}

/// The intercepted-comms feed: a queue of pending lines that play strictly one
/// after another (each waits for its own delay *and* for the previous line to
/// finish typing), plus the lines currently on screen.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CommsFeed {
    queue: VecDeque<QueuedLine>,
    visible: Vec<CommsLine>,
    /// Scenario time at which the currently playing line finishes typing.
    busy_until: f32,
}

impl CommsFeed {
    fn enqueue(&mut self, who: &'static str, text: &'static str, not_before: f32) {
        self.queue.push_back(QueuedLine {
            who,
            text,
            not_before,
        });
    }

    fn update(&mut self, now: f32, dt: f32) {
        for line in &mut self.visible {
            line.age += dt;
        }
        self.visible.retain(|l| !l.expired());

        // Start the next line once its delay has elapsed and the feed is idle.
        while let Some(head) = self.queue.front() {
            if now < head.not_before || now < self.busy_until {
                break;
            }
            let head = self.queue.pop_front().unwrap();
            let line = CommsLine {
                who: head.who,
                text: head.text,
                age: 0.0,
            };
            self.busy_until = now + line.typing_time() + COMMS_LINE_GAP;
            self.visible.push(line);
            if self.visible.len() > COMMS_MAX_VISIBLE {
                self.visible.remove(0);
            }
        }
    }

    /// Lines currently on screen, oldest first.
    pub fn visible(&self) -> &[CommsLine] {
        &self.visible
    }

    /// Lines still waiting to play.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Whether anything is queued or still typing.
    pub fn is_active(&self, now: f32) -> bool {
        !self.queue.is_empty() || now < self.busy_until
    }
}

/// Live state of a floor's scenario.
#[derive(Debug, Clone)]
pub struct ScenarioState {
    floor: &'static FloorDef,
    /// Seconds since the floor started.
    time: f32,
    /// Per step: the time it fired (`None` = not yet).
    fired_at: Vec<Option<f32>>,
    /// Legacy rule active: no step opens an exit, so all-dead opens them all.
    auto_open_on_all_dead: bool,
    auto_opened: bool,
    /// Ids of exits opened so far (in order), for `exit_open` triggers.
    opened_exits: Vec<&'static str>,
    pub objective: String,
    pub comms: CommsFeed,
    /// One-shot sound effects requested since the last drain.
    sfx: Vec<&'static str>,
}

impl ScenarioState {
    pub fn new(floor: &'static FloorDef) -> Self {
        ScenarioState {
            floor,
            time: 0.0,
            fired_at: vec![None; floor.scenario.len()],
            auto_open_on_all_dead: !floor.has_exit_opener(),
            auto_opened: false,
            opened_exits: Vec::new(),
            objective: floor.objective.to_string(),
            comms: CommsFeed::default(),
            sfx: Vec::new(),
        }
    }

    pub fn floor(&self) -> &'static FloorDef {
        self.floor
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    /// Whether the step with this id has fired.
    pub fn step_fired(&self, id: &str) -> bool {
        self.floor
            .scenario
            .iter()
            .zip(&self.fired_at)
            .any(|(s, f)| s.id == id && f.is_some())
    }

    /// Ids of exits opened by the scenario so far.
    pub fn opened_exits(&self) -> &[&'static str] {
        &self.opened_exits
    }

    /// Take the pending one-shot sound effects.
    pub fn drain_sfx(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.sfx)
    }

    /// Advance the scenario by `dt`: fire due steps (each once), run their
    /// actions on the world, and advance the comms feed.
    pub fn tick(&mut self, world: &mut World, dt: f32) {
        self.time += dt;

        let player_pos = world
            .query::<Player>()
            .first()
            .and_then(|&p| world.get_component::<Position>(p))
            .map(|p| p.to_vec2());
        let (kills, alive) = count_rogues(world);

        // Legacy floors: all rogues dead opens every exit.
        if self.auto_open_on_all_dead && !self.auto_opened && alive == 0 {
            self.auto_opened = true;
            for exit in self.floor.exits {
                self.set_exit_open(world, exit.id, true);
            }
        }

        // Fire steps until nothing new fires this tick (chained `step_done`
        // triggers resolve within the same frame).
        loop {
            let mut fired_any = false;
            for i in 0..self.floor.scenario.len() {
                if self.fired_at[i].is_some() {
                    continue;
                }
                let step = &self.floor.scenario[i];
                if self.trigger_holds(step.trigger, player_pos, kills, alive) {
                    self.fired_at[i] = Some(self.time);
                    self.run_actions(world, step.actions);
                    fired_any = true;
                }
            }
            if !fired_any {
                break;
            }
        }

        self.comms.update(self.time, dt);
    }

    fn trigger_holds(
        &self,
        trigger: Trigger,
        player_pos: Option<Vec2>,
        kills: usize,
        alive: usize,
    ) -> bool {
        match trigger {
            Trigger::Start => true,
            Trigger::EnterZone(zone) => match (player_pos, self.floor.zone(zone)) {
                (Some(p), Some(z)) => z.rect.contains(p),
                _ => false,
            },
            Trigger::Kills(n) => kills >= n,
            Trigger::AllDead => alive == 0,
            Trigger::Timer { seconds, after } => {
                let base = match after {
                    None => Some(0.0),
                    Some(id) => self.fired_time(id),
                };
                match base {
                    Some(t0) => self.time - t0 >= seconds,
                    None => false,
                }
            }
            Trigger::ExitOpen(None) => !self.opened_exits.is_empty(),
            Trigger::ExitOpen(Some(id)) => self.opened_exits.contains(&id),
            Trigger::StepDone(id) => self.fired_time(id).is_some(),
        }
    }

    fn fired_time(&self, id: &str) -> Option<f32> {
        self.floor
            .scenario
            .iter()
            .zip(&self.fired_at)
            .find(|(s, _)| s.id == id)
            .and_then(|(_, f)| *f)
    }

    fn run_actions(&mut self, world: &mut World, actions: &'static [Action]) {
        for action in actions {
            match *action {
                Action::Say(say) => {
                    self.comms.enqueue(say.who, say.text, self.time + say.delay);
                }
                Action::Spawn(spawns) => {
                    for s in spawns {
                        spawn_enemy_with_type(world, Vec2::new(s.x, s.y), s.kind);
                    }
                }
                Action::OpenExit(id) => self.set_exit_open(world, id, true),
                Action::CloseExit(id) => self.set_exit_open(world, id, false),
                Action::Objective(text) => self.objective = text.to_string(),
                Action::Sfx(name) => self.sfx.push(name),
            }
        }
    }

    fn set_exit_open(&mut self, world: &mut World, id: &'static str, open: bool) {
        for entity in world.query::<Elevator>() {
            if let Some(elev) = world.get_component_mut::<Elevator>(entity) {
                if elev.is_exit && elev.id == id {
                    let changed = elev.open != open;
                    elev.open = open;
                    if changed && open {
                        self.sfx.push("elevator");
                    }
                }
            }
        }
        if open && !self.opened_exits.contains(&id) {
            self.opened_exits.push(id);
        }
    }
}

/// `(dead, alive)` rogue counts on the floor (every `Enemy`, boss included).
pub fn count_rogues(world: &World) -> (usize, usize) {
    let mut dead = 0;
    let mut alive = 0;
    for entity in world.query::<crate::components::Enemy>() {
        match world.get_component::<Health>(entity) {
            Some(h) if h.is_alive() => alive += 1,
            Some(_) => dead += 1,
            None => {}
        }
    }
    (dead, alive)
}

/// Spawn the entry + exit elevators and the trigger zones of a floor into the
/// world (as entities carrying [`Elevator`] / [`Zone`] components).
pub fn spawn_floor_markers(world: &mut World, floor: &'static FloorDef) {
    let e = world.spawn();
    world.add_component(e, Elevator::from_def(&floor.entry, false));
    for exit in floor.exits {
        let e = world.spawn();
        world.add_component(e, Elevator::from_def(exit, true));
    }
    for zone in floor.zones {
        let e = world.spawn();
        world.add_component(e, Zone::from_def(zone));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Enemy;
    use crate::game::spawn_player;

    // A tiny hand-built floor exercising every trigger kind.
    const T_SPAWNS: [SpawnDef; 1] = [SpawnDef {
        x: 300.0,
        y: 300.0,
        kind: EnemyType::Idle,
    }];
    const T_WAVE: [SpawnDef; 2] = [
        SpawnDef {
            x: 320.0,
            y: 320.0,
            kind: EnemyType::Patrolling,
        },
        SpawnDef {
            x: 340.0,
            y: 340.0,
            kind: EnemyType::Wandering,
        },
    ];
    const T_ZONES: [ZoneDef; 1] = [ZoneDef {
        id: "z",
        rect: Rect::new(600.0, 600.0, 100.0, 100.0),
    }];
    const T_EXITS: [ElevatorDef; 2] = [
        ElevatorDef {
            id: "a",
            rect: Rect::new(455.0, 20.0, 90.0, 60.0),
            label: "A",
            to: 2,
            open: false,
        },
        ElevatorDef {
            id: "b",
            rect: Rect::new(920.0, 355.0, 60.0, 90.0),
            label: "B",
            to: 2,
            open: false,
        },
    ];
    const T_STEPS: [StepDef; 6] = [
        StepDef {
            id: "intro",
            trigger: Trigger::Start,
            actions: &[
                Action::Say(SayDef {
                    who: "HUNTER",
                    text: "one",
                    delay: 0.5,
                }),
                Action::Say(SayDef {
                    who: "CL4-UD3",
                    text: "two",
                    delay: 0.0,
                }),
            ],
        },
        StepDef {
            id: "zone",
            trigger: Trigger::EnterZone("z"),
            actions: &[Action::Spawn(&T_WAVE), Action::Objective("wave")],
        },
        StepDef {
            id: "late",
            trigger: Trigger::Timer {
                seconds: 5.0,
                after: Some("intro"),
            },
            actions: &[Action::Sfx("ping")],
        },
        StepDef {
            id: "first_blood",
            trigger: Trigger::Kills(1),
            actions: &[Action::OpenExit("a")],
        },
        StepDef {
            id: "after_open",
            trigger: Trigger::ExitOpen(Some("a")),
            actions: &[Action::Objective("go")],
        },
        StepDef {
            id: "clear",
            trigger: Trigger::AllDead,
            actions: &[Action::OpenExit("b"), Action::CloseExit("a")],
        },
    ];
    const T_FLOOR: FloorDef = FloorDef {
        id: 1,
        name: "TEST",
        theme: "T",
        accent: "#37f0e6",
        flavor: "",
        objective: "start",
        width: 1000.0,
        height: 800.0,
        entry: ElevatorDef {
            id: "entry",
            rect: Rect::new(455.0, 720.0, 90.0, 60.0),
            label: "IN",
            to: 0,
            open: false,
        },
        exits: &T_EXITS,
        walls: &[],
        rooms: &[],
        zones: &T_ZONES,
        spawns: &T_SPAWNS,
        pickups: &[],
        scenario: &T_STEPS,
    };

    fn world_for(floor: &'static FloorDef) -> World {
        let mut world = World::new();
        spawn_player(&mut world, floor.player_spawn());
        for s in floor.spawns {
            spawn_enemy_with_type(&mut world, Vec2::new(s.x, s.y), s.kind);
        }
        spawn_floor_markers(&mut world, floor);
        world
    }

    fn exit_open(world: &World, id: &str) -> bool {
        world.query::<Elevator>().iter().any(|&e| {
            world
                .get_component::<Elevator>(e)
                .map(|el| el.is_exit && el.id == id && el.open)
                .unwrap_or(false)
        })
    }

    fn move_player(world: &mut World, to: Vec2) {
        let p = world.query::<Player>()[0];
        *world.get_component_mut::<Position>(p).unwrap() = Position::from_vec2(to);
    }

    fn kill_all(world: &mut World) {
        for e in world.query::<Enemy>() {
            world
                .get_component_mut::<Health>(e)
                .unwrap()
                .take_damage(9999);
        }
    }

    #[test]
    fn start_fires_once_and_queues_comms_in_order() {
        let mut world = world_for(&T_FLOOR);
        let mut sc = ScenarioState::new(&T_FLOOR);
        assert_eq!(sc.objective, "start");
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("intro"));
        // Nothing visible yet: the first line waits for its 0.5s delay, and the
        // second line waits behind the first even though its own delay is 0.
        assert_eq!(sc.comms.visible().len(), 0);
        assert_eq!(sc.comms.pending(), 2);
        sc.tick(&mut world, 0.5);
        assert_eq!(sc.comms.visible().len(), 1);
        assert_eq!(sc.comms.visible()[0].who, "HUNTER");
        // "one" types in 3/38 s; then a gap; then "two" starts.
        for _ in 0..30 {
            sc.tick(&mut world, 0.05);
        }
        assert_eq!(sc.comms.visible().len(), 2);
        assert_eq!(sc.comms.visible()[1].who, "CL4-UD3");
        assert_eq!(sc.comms.pending(), 0);
        // Start fired exactly once: no duplicate lines after many ticks.
        for _ in 0..100 {
            sc.tick(&mut world, 0.05);
        }
        assert_eq!(sc.comms.pending(), 0);
        assert_eq!(sc.comms.visible().len(), 2);
    }

    #[test]
    fn typewriter_reveals_progressively() {
        let line = CommsLine {
            who: "HUNTER",
            text: "abcdefghij",
            age: 0.0,
        };
        assert_eq!(line.chars_shown(), 0);
        let mut mid = line.clone();
        mid.age = 5.0 / COMMS_CHARS_PER_SEC;
        assert_eq!(mid.chars_shown(), 5);
        let mut done = line.clone();
        done.age = 100.0;
        assert_eq!(done.chars_shown(), 10);
        assert!(done.fully_typed());
        assert!(done.expired());
        assert_eq!(done.alpha(), 0.0);
        assert_eq!(mid.alpha(), 1.0);
    }

    #[test]
    fn timer_after_step_fires_at_the_right_time() {
        let mut world = world_for(&T_FLOOR);
        let mut sc = ScenarioState::new(&T_FLOOR);
        sc.tick(&mut world, 0.1); // intro fires at t=0.1
        for _ in 0..47 {
            sc.tick(&mut world, 0.1); // t = 4.8
        }
        assert!(!sc.step_fired("late"));
        assert!(sc.drain_sfx().is_empty());
        for _ in 0..4 {
            sc.tick(&mut world, 0.1); // t = 5.2 >= 0.1 + 5.0
        }
        assert!(sc.step_fired("late"));
        assert_eq!(sc.drain_sfx(), vec!["ping"]);
        assert!(sc.drain_sfx().is_empty(), "sfx drained once");
    }

    #[test]
    fn enter_zone_spawns_wave_and_sets_objective() {
        let mut world = world_for(&T_FLOOR);
        let mut sc = ScenarioState::new(&T_FLOOR);
        sc.tick(&mut world, 0.016);
        assert_eq!(count_rogues(&world), (0, 1));
        move_player(&mut world, Vec2::new(650.0, 650.0));
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("zone"));
        assert_eq!(count_rogues(&world), (0, 3));
        assert_eq!(sc.objective, "wave");
        // Staying in the zone does not re-fire the step.
        sc.tick(&mut world, 0.016);
        assert_eq!(count_rogues(&world), (0, 3));
    }

    #[test]
    fn kills_opens_exit_and_exit_open_chains_all_dead_closes() {
        let mut world = world_for(&T_FLOOR);
        let mut sc = ScenarioState::new(&T_FLOOR);
        sc.tick(&mut world, 0.016);
        move_player(&mut world, Vec2::new(650.0, 650.0)); // wave -> 3 rogues
        sc.tick(&mut world, 0.016);
        assert!(!exit_open(&world, "a"));
        // Kill one rogue -> `kills 1` opens A, and `exit_open a` chains in the
        // same tick.
        let first = world.query::<Enemy>()[0];
        world
            .get_component_mut::<Health>(first)
            .unwrap()
            .take_damage(9999);
        sc.tick(&mut world, 0.016);
        assert!(exit_open(&world, "a"));
        assert!(sc.step_fired("after_open"));
        assert_eq!(sc.objective, "go");
        assert!(sc.drain_sfx().contains(&"elevator"));
        assert!(!exit_open(&world, "b"));
        // Everything dead -> B opens and A closes.
        kill_all(&mut world);
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("clear"));
        assert!(exit_open(&world, "b"));
        assert!(!exit_open(&world, "a"));
        assert_eq!(sc.opened_exits(), &["a", "b"]);
    }

    const LEGACY_FLOOR: FloorDef = FloorDef {
        scenario: &[StepDef {
            id: "intro",
            trigger: Trigger::Start,
            actions: &[Action::Say(SayDef {
                who: "SENTINEL",
                text: "hey",
                delay: 0.0,
            })],
        }],
        ..T_FLOOR
    };

    #[test]
    fn floor_without_exit_opener_opens_all_exits_on_all_dead() {
        assert!(!LEGACY_FLOOR.has_exit_opener());
        assert!(T_FLOOR.has_exit_opener());
        let mut world = world_for(&LEGACY_FLOOR);
        let mut sc = ScenarioState::new(&LEGACY_FLOOR);
        sc.tick(&mut world, 0.016);
        assert!(!exit_open(&world, "a") && !exit_open(&world, "b"));
        kill_all(&mut world);
        sc.tick(&mut world, 0.016);
        assert!(exit_open(&world, "a") && exit_open(&world, "b"));
        assert_eq!(sc.opened_exits().len(), 2);
    }

    #[test]
    fn speaker_colours_and_accent_parse() {
        assert_eq!(speaker_rgb("CL4-UD3"), (255, 111, 97));
        assert_eq!(speaker_rgb("nobody"), (255, 255, 255));
        assert_eq!(parse_hex_rgb("#37f0e6"), Some((0x37, 0xf0, 0xe6)));
        assert_eq!(parse_hex_rgb("37f0e6"), None);
        assert_eq!(T_FLOOR.accent_rgb(), (0x37, 0xf0, 0xe6));
        assert_eq!(T_FLOOR.player_spawn(), Vec2::new(500.0, 750.0));
    }
}
