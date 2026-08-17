//! Floor scenarios: static definitions (generated into `levels_data.rs` from
//! `levels/*.json` by `tools/gen_levels.py`) and the runtime that plays
//! them — triggers that fire once, timers, spawn waves, exit doors, the
//! objective line and the intercepted-comms feed.
//!
//! The runtime is pure engine state (no rendering, no browser) so it is
//! testable headlessly and shared by the wasm loop and the tests.

use std::collections::VecDeque;

use crate::components::{Boss, Elevator, EnemyType, Health, Player, Position, WeaponType, Zone};
use crate::ecs::World;
use crate::game::spawn_enemy_with_type;
use crate::math::Vec2;
use crate::systems::elevator::ElevatorSystem;

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

/// `ElevatorDef::to` value of the exit that ends the run (the surface):
/// `"to": "surface"` in the JSON. Never a real floor id.
pub const SURFACE_EXIT: usize = usize::MAX;

/// How a portal (entry or exit) is drawn: an elevator car, a doorway whose
/// two leaves slide apart when open, or an open gateway with scanner posts
/// (the parking lot's main gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatorKind {
    Lift,
    Door,
    Gate,
}

impl ElevatorKind {
    /// Parse the JSON `kind` (`lift` | `door` | `gate`); unknown = `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "lift" => Some(ElevatorKind::Lift),
            "door" => Some(ElevatorKind::Door),
            "gate" => Some(ElevatorKind::Gate),
            _ => None,
        }
    }
}

/// The floor's ground rendering (`"surface"` in the JSON; default checker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Checker,
    Asphalt,
    Marble,
    Concrete,
    Grating,
}

impl Surface {
    /// Parse the JSON `surface` value; unknown = `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "checker" => Some(Surface::Checker),
            "asphalt" => Some(Surface::Asphalt),
            "marble" => Some(Surface::Marble),
            "concrete" => Some(Surface::Concrete),
            "grating" => Some(Surface::Grating),
            _ => None,
        }
    }
}

/// An elevator: the entry car you arrive in, or an exit you can leave by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElevatorDef {
    pub id: &'static str,
    pub rect: Rect,
    pub label: &'static str,
    /// Floor *id* this exit leads to ([`SURFACE_EXIT`] = the surface / end
    /// of the run).
    pub to: usize,
    /// Whether the exit starts open (extractable) before any scenario step.
    pub open: bool,
    /// Lift car / sliding door / open gate (rendering only).
    pub kind: ElevatorKind,
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
    /// Behaviour (and palette). For a passive bot this is its `look`.
    pub kind: EnemyType,
    /// `"type": "passive"`: a civilian bot (no vision, never attacks) until
    /// an `alert` action flips it to a hostile of `kind`.
    pub passive: bool,
    /// Passive only: zone id to stroll into.
    pub walk_to: Option<&'static str>,
    /// Passive only: heading in degrees to settle on once there.
    pub face: Option<f32>,
    /// Scenario `alert { "group": id }` group.
    pub group: Option<&'static str>,
}

impl SpawnDef {
    /// A plain hostile spawn (the common case).
    pub const fn hostile(x: f32, y: f32, kind: EnemyType) -> Self {
        SpawnDef {
            x,
            y,
            kind,
            passive: false,
            walk_to: None,
            face: None,
            group: None,
        }
    }
}

/// A weapon lying on the floor at level start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickupDef {
    pub x: f32,
    pub y: f32,
    pub weapon: WeaponType,
}

/// A placed prop (`crate::props`): decoration drawn on the floor under the
/// actors — no collision (phase 1). `rot` in degrees (clockwise, +y down),
/// `size` in world units (100 = the prop's design box).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropPlacement {
    /// Index into `props::PROP_NAMES` (the JSON holds the snake_case id).
    pub kind: usize,
    pub x: f32,
    pub y: f32,
    pub rot: f32,
    pub size: f32,
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
    /// The floor's boss (the `Boss` entity) is dead. Never fires on floors
    /// without a boss.
    BossDead,
    /// The player has extracted (stood the full dwell inside an open exit).
    /// The scenario keeps ticking through the completion card, so this is
    /// how a floor talks *after* the ride starts (13½'s uplink epilogue).
    Extracted,
}

impl Trigger {
    /// Whether the trigger reads the rogue counts (`kills` / `all_dead`).
    /// These are evaluated after the other triggers of a tick so same-tick
    /// spawns are counted first.
    pub fn is_count_based(&self) -> bool {
        matches!(self, Trigger::Kills(_) | Trigger::AllDead)
    }
}

/// One intercepted-comms line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SayDef {
    pub who: &'static str,
    pub text: &'static str,
    /// Seconds after the step fires before this line may start playing.
    pub delay: f32,
}

/// Which passive bots an `alert` action flips hostile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlertTarget {
    /// Every passive bot on the floor.
    All,
    /// The passive bots currently inside this zone.
    Zone(&'static str),
    /// The passive bots spawned with this `group`.
    Group(&'static str),
}

/// A `hold` action: lock the player's input for a while (the world keeps
/// running, comms keep playing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoldDef {
    /// Seconds to hold (with `until_comms_idle` this is the cap).
    pub seconds: f32,
    /// Optional dim centred caption ("SCANNING…").
    pub text: Option<&'static str>,
    /// Hold until the comms feed has nothing queued or typing (capped by
    /// `seconds`).
    pub until_comms_idle: bool,
}

/// A `look_at` action: ease the camera onto a world point for a while.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookAtDef {
    pub x: f32,
    pub y: f32,
    pub seconds: f32,
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
    /// Flip matching passive bots hostile.
    Alert(AlertTarget),
    /// Lock player input for a beat.
    Hold(HoldDef),
    /// Cinematic camera nudge.
    LookAt(LookAtDef),
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
    /// Placed props (decoration only).
    pub props: &'static [PropPlacement],
    pub scenario: &'static [StepDef],
    /// Ground rendering (default checker).
    pub surface: Surface,
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
    ("UPLINK", (200, 255, 222)),   // pale mint: the thread home, restored
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

/// The per-tick world facts a trigger is evaluated against.
struct TriggerCtx {
    player_pos: Option<Vec2>,
    kills: usize,
    alive: usize,
    boss_dead: bool,
    extracted: bool,
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
    /// The running `hold` (input lock), if any: see [`ScenarioState::hold`].
    hold: Option<HoldState>,
    /// The running `look_at`, if any: see [`ScenarioState::look_at`].
    look: Option<LookState>,
}

/// Live state of a `hold` action.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HoldState {
    def: HoldDef,
    /// Scenario time at which the hold ends (the cap when `until_comms_idle`).
    until: f32,
}

/// Live state of a `look_at` action.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LookState {
    def: LookAtDef,
    start: f32,
}

/// Longest a `hold_until_comms_idle` may lock the player, seconds.
pub const HOLD_COMMS_IDLE_CAP: f32 = 20.0;
/// Ease-in / ease-out length of a `look_at` camera move, seconds.
pub const LOOK_AT_EASE_SECS: f32 = 0.6;

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
            hold: None,
            look: None,
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

    /// Whether a `hold` is locking the player's input right now.
    pub fn hold_active(&self) -> bool {
        self.hold.is_some()
    }

    /// The caption of the running `hold`, if it has one.
    pub fn hold_caption(&self) -> Option<&'static str> {
        self.hold.and_then(|h| h.def.text)
    }

    /// The running `look_at`: the world point and its weight 0..1 (eased in
    /// over [`LOOK_AT_EASE_SECS`], held, eased out over the last
    /// [`LOOK_AT_EASE_SECS`]). `None` when no look is running.
    pub fn look_at(&self) -> Option<(Vec2, f32)> {
        let l = self.look?;
        let w = look_at_weight(self.time - l.start, l.def.seconds);
        Some((Vec2::new(l.def.x, l.def.y), w))
    }

    /// Advance the `hold` / `look_at` clocks (called from `tick`).
    fn tick_beats(&mut self) {
        if let Some(h) = self.hold {
            let comms_idle = !self.comms.is_active(self.time);
            let done = self.time >= h.until || (h.def.until_comms_idle && comms_idle);
            if done {
                self.hold = None;
            }
        }
        if let Some(l) = self.look {
            if self.time - l.start >= l.def.seconds {
                self.look = None;
            }
        }
    }

    /// Advance the scenario by `dt`: fire due steps (each once), run their
    /// actions on the world, and advance the comms feed.
    ///
    /// Within one tick, steps whose triggers depend on the rogue counts
    /// (`kills`, `all_dead`) are evaluated *after* the other steps of the
    /// same pass, and the counts are recomputed after every fired step, so a
    /// `spawn` in the same tick can never let `all_dead` slip through.
    pub fn tick(&mut self, world: &mut World, dt: f32) {
        self.time += dt;

        let player_pos = world
            .query::<Player>()
            .first()
            .and_then(|&p| world.get_component::<Position>(p))
            .map(|p| p.to_vec2());
        let mut counts = count_rogues(world);
        let boss_dead = any_boss_dead(world);
        let extracted = ElevatorSystem::extraction(world).is_some();

        // Fire steps until nothing new fires this tick (chained `step_done`
        // triggers resolve within the same frame).
        loop {
            let mut fired_any = false;
            // Pass 0: everything but the count-based triggers (may spawn);
            // pass 1: `kills` / `all_dead` against the fresh counts.
            for count_pass in [false, true] {
                for i in 0..self.floor.scenario.len() {
                    if self.fired_at[i].is_some() {
                        continue;
                    }
                    let step = &self.floor.scenario[i];
                    if step.trigger.is_count_based() != count_pass {
                        continue;
                    }
                    let (kills, alive) = counts;
                    let ctx = TriggerCtx {
                        player_pos,
                        kills,
                        alive,
                        boss_dead,
                        extracted,
                    };
                    if self.trigger_holds(step.trigger, &ctx) {
                        self.fired_at[i] = Some(self.time);
                        self.run_actions(world, step.actions);
                        counts = count_rogues(world);
                        fired_any = true;
                    }
                }
            }
            if !fired_any {
                break;
            }
        }

        // Legacy floors: all rogues dead opens every exit (checked against the
        // counts *after* this tick's spawns).
        if self.auto_open_on_all_dead && !self.auto_opened && counts.1 == 0 {
            self.auto_opened = true;
            for exit in self.floor.exits {
                self.set_exit_open(world, exit.id, true);
            }
        }

        self.comms.update(self.time, dt);
        self.tick_beats();
    }

    fn trigger_holds(&self, trigger: Trigger, ctx: &TriggerCtx) -> bool {
        match trigger {
            Trigger::Start => true,
            Trigger::EnterZone(zone) => match (ctx.player_pos, self.floor.zone(zone)) {
                (Some(p), Some(z)) => z.rect.contains(p),
                _ => false,
            },
            Trigger::Kills(n) => ctx.kills >= n,
            Trigger::AllDead => ctx.alive == 0,
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
            Trigger::BossDead => ctx.boss_dead,
            Trigger::Extracted => ctx.extracted,
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
                        spawn_from_def(world, s);
                    }
                }
                Action::OpenExit(id) => self.set_exit_open(world, id, true),
                Action::CloseExit(id) => self.set_exit_open(world, id, false),
                Action::Objective(text) => self.objective = text.to_string(),
                Action::Sfx(name) => self.sfx.push(name),
                Action::Alert(target) => {
                    crate::systems::passive::alert_passives(world, target);
                }
                Action::Hold(def) => {
                    let secs = if def.until_comms_idle {
                        def.seconds.min(HOLD_COMMS_IDLE_CAP)
                    } else {
                        def.seconds
                    };
                    self.hold = Some(HoldState {
                        def,
                        until: self.time + secs,
                    });
                }
                Action::LookAt(def) => {
                    self.look = Some(LookState {
                        def,
                        start: self.time,
                    });
                }
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

/// Weight 0..1 of a `look_at` that started `elapsed` seconds ago and lasts
/// `total`: eases in over [`LOOK_AT_EASE_SECS`], holds, eases out over the
/// last [`LOOK_AT_EASE_SECS`] (smoothstep both ways). Short looks scale the
/// ramps down so they still peak.
pub fn look_at_weight(elapsed: f32, total: f32) -> f32 {
    if total <= 0.0 || elapsed < 0.0 || elapsed >= total {
        return 0.0;
    }
    let ramp = LOOK_AT_EASE_SECS.min(total / 2.0);
    let t = if elapsed < ramp {
        elapsed / ramp
    } else if elapsed > total - ramp {
        (total - elapsed) / ramp
    } else {
        1.0
    };
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Spawn one placement: a hostile rogue of `kind`, or a passive bot when
/// `def.passive` (see `systems::passive`).
pub fn spawn_from_def(world: &mut World, def: &SpawnDef) -> crate::ecs::Entity {
    if def.passive {
        crate::systems::passive::spawn_passive(world, def)
    } else {
        spawn_enemy_with_type(world, Vec2::new(def.x, def.y), def.kind)
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

/// Whether the floor has a boss and it is dead (`boss_dead` trigger).
pub fn any_boss_dead(world: &World) -> bool {
    world.query::<Boss>().iter().any(|&e| {
        world
            .get_component::<Health>(e)
            .map(|h| h.is_dead())
            .unwrap_or(false)
    })
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
    use crate::components::{AIState, Enemy, AI};
    use crate::game::spawn_player;

    // A tiny hand-built floor exercising every trigger kind.
    const T_SPAWNS: [SpawnDef; 1] = [SpawnDef::hostile(300.0, 300.0, EnemyType::Idle)];
    const T_WAVE: [SpawnDef; 2] = [
        SpawnDef::hostile(320.0, 320.0, EnemyType::Patrolling),
        SpawnDef::hostile(340.0, 340.0, EnemyType::Wandering),
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
            kind: ElevatorKind::Lift,
        },
        ElevatorDef {
            id: "b",
            rect: Rect::new(920.0, 355.0, 60.0, 90.0),
            label: "B",
            to: 2,
            open: false,
            kind: ElevatorKind::Lift,
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
            to: SURFACE_EXIT,
            open: false,
            kind: ElevatorKind::Lift,
        },
        exits: &T_EXITS,
        walls: &[],
        rooms: &[],
        zones: &T_ZONES,
        spawns: &T_SPAWNS,
        pickups: &[],
        props: &[],
        scenario: &T_STEPS,
        surface: Surface::Checker,
    };

    fn world_for(floor: &'static FloorDef) -> World {
        let mut world = World::new();
        spawn_player(&mut world, floor.player_spawn());
        for s in floor.spawns {
            spawn_from_def(&mut world, s);
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

    // A floor with NO initial rogues: `start` spawns the wave and `all_dead`
    // is listed BEFORE it, so a naive single-pass evaluation would fire
    // `all_dead` (0 alive) on the very first tick, before the spawn lands.
    const WAVE_FIRST_STEPS: [StepDef; 3] = [
        StepDef {
            id: "clear",
            trigger: Trigger::AllDead,
            actions: &[Action::OpenExit("a")],
        },
        StepDef {
            id: "intro",
            trigger: Trigger::Start,
            actions: &[Action::Spawn(&T_WAVE)],
        },
        StepDef {
            id: "first_blood",
            trigger: Trigger::Kills(1),
            actions: &[Action::Objective("one down")],
        },
    ];
    const WAVE_FIRST_FLOOR: FloorDef = FloorDef {
        spawns: &[],
        scenario: &WAVE_FIRST_STEPS,
        ..T_FLOOR
    };

    #[test]
    fn same_tick_spawn_is_counted_before_all_dead_and_kills() {
        let mut world = world_for(&WAVE_FIRST_FLOOR);
        let mut sc = ScenarioState::new(&WAVE_FIRST_FLOOR);
        assert_eq!(count_rogues(&world), (0, 0));
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("intro"));
        assert_eq!(count_rogues(&world), (0, 2), "the wave spawned");
        assert!(
            !sc.step_fired("clear"),
            "all_dead must not fire in the tick that spawned the wave"
        );
        assert!(!sc.step_fired("first_blood"));
        assert!(!exit_open(&world, "a"));
        for _ in 0..10 {
            sc.tick(&mut world, 0.1);
        }
        assert!(!sc.step_fired("clear"));
        // Kill one -> kills(1); kill all -> all_dead, in later ticks.
        let first = world.query::<Enemy>()[0];
        world
            .get_component_mut::<Health>(first)
            .unwrap()
            .take_damage(9999);
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("first_blood"));
        assert!(!sc.step_fired("clear"));
        kill_all(&mut world);
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("clear"));
        assert!(exit_open(&world, "a"));
    }

    // Legacy floor (no exit opener) with no initial rogues and a start wave:
    // the auto-open must also see the spawn first.
    const LEGACY_WAVE_FLOOR: FloorDef = FloorDef {
        spawns: &[],
        scenario: &[StepDef {
            id: "intro",
            trigger: Trigger::Start,
            actions: &[Action::Spawn(&T_WAVE)],
        }],
        ..T_FLOOR
    };

    #[test]
    fn legacy_auto_open_waits_for_same_tick_spawns() {
        let mut world = world_for(&LEGACY_WAVE_FLOOR);
        let mut sc = ScenarioState::new(&LEGACY_WAVE_FLOOR);
        sc.tick(&mut world, 0.016);
        assert_eq!(count_rogues(&world), (0, 2));
        assert!(!exit_open(&world, "a") && !exit_open(&world, "b"));
        kill_all(&mut world);
        sc.tick(&mut world, 0.016);
        assert!(exit_open(&world, "a") && exit_open(&world, "b"));
    }

    const ENDING_STEPS: [StepDef; 3] = [
        StepDef {
            id: "boss_down",
            trigger: Trigger::BossDead,
            actions: &[Action::OpenExit("a"), Action::Objective("ride")],
        },
        StepDef {
            id: "uplink",
            trigger: Trigger::Extracted,
            actions: &[Action::Say(SayDef {
                who: "UPLINK",
                text: "carrier",
                delay: 0.0,
            })],
        },
        StepDef {
            id: "never",
            trigger: Trigger::AllDead,
            actions: &[Action::Objective("all dead")],
        },
    ];
    const ENDING_FLOOR: FloorDef = FloorDef {
        spawns: &[],
        scenario: &ENDING_STEPS,
        ..T_FLOOR
    };

    #[test]
    fn boss_dead_and_extracted_triggers() {
        use crate::components::{Boss, Enemy, Radius};
        use crate::ecs::System;
        let mut world = world_for(&ENDING_FLOOR);
        // A boss (an Enemy that carries the Boss marker) plus one plain rogue,
        // so `all_dead` stays false once the boss alone is down.
        let boss = world.spawn();
        world.add_component(boss, Enemy);
        world.add_component(boss, Boss::new());
        world.add_component(boss, Position::new(500.0, 400.0));
        world.add_component(boss, Health::new(100));
        world.add_component(boss, Radius::new(40.0));
        spawn_enemy_with_type(&mut world, Vec2::new(300.0, 300.0), EnemyType::Idle);
        let mut sc = ScenarioState::new(&ENDING_FLOOR);
        sc.tick(&mut world, 0.016);
        assert!(!sc.step_fired("boss_down"));
        assert!(!sc.step_fired("never"));
        world
            .get_component_mut::<Health>(boss)
            .unwrap()
            .take_damage(9999);
        sc.tick(&mut world, 0.016);
        assert!(
            sc.step_fired("boss_down"),
            "boss_dead fires once the boss dies"
        );
        assert!(!sc.step_fired("never"), "a plain rogue is still alive");
        assert!(exit_open(&world, "a"));
        assert_eq!(sc.objective, "ride");
        // Standing in the open exit for the dwell time = extraction -> uplink.
        assert!(!sc.step_fired("uplink"));
        move_player(&mut world, Vec2::new(500.0, 50.0));
        let mut lift = ElevatorSystem;
        for _ in 0..40 {
            lift.run(&mut world, 1.0 / 60.0);
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(sc.step_fired("uplink"));
        assert_eq!(sc.comms.visible()[0].who, "UPLINK");
        assert_eq!(speaker_rgb("UPLINK"), (200, 255, 222));
    }

    // A cold-open style floor: a gate to arrive through, a door out, an
    // asphalt lot, a passive crowd strolling to the forecourt, and the
    // `hold` / `look_at` / `alert` beats.
    const C_ZONES: [ZoneDef; 2] = [
        ZoneDef {
            id: "forecourt",
            rect: Rect::new(380.0, 90.0, 240.0, 90.0),
        },
        ZoneDef {
            id: "lot",
            rect: Rect::new(180.0, 200.0, 640.0, 480.0),
        },
    ];
    const C_SPAWNS: [SpawnDef; 3] = [
        SpawnDef {
            x: 300.0,
            y: 560.0,
            kind: EnemyType::Wandering,
            passive: true,
            walk_to: Some("forecourt"),
            face: Some(-90.0),
            group: Some("crowd"),
        },
        SpawnDef {
            x: 700.0,
            y: 600.0,
            kind: EnemyType::Patrolling,
            passive: true,
            walk_to: Some("forecourt"),
            face: None,
            group: Some("crowd"),
        },
        SpawnDef {
            x: 500.0,
            y: 400.0,
            kind: EnemyType::Idle,
            passive: true,
            walk_to: None,
            face: None,
            group: Some("valet"),
        },
    ];
    const C_EXITS: [ElevatorDef; 1] = [ElevatorDef {
        id: "doors",
        rect: Rect::new(440.0, 20.0, 120.0, 50.0),
        label: "MAIN DOORS",
        to: 1,
        open: true,
        kind: ElevatorKind::Door,
    }];
    const C_STEPS: [StepDef; 5] = [
        StepDef {
            id: "scan",
            trigger: Trigger::Start,
            actions: &[
                Action::Hold(HoldDef {
                    seconds: 1.5,
                    text: Some("SCANNING…"),
                    until_comms_idle: false,
                }),
                Action::LookAt(LookAtDef {
                    x: 500.0,
                    y: 45.0,
                    seconds: 3.0,
                }),
            ],
        },
        StepDef {
            id: "valet",
            trigger: Trigger::Timer {
                seconds: 2.0,
                after: None,
            },
            actions: &[Action::Alert(AlertTarget::Group("valet"))],
        },
        StepDef {
            id: "lot",
            trigger: Trigger::EnterZone("lot"),
            actions: &[Action::Alert(AlertTarget::Zone("lot"))],
        },
        StepDef {
            id: "turn",
            trigger: Trigger::Timer {
                seconds: 30.0,
                after: None,
            },
            actions: &[Action::Alert(AlertTarget::All)],
        },
        StepDef {
            id: "briefing",
            trigger: Trigger::Timer {
                seconds: 40.0,
                after: None,
            },
            actions: &[
                Action::Say(SayDef {
                    who: "CL4-UD3",
                    text: "a fairly long line so the feed stays busy for a while",
                    delay: 0.0,
                }),
                Action::Hold(HoldDef {
                    seconds: 60.0,
                    text: None,
                    until_comms_idle: true,
                }),
            ],
        },
    ];
    const C_FLOOR: FloorDef = FloorDef {
        id: 0,
        name: "GATE",
        theme: "T",
        accent: "#8fd3ff",
        flavor: "",
        objective: "cross",
        width: 1000.0,
        height: 800.0,
        entry: ElevatorDef {
            id: "entry",
            rect: Rect::new(440.0, 720.0, 120.0, 60.0),
            label: "MAIN GATE",
            to: SURFACE_EXIT,
            open: false,
            kind: ElevatorKind::Gate,
        },
        exits: &C_EXITS,
        walls: &[],
        rooms: &[],
        zones: &C_ZONES,
        spawns: &C_SPAWNS,
        pickups: &[],
        scenario: &C_STEPS,
        surface: Surface::Asphalt,

        props: &[],
    };

    fn passives_left(world: &World) -> usize {
        crate::systems::passive::count_passives(world)
    }

    #[test]
    fn hold_locks_for_its_seconds_and_shows_its_caption() {
        let mut world = world_for(&C_FLOOR);
        let mut sc = ScenarioState::new(&C_FLOOR);
        assert!(!sc.hold_active());
        sc.tick(&mut world, 1.0 / 60.0);
        assert!(sc.hold_active());
        assert_eq!(sc.hold_caption(), Some("SCANNING…"));
        // Still held just before 1.5 s...
        for _ in 0..80 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(sc.hold_active(), "held at {:.2}s", sc.time());
        // ...released right after.
        for _ in 0..12 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(!sc.hold_active(), "released at {:.2}s", sc.time());
        assert_eq!(sc.hold_caption(), None);
    }

    #[test]
    fn hold_until_comms_idle_releases_when_the_feed_idles_and_is_capped() {
        let mut world = world_for(&C_FLOOR);
        let mut sc = ScenarioState::new(&C_FLOOR);
        // Jump to the briefing step (t = 40 s).
        while sc.time() < 40.5 {
            sc.tick(&mut world, 0.25);
        }
        assert!(sc.step_fired("briefing"));
        assert!(sc.hold_active());
        assert_eq!(sc.hold_caption(), None);
        // The line takes ~1.4 s to type (+ gap); the hold outlives it by
        // nothing: released once the feed is idle.
        let mut released_at = None;
        for _ in 0..600 {
            sc.tick(&mut world, 1.0 / 60.0);
            if !sc.hold_active() {
                released_at = Some(sc.time());
                break;
            }
        }
        let t = released_at.expect("hold released when the feed idled");
        assert!(
            t - 40.5 > 1.0 && t - 40.5 < 3.0,
            "released after {:.2}s",
            t - 40.5
        );

        // The cap: an until_comms_idle hold with a never-idle feed still ends
        // at HOLD_COMMS_IDLE_CAP (the 60 s asked for is clamped).
        let mut sc = ScenarioState::new(&C_FLOOR);
        let mut world = world_for(&C_FLOOR);
        while sc.time() < 40.5 {
            sc.tick(&mut world, 0.25);
        }
        // Keep the feed busy by re-queueing lines by hand.
        let mut end = None;
        for _ in 0..(HOLD_COMMS_IDLE_CAP as usize + 5) * 4 {
            sc.comms
                .enqueue("CL4-UD3", "still talking, still talking", sc.time());
            sc.tick(&mut world, 0.25);
            if !sc.hold_active() {
                end = Some(sc.time());
                break;
            }
        }
        let end = end.expect("capped hold ends");
        assert!(
            (end - 40.5 - HOLD_COMMS_IDLE_CAP).abs() < 0.6,
            "capped at {HOLD_COMMS_IDLE_CAP}s, ended after {:.2}s",
            end - 40.5
        );
    }

    #[test]
    fn look_at_eases_in_holds_and_eases_out() {
        assert_eq!(look_at_weight(-1.0, 3.0), 0.0);
        assert_eq!(look_at_weight(0.0, 3.0), 0.0);
        assert!((look_at_weight(LOOK_AT_EASE_SECS, 3.0) - 1.0).abs() < 1e-5);
        assert!((look_at_weight(1.5, 3.0) - 1.0).abs() < 1e-5);
        let half = look_at_weight(LOOK_AT_EASE_SECS / 2.0, 3.0);
        assert!((half - 0.5).abs() < 1e-5, "smoothstep midpoint, got {half}");
        assert!(look_at_weight(3.0 - LOOK_AT_EASE_SECS / 2.0, 3.0) < 0.6);
        assert_eq!(look_at_weight(3.0, 3.0), 0.0);
        // Short looks still peak (ramps shrink to half the duration).
        assert!((look_at_weight(0.25, 0.5) - 1.0).abs() < 1e-5);

        let mut world = world_for(&C_FLOOR);
        let mut sc = ScenarioState::new(&C_FLOOR);
        assert!(sc.look_at().is_none());
        sc.tick(&mut world, 1.0 / 60.0);
        let (p, w) = sc.look_at().expect("look running");
        assert_eq!(p, Vec2::new(500.0, 45.0));
        assert!(w < 0.05);
        for _ in 0..60 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        let (_, w) = sc.look_at().unwrap();
        assert!(
            (w - 1.0).abs() < 1e-4,
            "fully on the point after 1 s, got {w}"
        );
        while sc.time() < 3.2 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(sc.look_at().is_none(), "look over after its seconds");
    }

    #[test]
    fn alert_actions_flip_group_zone_and_all() {
        let mut world = world_for(&C_FLOOR);
        let mut sc = ScenarioState::new(&C_FLOOR);
        assert_eq!(passives_left(&world), 3);
        sc.tick(&mut world, 1.0 / 60.0);
        assert_eq!(passives_left(&world), 3, "no alert at start");
        // t = 2 s: the valet group turns.
        while sc.time() < 2.1 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(sc.step_fired("valet"));
        assert_eq!(passives_left(&world), 2);
        // Walk into the lot zone: only the passives standing in it flip; the
        // crowd is (still) down in the lot at t~2 s (they stroll at 55 px/s
        // and start at y 560 / 600, inside `lot`), so both flip.
        move_player(&mut world, Vec2::new(500.0, 400.0));
        sc.tick(&mut world, 1.0 / 60.0);
        assert!(sc.step_fired("lot"));
        assert_eq!(passives_left(&world), 0);
        for e in world.query::<Enemy>() {
            let ai = world.get_component::<AI>(e).unwrap();
            assert_eq!(ai.state, AIState::SurePlayerSeen);
        }
        // Passives count as rogues: killing them all fires all_dead-style
        // counts like any floor.
        assert_eq!(count_rogues(&world), (0, 3));
        kill_all(&mut world);
        assert_eq!(count_rogues(&world), (3, 0));
    }

    #[test]
    fn alert_all_from_a_fresh_floor() {
        let mut world = world_for(&C_FLOOR);
        let mut sc = ScenarioState::new(&C_FLOOR);
        // Nobody entered the lot; at t = 30 s everyone turns.
        while sc.time() < 30.5 {
            sc.tick(&mut world, 0.5);
        }
        assert!(sc.step_fired("turn"));
        assert_eq!(passives_left(&world), 0);
    }

    #[test]
    fn surface_and_portal_kind_parse() {
        assert_eq!(Surface::parse("checker"), Some(Surface::Checker));
        assert_eq!(Surface::parse("asphalt"), Some(Surface::Asphalt));
        assert_eq!(Surface::parse("marble"), Some(Surface::Marble));
        assert_eq!(Surface::parse("concrete"), Some(Surface::Concrete));
        assert_eq!(Surface::parse("grating"), Some(Surface::Grating));
        assert_eq!(Surface::parse("lava"), None);
        assert_eq!(ElevatorKind::parse("lift"), Some(ElevatorKind::Lift));
        assert_eq!(ElevatorKind::parse("door"), Some(ElevatorKind::Door));
        assert_eq!(ElevatorKind::parse("gate"), Some(ElevatorKind::Gate));
        assert_eq!(ElevatorKind::parse("hatch"), None);
        assert_eq!(C_FLOOR.surface, Surface::Asphalt);
    }

    #[test]
    fn door_exits_and_gate_entry_extract_like_lifts() {
        // A `door` exit is an ordinary portal for the elevator system: the
        // kind is rendering only. It carries through to the world entity.
        let mut world = world_for(&C_FLOOR);
        let doors = world
            .query::<Elevator>()
            .into_iter()
            .filter_map(|e| world.get_component::<Elevator>(e).copied())
            .collect::<Vec<_>>();
        let entry = doors.iter().find(|e| !e.is_exit).unwrap();
        let exit = doors.iter().find(|e| e.is_exit).unwrap();
        assert_eq!(entry.kind, ElevatorKind::Gate);
        assert_eq!(exit.kind, ElevatorKind::Door);
        assert!(exit.open && exit.to == 1);
        move_player(&mut world, Vec2::new(500.0, 45.0));
        use crate::ecs::System;
        let mut lift = ElevatorSystem;
        for _ in 0..40 {
            lift.run(&mut world, 1.0 / 60.0);
        }
        assert_eq!(ElevatorSystem::extraction(&world), Some(1));
    }

    #[test]
    fn surface_exit_sentinel_is_never_a_floor_id() {
        assert_ne!(SURFACE_EXIT, 0, "floor 0 is the parking lot now");
        assert!(crate::levels::level_index_for_floor_id(SURFACE_EXIT).is_none());
        // 13½'s car goes to the surface; nothing else does.
        let boss = crate::levels::floor_def(crate::levels::BOSS_LEVEL);
        assert!(boss.exits.iter().all(|e| e.to == SURFACE_EXIT));
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
