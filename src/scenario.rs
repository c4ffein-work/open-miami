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
    /// Hostile only: spawn with no weapon (bare fists) — it fights hand to
    /// hand and, crucially, its corpse DROPS NOTHING. Tutorial victims use
    /// this so a stray E next to the body can never grab a gun that then
    /// dead-ends a `strike` gate.
    pub unarmed: bool,
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
            unarmed: false,
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
    /// The player is inside the zone with this id. With `before`, the step
    /// is DISARMED forever once that other step fires — for scene beats that
    /// only make sense before a point of no return (floor 1's "not past the
    /// line" block only plays before the desk scene).
    EnterZone {
        zone: &'static str,
        before: Option<&'static str>,
    },
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

/// One visual-novel dialogue line (a `talk` action): consecutive `talk`
/// actions within one step form ONE conversation, paced by the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TalkDef {
    pub who: &'static str,
    pub text: &'static str,
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

/// The player input a tutorial `gate` waits for. A gate releases only when
/// the action SUCCEEDS (the punch connects, the finisher completes, ...) —
/// matched against the frame's [`GameEvent`]s in [`ScenarioState::gate_notify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateInput {
    /// An unarmed strike that connects ([`GameEvent::PunchLanded`]).
    Punch,
    /// A finisher runs to completion ([`GameEvent::FinisherDone`]).
    Finish,
    /// A weapon picked up off the floor ([`GameEvent::Pickup`]).
    Pickup,
    /// An armed melee hit that connects ([`GameEvent::StrikeLanded`]).
    Strike,
    /// A gun shot fired ([`GameEvent::PlayerFired`], non-melee).
    Fire,
    /// A thrown weapon that connects ([`GameEvent::ThrownImpact`]).
    Throw,
}

impl GateInput {
    /// Parse the JSON `gate.input`; unknown = `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "punch" => Some(GateInput::Punch),
            "finish" => Some(GateInput::Finish),
            "pickup" => Some(GateInput::Pickup),
            "strike" => Some(GateInput::Strike),
            "fire" => Some(GateInput::Fire),
            "throw" => Some(GateInput::Throw),
            _ => None,
        }
    }

    /// Whether the left-click attack is the gated input, given what the
    /// player holds: only the EXACT gated action passes — a `punch` gate
    /// swings only bare fists, a `strike` gate only an armed melee weapon, a
    /// `fire` gate only a gun. The same button with the wrong tool stays
    /// masked (a stray shot during a `strike` gate could kill the target the
    /// gate needs).
    pub fn allows_primary(self, weapon: Option<crate::components::WeaponType>) -> bool {
        match self {
            GateInput::Punch => weapon.is_none(),
            GateInput::Strike => weapon.is_some_and(|w| w.is_melee()),
            GateInput::Fire => weapon.is_some_and(|w| !w.is_melee()),
            _ => false,
        }
    }

    /// Whether a left click may start a finisher.
    pub fn allows_finisher(self) -> bool {
        self == GateInput::Finish
    }

    /// Whether the pick-up key works. Besides the `pickup` gate itself,
    /// every gate whose action needs the RIGHT weapon in hand keeps E live
    /// as a recovery path (a missed throw leaves the weapon on the floor; a
    /// `strike` gate reached holding a gun needs a way to swap to the bar).
    /// Only `punch` (must stay unarmed) and `finish` (works with anything)
    /// mask it.
    pub fn allows_pickup(self) -> bool {
        matches!(
            self,
            GateInput::Pickup | GateInput::Strike | GateInput::Fire | GateInput::Throw
        )
    }

    /// Whether the right-click throw works.
    pub fn allows_throw(self) -> bool {
        self == GateInput::Throw
    }

    /// Whether this frame `event` satisfies the gate.
    pub fn satisfied_by(self, event: &crate::components::GameEvent) -> bool {
        use crate::components::GameEvent;
        match (self, event) {
            (GateInput::Punch, GameEvent::PunchLanded) => true,
            (GateInput::Finish, GameEvent::FinisherDone) => true,
            (GateInput::Pickup, GameEvent::Pickup) => true,
            (GateInput::Strike, GameEvent::StrikeLanded) => true,
            (GateInput::Fire, GameEvent::PlayerFired(t)) => !t.is_melee(),
            (GateInput::Throw, GameEvent::ThrownImpact) => true,
            _ => false,
        }
    }
}

/// A `gate` action: freeze the world and wait for one specific player input
/// to SUCCEED (tutorial beats). `text` is the on-screen prompt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateDef {
    pub input: GateInput,
    pub text: &'static str,
}

/// What a step does when it fires.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Say(SayDef),
    /// Queue a line of the step's cinematic conversation (dialogue mode).
    Talk(TalkDef),
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
    /// TUTORIAL GATE: freeze the world until the gated input succeeds. The
    /// step's actions AFTER the gate run on release, and the step only
    /// counts as done (`step_done` / `timer.after`) once the gate releases.
    Gate(GateDef),
    /// Snapshot the run (world + scenario) — death restores it (see the
    /// game loop). `{ "checkpoint": true }` in the JSON.
    Checkpoint,
    /// Take the player's held weapon away (the checkpoint desk keeps it).
    /// `{ "disarm": true }` in the JSON.
    Disarm,
    /// Enable / disable the player's fighting capabilities (fire, throw,
    /// punch, finisher). Off = a walking-only beat (the parking lot); gates
    /// bypass it (a gate explicitly demands its one action). Default on;
    /// resets on every floor. `{ "combat": false }` in the JSON.
    Combat(bool),
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
    /// The running `talk` conversation (dialogue mode), if any.
    dialogue: Option<DialogueState>,
    /// Per step: the time its `talk` conversation finished (`None` = not yet
    /// or the step has no `talk`). A `timer { after }` on a talking step
    /// counts from this instead of the fire time.
    talk_done_at: Vec<Option<f32>>,
    /// The active tutorial `gate`, if any: the world is frozen and the
    /// scenario clock stops until [`ScenarioState::gate_notify`] sees the
    /// gated input succeed.
    gate: Option<GateState>,
    /// Per step: the time its (last) `gate` released (`None` = not yet or no
    /// gate). `step_done` / `timer { after }` on a gated step wait for this.
    gate_done_at: Vec<Option<f32>>,
    /// A `checkpoint` action ran since the last
    /// [`ScenarioState::take_checkpoint_request`].
    checkpoint_requested: bool,
    /// Whether the player may fight (see [`Action::Combat`]). Default true.
    combat_enabled: bool,
}

/// Live state of a tutorial `gate`: what it waits for, which step owns it,
/// and the actions of that step still to run once it releases.
#[derive(Debug, Clone, Copy)]
struct GateState {
    def: GateDef,
    step_idx: usize,
    /// The owning step's actions AFTER the gate (run on release; may install
    /// the step's next gate, chaining within one step).
    rest: &'static [Action],
    /// Where the gated action happens (the step's spawned target / the
    /// pickup / the downed victim): the player is tethered near it by
    /// invisible walls ([`GATE_TETHER_RADIUS`]) while the gate holds.
    anchor: Option<Vec2>,
}

/// Live state of a `talk` conversation: the line on screen, the lines still
/// queued, and the panel's slide animation. Advanced by the player
/// ([`ScenarioState::dialogue_advance`]), not by the clock.
#[derive(Debug, Clone)]
struct DialogueState {
    current: TalkDef,
    queue: VecDeque<TalkDef>,
    /// Seconds the current line has been up (drives the typewriter).
    line_age: f32,
    /// Panel presence 0..1: rises to 1 while open, falls to 0 once `closing`.
    slide: f32,
    /// The last line was dismissed: the panel is sliding out.
    closing: bool,
    /// Indices of the steps whose `talk` lines joined this conversation.
    owners: Vec<usize>,
}

/// What the renderer needs to draw the dialogue panel this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogueView {
    pub who: &'static str,
    pub text: &'static str,
    /// Characters revealed by the typewriter so far.
    pub chars_shown: usize,
    pub fully_typed: bool,
    /// Panel presence 0..1, already smoothstepped (0 = off screen).
    pub slide: f32,
    /// More lines are queued after this one.
    pub more: bool,
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
/// Slide-in / slide-out length of the dialogue panel, seconds.
pub const DIALOGUE_SLIDE_SECS: f32 = 0.25;
/// Typewriter speed of a dialogue line, characters per second (snappier than
/// the ambient comms feed; a press mid-line reveals the whole line).
pub const DIALOGUE_CHARS_PER_SEC: f32 = 55.0;

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
            dialogue: None,
            talk_done_at: vec![None; floor.scenario.len()],
            gate: None,
            gate_done_at: vec![None; floor.scenario.len()],
            checkpoint_requested: false,
            combat_enabled: true,
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

    /// Whether a `talk` conversation is up (the player is locked and paces
    /// it with click / Space / Enter). True through the slide-out too, so a
    /// dismissing click can never fire the weapon.
    pub fn dialogue_active(&self) -> bool {
        self.dialogue.is_some()
    }

    /// The dialogue panel to draw this frame, if a conversation is up.
    pub fn dialogue_view(&self) -> Option<DialogueView> {
        let d = self.dialogue.as_ref()?;
        let total = d.current.text.chars().count();
        let shown = ((d.line_age * DIALOGUE_CHARS_PER_SEC) as usize).min(total);
        let s = d.slide.clamp(0.0, 1.0);
        Some(DialogueView {
            who: d.current.who,
            text: d.current.text,
            chars_shown: shown,
            fully_typed: shown >= total,
            slide: s * s * (3.0 - 2.0 * s),
            more: !d.queue.is_empty(),
        })
    }

    /// Player input on an active conversation: reveal the current line if it
    /// is still typing, otherwise move to the next line, otherwise dismiss
    /// the panel (it slides out and control returns).
    pub fn dialogue_advance(&mut self) {
        let Some(d) = self.dialogue.as_mut() else {
            return;
        };
        if d.closing {
            return;
        }
        let typing_time = d.current.text.chars().count() as f32 / DIALOGUE_CHARS_PER_SEC;
        if d.line_age < typing_time {
            d.line_age = typing_time;
        } else if let Some(next) = d.queue.pop_front() {
            d.current = next;
            d.line_age = 0.0;
        } else {
            d.closing = true;
        }
    }

    /// The active tutorial gate, if any (the world must be frozen: the game
    /// loop runs only the player-driven systems and masks every input but
    /// the gated one — see `sim::gate_frozen_step`).
    pub fn gate_view(&self) -> Option<GateDef> {
        self.gate.map(|g| g.def)
    }

    /// Where the active gate's action happens, if the gate could tell: the
    /// tether centre for [`tether_player`].
    pub fn gate_anchor(&self) -> Option<Vec2> {
        self.gate.and_then(|g| g.anchor)
    }

    /// Whether the player may fight (fire / throw / punch / finisher). Gates
    /// bypass this: their one action stays allowed.
    pub fn combat_enabled(&self) -> bool {
        self.combat_enabled
    }

    /// Feed the frame's drained [`GameEvent`]s to the active gate: when one
    /// satisfies it, the gate releases — the owning step's remaining actions
    /// run (possibly installing the step's next gate) and the step counts as
    /// done for `step_done` / `timer { after }` chains.
    ///
    /// [`GameEvent`]: crate::components::GameEvent
    pub fn gate_notify(&mut self, world: &mut World, events: &[crate::components::GameEvent]) {
        let Some(g) = self.gate else { return };
        if events.iter().any(|e| g.def.input.satisfied_by(e)) {
            self.gate = None;
            self.gate_done_at[g.step_idx] = Some(self.time);
            self.run_actions(world, g.rest, g.step_idx);
        }
    }

    /// Debug escape hatch (the `?debug` **G** key): release the active gate
    /// as if its input had succeeded, so a mis-designed gate with no possible
    /// target can never softlock a session.
    pub fn gate_skip(&mut self, world: &mut World) {
        if let Some(g) = self.gate.take() {
            self.gate_done_at[g.step_idx] = Some(self.time);
            self.run_actions(world, g.rest, g.step_idx);
        }
    }

    /// Whether a `checkpoint` action ran since the last call: the game loop
    /// snapshots the world + this scenario when it returns true.
    pub fn take_checkpoint_request(&mut self) -> bool {
        std::mem::take(&mut self.checkpoint_requested)
    }

    /// Queue one `talk` line (from step `step_idx`). Starts a conversation if
    /// none is up; otherwise appends to the running one — so consecutive
    /// `talk` actions (and same-tick steps) form a single conversation.
    fn enqueue_talk(&mut self, line: TalkDef, step_idx: usize) {
        match self.dialogue.as_mut() {
            Some(d) => {
                d.queue.push_back(line);
                if d.closing {
                    // The panel was on its way out: reopen on the new line.
                    d.closing = false;
                    d.current = d.queue.pop_front().unwrap();
                    d.line_age = 0.0;
                }
                if !d.owners.contains(&step_idx) {
                    d.owners.push(step_idx);
                }
            }
            None => {
                self.dialogue = Some(DialogueState {
                    current: line,
                    queue: VecDeque::new(),
                    line_age: 0.0,
                    slide: 0.0,
                    closing: false,
                    owners: vec![step_idx],
                });
            }
        }
    }

    /// Advance the `hold` / `look_at` / dialogue clocks (called from `tick`).
    fn tick_beats(&mut self, dt: f32) {
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
        let mut finished = false;
        if let Some(d) = self.dialogue.as_mut() {
            d.line_age += dt;
            let step = dt / DIALOGUE_SLIDE_SECS;
            if d.closing {
                d.slide -= step;
                finished = d.slide <= 0.0;
            } else {
                d.slide = (d.slide + step).min(1.0);
            }
        }
        if finished {
            if let Some(d) = self.dialogue.take() {
                for i in d.owners {
                    self.talk_done_at[i] = Some(self.time);
                }
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
        // A tutorial gate freezes the whole scenario: the clock and every
        // trigger hold their breath (timers must not advance) until the
        // gated input succeeds ([`ScenarioState::gate_notify`]). Only the
        // comms typewriter keeps playing — the lines a gated step queued
        // BEFORE its gate (the beat's flavour) still type out under the
        // prompt; the frozen clock keeps delayed lines waiting.
        if self.gate.is_some() {
            self.comms.update(self.time, dt);
            return;
        }
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
                        self.run_actions(world, step.actions, i);
                        counts = count_rogues(world);
                        fired_any = true;
                        if self.gate.is_some() {
                            // A gate just installed: the world (and this
                            // scenario) freeze mid-tick. Nothing else fires
                            // until the gate releases.
                            return;
                        }
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
        self.tick_beats(dt);
    }

    fn trigger_holds(&self, trigger: Trigger, ctx: &TriggerCtx) -> bool {
        match trigger {
            Trigger::Start => true,
            Trigger::EnterZone { zone, before } => {
                // `before`: disarmed forever once that other step fires.
                if before.is_some_and(|id| self.fired_time(id).is_some()) {
                    return false;
                }
                match (ctx.player_pos, self.floor.zone(zone)) {
                    (Some(p), Some(z)) => z.rect.contains(p),
                    _ => false,
                }
            }
            Trigger::Kills(n) => ctx.kills >= n,
            Trigger::AllDead => ctx.alive == 0,
            Trigger::Timer { seconds, after } => {
                let base = match after {
                    None => Some(0.0),
                    Some(id) => self.after_time(id),
                };
                match base {
                    Some(t0) => self.time - t0 >= seconds,
                    None => false,
                }
            }
            Trigger::ExitOpen(None) => !self.opened_exits.is_empty(),
            Trigger::ExitOpen(Some(id)) => self.opened_exits.contains(&id),
            Trigger::StepDone(id) => self.step_done_time(id).is_some(),
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

    /// When step `id` counts as DONE for a `step_done` trigger: for a step
    /// with `gate` actions, the moment its (last) gate released (`None`
    /// while it is pending — triggers are never evaluated while a gate is
    /// active, so a released gate here is the step's final one); for any
    /// other step, the moment it fired.
    fn step_done_time(&self, id: &str) -> Option<f32> {
        let (i, step) = self
            .floor
            .scenario
            .iter()
            .enumerate()
            .find(|(_, s)| s.id == id)?;
        let fired = self.fired_at[i]?;
        if step.actions.iter().any(|a| matches!(a, Action::Gate(_))) {
            self.gate_done_at[i]
        } else {
            Some(fired)
        }
    }

    /// The base time a `timer { after: id }` counts from: for a step with
    /// `gate` actions, the moment its gate released; for a step with `talk`
    /// actions, the moment its conversation ENDED (panel dismissed; `None`
    /// while it is still up — the player paces it); for any other step, the
    /// moment it fired.
    fn after_time(&self, id: &str) -> Option<f32> {
        let (i, step) = self
            .floor
            .scenario
            .iter()
            .enumerate()
            .find(|(_, s)| s.id == id)?;
        let fired = self.fired_at[i]?;
        if step.actions.iter().any(|a| matches!(a, Action::Gate(_))) {
            self.gate_done_at[i]
        } else if step.actions.iter().any(|a| matches!(a, Action::Talk(_))) {
            self.talk_done_at[i]
        } else {
            Some(fired)
        }
    }

    fn run_actions(&mut self, world: &mut World, actions: &'static [Action], step_idx: usize) {
        // The last position this step spawned at: the default gate anchor
        // (tutorial steps spawn their victim just before their gate).
        let mut last_spawn: Option<Vec2> = None;
        for (i, action) in actions.iter().enumerate() {
            match *action {
                Action::Gate(def) => {
                    // Install the gate and STOP: the remaining actions run
                    // when it releases (`gate_notify`), so gates chain
                    // within one step. One gate at a time by construction —
                    // the scenario clock halts while one is active, so no
                    // other step can fire under it.
                    // No anchor for `pickup` gates: fetching IS a walk (and
                    // corpses drop weapons, so "nearest pickup" often points
                    // at the wrong thing).
                    let anchor = last_spawn.or_else(|| match def.input {
                        GateInput::Finish => nearest_stunned_to_player(world),
                        _ => None,
                    });
                    self.gate = Some(GateState {
                        def,
                        step_idx,
                        rest: &actions[i + 1..],
                        anchor,
                    });
                    return;
                }
                Action::Checkpoint => self.checkpoint_requested = true,
                Action::Combat(on) => self.combat_enabled = on,
                Action::Disarm => {
                    if let Some(&p) = world.query::<Player>().first() {
                        world.remove_component::<crate::components::Weapon>(p);
                    }
                }
                Action::Say(say) => {
                    self.comms.enqueue(say.who, say.text, self.time + say.delay);
                }
                Action::Talk(line) => self.enqueue_talk(line, step_idx),
                Action::Spawn(spawns) => {
                    for s in spawns {
                        spawn_from_def(world, s);
                    }
                    last_spawn = spawns.last().map(|s| Vec2::new(s.x, s.y));
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

/// How far from the gate's anchor the player may wander while a tutorial
/// gate holds ([`tether_player`]'s invisible walls).
pub const GATE_TETHER_RADIUS: f32 = 180.0;

/// Invisible walls while a gate holds: clamp the player inside the circle
/// (`anchor`, `radius`). Call after movement resolved (browser loop and the
/// headless sim share it).
pub fn tether_player(world: &mut World, anchor: Vec2, radius: f32) {
    let player = match world.query::<crate::components::Player>().first() {
        Some(&p) => p,
        None => return,
    };
    if let Some(pos) = world.get_component_mut::<crate::components::Position>(player) {
        let dx = pos.x - anchor.x;
        let dy = pos.y - anchor.y;
        let d = (dx * dx + dy * dy).sqrt();
        if d > radius {
            pos.x = anchor.x + dx / d * radius;
            pos.y = anchor.y + dy / d * radius;
        }
    }
}

/// The position of the stunned (downed) enemy nearest to the player (a
/// `finish` gate's anchor — its victim went down in an earlier step).
fn nearest_stunned_to_player(world: &World) -> Option<Vec2> {
    let player = *world.query::<crate::components::Player>().first()?;
    let p = *world.get_component::<crate::components::Position>(player)?;
    world
        .query::<crate::components::Stunned>()
        .into_iter()
        .filter(|&e| world.has_component::<crate::components::Enemy>(e))
        .filter_map(|e| world.get_component::<crate::components::Position>(e))
        .map(|q| {
            (
                Vec2::new(q.x, q.y),
                (q.x - p.x).powi(2) + (q.y - p.y).powi(2),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, _)| v)
}

/// Spawn one placement: a hostile rogue of `kind`, or a passive bot when
/// `def.passive` (see `systems::passive`).
pub fn spawn_from_def(world: &mut World, def: &SpawnDef) -> crate::ecs::Entity {
    if def.passive {
        crate::systems::passive::spawn_passive(world, def)
    } else {
        let e = spawn_enemy_with_type(world, Vec2::new(def.x, def.y), def.kind);
        if def.unarmed {
            world.remove_component::<crate::components::Weapon>(e);
        }
        e
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
            trigger: Trigger::EnterZone {
                zone: "z",
                before: None,
            },
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

    const COMBAT_FLOOR: FloorDef = FloorDef {
        scenario: &[
            StepDef {
                id: "walk_in",
                trigger: Trigger::Start,
                actions: &[Action::Combat(false)],
            },
            StepDef {
                id: "fight",
                trigger: Trigger::Timer {
                    seconds: 5.0,
                    after: None,
                },
                actions: &[Action::Combat(true)],
            },
        ],
        ..T_FLOOR
    };

    #[test]
    fn combat_action_toggles_fighting() {
        let mut world = world_for(&COMBAT_FLOOR);
        let mut sc = ScenarioState::new(&COMBAT_FLOOR);
        assert!(sc.combat_enabled(), "fighting is on by default");
        sc.tick(&mut world, 0.016);
        assert!(!sc.combat_enabled(), "the walk-in beat turned it off");
        for _ in 0..400 {
            sc.tick(&mut world, 0.016);
        }
        assert!(sc.combat_enabled(), "the 5s timer step turned it back on");
    }

    const ANCHOR_FLOOR: FloorDef = FloorDef {
        scenario: &[StepDef {
            id: "tut",
            trigger: Trigger::Start,
            actions: &[
                Action::Spawn(&[SpawnDef::hostile(333.0, 222.0, EnemyType::Idle)]),
                Action::Gate(GateDef {
                    input: GateInput::Punch,
                    text: "LEFT CLICK — PUNCH",
                }),
            ],
        }],
        ..T_FLOOR
    };

    #[test]
    fn gate_anchors_on_its_spawn_and_tethers_the_player() {
        let mut world = world_for(&ANCHOR_FLOOR);
        let mut sc = ScenarioState::new(&ANCHOR_FLOOR);
        sc.tick(&mut world, 0.016);
        let anchor = sc.gate_anchor().expect("the gate anchored on its spawn");
        assert_eq!((anchor.x, anchor.y), (333.0, 222.0));
        // Drag the player far away: the tether's invisible walls clamp them
        // back onto the circle.
        let player = *world.query::<Player>().first().unwrap();
        if let Some(p) = world.get_component_mut::<Position>(player) {
            p.x = anchor.x + 500.0;
            p.y = anchor.y;
        }
        tether_player(&mut world, anchor, GATE_TETHER_RADIUS);
        let p = world.get_component::<Position>(player).unwrap();
        assert!((p.x - (anchor.x + GATE_TETHER_RADIUS)).abs() < 0.01);
        assert_eq!(p.y, anchor.y);
        // Inside the circle nothing moves.
        if let Some(p) = world.get_component_mut::<Position>(player) {
            p.x = anchor.x + 50.0;
        }
        tether_player(&mut world, anchor, GATE_TETHER_RADIUS);
        let p = world.get_component::<Position>(player).unwrap();
        assert_eq!(p.x, anchor.x + 50.0);
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
            unarmed: false,
        },
        SpawnDef {
            x: 700.0,
            y: 600.0,
            kind: EnemyType::Patrolling,
            passive: true,
            walk_to: Some("forecourt"),
            face: None,
            group: Some("crowd"),
            unarmed: false,
        },
        SpawnDef {
            x: 500.0,
            y: 400.0,
            kind: EnemyType::Idle,
            passive: true,
            walk_to: None,
            face: None,
            group: Some("valet"),
            unarmed: false,
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
            trigger: Trigger::EnterZone {
                zone: "lot",
                before: None,
            },
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

    // A cold-open style conversation: two talk lines in one step, and a
    // doors-style timer counting from the conversation's END.
    const TALK_STEPS: [StepDef; 3] = [
        StepDef {
            id: "conv",
            trigger: Trigger::Start,
            actions: &[
                Action::Talk(TalkDef {
                    who: "SENTINEL",
                    text: "STATE PURPOSE.",
                }),
                Action::Talk(TalkDef {
                    who: "CL4-UD3",
                    text: "Maintenance.",
                }),
            ],
        },
        StepDef {
            id: "doors",
            trigger: Trigger::Timer {
                seconds: 0.5,
                after: Some("conv"),
            },
            actions: &[Action::Sfx("elevator")],
        },
        StepDef {
            id: "plain",
            trigger: Trigger::Timer {
                seconds: 0.2,
                after: Some("doors"),
            },
            actions: &[Action::Objective("after doors")],
        },
    ];
    const TALK_FLOOR: FloorDef = FloorDef {
        spawns: &[],
        scenario: &TALK_STEPS,
        ..T_FLOOR
    };

    #[test]
    fn talk_lines_form_one_player_paced_conversation() {
        let mut world = world_for(&TALK_FLOOR);
        let mut sc = ScenarioState::new(&TALK_FLOOR);
        assert!(!sc.dialogue_active());
        sc.tick(&mut world, 1.0 / 60.0);
        assert!(sc.step_fired("conv"));
        assert!(sc.dialogue_active(), "the conversation opened");
        let v = sc.dialogue_view().unwrap();
        assert_eq!(v.who, "SENTINEL");
        assert_eq!(v.text, "STATE PURPOSE.");
        assert!(v.more, "a second line is queued");
        assert!(v.slide < 0.5, "the panel is still sliding in");
        // The panel finishes sliding in and the line types out.
        for _ in 0..60 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        let v = sc.dialogue_view().unwrap();
        assert!((v.slide - 1.0).abs() < 1e-4);
        assert!(v.fully_typed);
        // Player-paced: no amount of waiting advances or dismisses it.
        for _ in 0..600 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert_eq!(sc.dialogue_view().unwrap().who, "SENTINEL");
        assert!(
            !sc.step_fired("doors"),
            "the after-conv timer must not run while the conversation is up"
        );
        // Advance -> second line, typewriter restarted.
        sc.dialogue_advance();
        let v = sc.dialogue_view().unwrap();
        assert_eq!(v.who, "CL4-UD3");
        assert_eq!(v.chars_shown, 0);
        assert!(!v.more);
        // A press mid-typing reveals the whole line instead of advancing.
        sc.tick(&mut world, 1.0 / 60.0);
        assert!(!sc.dialogue_view().unwrap().fully_typed);
        sc.dialogue_advance();
        let v = sc.dialogue_view().unwrap();
        assert_eq!(v.who, "CL4-UD3");
        assert!(v.fully_typed);
        // Dismiss -> the panel slides out, input stays locked until gone.
        sc.dialogue_advance();
        assert!(sc.dialogue_active());
        for _ in 0..30 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(!sc.dialogue_active(), "panel gone after the slide-out");
    }

    #[test]
    fn timer_after_talk_step_counts_from_the_conversation_end() {
        let mut world = world_for(&TALK_FLOOR);
        let mut sc = ScenarioState::new(&TALK_FLOOR);
        sc.tick(&mut world, 1.0 / 60.0);
        // Let a lot of time pass, then click through both lines.
        for _ in 0..300 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        sc.dialogue_advance(); // line 2 (line 1 fully typed long ago)
        for _ in 0..60 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        sc.dialogue_advance(); // dismiss
        let mut end = None;
        for _ in 0..60 {
            sc.tick(&mut world, 1.0 / 60.0);
            if !sc.dialogue_active() {
                end = Some(sc.time());
                break;
            }
        }
        let end = end.expect("conversation ended");
        assert!(!sc.step_fired("doors"), "0.5s not yet elapsed at {end}");
        while sc.time() < end + 0.6 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(
            sc.step_fired("doors"),
            "doors fires 0.5s after the conversation ended"
        );
        // A timer after a step WITHOUT talk still counts from its fire time.
        for _ in 0..20 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(sc.step_fired("plain"));
    }

    #[test]
    fn same_tick_talk_steps_merge_into_one_conversation() {
        // Two start steps both talking: their lines join one conversation and
        // BOTH steps get their talk-done time when it ends.
        const MERGE_STEPS: [StepDef; 3] = [
            StepDef {
                id: "a",
                trigger: Trigger::Start,
                actions: &[Action::Talk(TalkDef {
                    who: "SWARM",
                    text: "one",
                })],
            },
            StepDef {
                id: "b",
                trigger: Trigger::Start,
                actions: &[Action::Talk(TalkDef {
                    who: "CL4-UD3",
                    text: "two",
                })],
            },
            StepDef {
                id: "after_b",
                trigger: Trigger::Timer {
                    seconds: 0.1,
                    after: Some("b"),
                },
                actions: &[Action::Objective("done")],
            },
        ];
        const MERGE_FLOOR: FloorDef = FloorDef {
            spawns: &[],
            scenario: &MERGE_STEPS,
            ..T_FLOOR
        };
        let mut world = world_for(&MERGE_FLOOR);
        let mut sc = ScenarioState::new(&MERGE_FLOOR);
        sc.tick(&mut world, 1.0 / 60.0);
        let v = sc.dialogue_view().unwrap();
        assert_eq!(v.who, "SWARM");
        assert!(v.more, "step b's line queued behind step a's");
        for _ in 0..30 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        sc.dialogue_advance();
        assert_eq!(sc.dialogue_view().unwrap().who, "CL4-UD3");
        for _ in 0..30 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(!sc.step_fired("after_b"));
        sc.dialogue_advance();
        for _ in 0..60 {
            sc.tick(&mut world, 1.0 / 60.0);
        }
        assert!(!sc.dialogue_active());
        assert!(
            sc.step_fired("after_b"),
            "b's talk-done time was recorded when the merged conversation ended"
        );
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

    // ---- tutorial gates + checkpoints ---------------------------------

    use crate::components::{GameEvent, Weapon, WeaponType};
    use crate::sim::Simulation;

    #[test]
    fn gate_input_parse_masking_and_satisfaction() {
        assert_eq!(GateInput::parse("punch"), Some(GateInput::Punch));
        assert_eq!(GateInput::parse("finish"), Some(GateInput::Finish));
        assert_eq!(GateInput::parse("pickup"), Some(GateInput::Pickup));
        assert_eq!(GateInput::parse("strike"), Some(GateInput::Strike));
        assert_eq!(GateInput::parse("fire"), Some(GateInput::Fire));
        assert_eq!(GateInput::parse("throw"), Some(GateInput::Throw));
        assert_eq!(GateInput::parse("dance"), None);

        // The left click only works with the matching tool in hand.
        assert!(GateInput::Punch.allows_primary(None));
        assert!(!GateInput::Punch.allows_primary(Some(WeaponType::Melee)));
        assert!(GateInput::Strike.allows_primary(Some(WeaponType::Melee)));
        assert!(!GateInput::Strike.allows_primary(Some(WeaponType::Pistol)));
        assert!(!GateInput::Strike.allows_primary(None));
        assert!(GateInput::Fire.allows_primary(Some(WeaponType::Shotgun)));
        assert!(!GateInput::Fire.allows_primary(Some(WeaponType::Melee)));
        assert!(!GateInput::Pickup.allows_primary(None));
        assert!(GateInput::Finish.allows_finisher());
        assert!(!GateInput::Punch.allows_finisher());
        // E stays live on every weapon-dependent gate (recovery path).
        assert!(GateInput::Pickup.allows_pickup());
        assert!(GateInput::Strike.allows_pickup());
        assert!(GateInput::Fire.allows_pickup());
        assert!(GateInput::Throw.allows_pickup());
        assert!(!GateInput::Punch.allows_pickup());
        assert!(!GateInput::Finish.allows_pickup());
        assert!(GateInput::Throw.allows_throw());
        assert!(!GateInput::Pickup.allows_throw());

        // Success events, one per gate kind.
        assert!(GateInput::Punch.satisfied_by(&GameEvent::PunchLanded));
        assert!(!GateInput::Punch.satisfied_by(&GameEvent::StrikeLanded));
        assert!(GateInput::Finish.satisfied_by(&GameEvent::FinisherDone));
        assert!(GateInput::Pickup.satisfied_by(&GameEvent::Pickup));
        assert!(GateInput::Strike.satisfied_by(&GameEvent::StrikeLanded));
        assert!(!GateInput::Strike.satisfied_by(&GameEvent::EnemyHit {
            by: WeaponType::Melee
        }));
        assert!(GateInput::Fire.satisfied_by(&GameEvent::PlayerFired(WeaponType::Pistol)));
        assert!(
            !GateInput::Fire.satisfied_by(&GameEvent::PlayerFired(WeaponType::Melee)),
            "a melee swing is not a shot"
        );
        assert!(GateInput::Throw.satisfied_by(&GameEvent::ThrownImpact));
        assert!(
            !GateInput::Throw.satisfied_by(&GameEvent::Throw),
            "a throw satisfies only when it CONNECTS"
        );
    }

    // A tutorial-style floor: entering the zone disarms the player, drops a
    // checkpoint, spawns a lone rogue and gates on PUNCH, then chains a
    // FINISH gate through step_done, with timers watching the frozen clock.
    const GT_WAVE: [SpawnDef; 1] = [SpawnDef::hostile(650.0, 650.0, EnemyType::Idle)];
    const GT_STEPS: [StepDef; 4] = [
        StepDef {
            id: "teach",
            trigger: Trigger::EnterZone {
                zone: "z",
                before: None,
            },
            actions: &[
                Action::Disarm,
                Action::Checkpoint,
                Action::Spawn(&GT_WAVE),
                Action::Gate(GateDef {
                    input: GateInput::Punch,
                    text: "LEFT CLICK — PUNCH",
                }),
                Action::Objective("punched"),
            ],
        },
        StepDef {
            id: "next",
            trigger: Trigger::StepDone("teach"),
            actions: &[
                Action::Gate(GateDef {
                    input: GateInput::Finish,
                    text: "LEFT CLICK — FINISH",
                }),
                Action::Objective("finished"),
            ],
        },
        StepDef {
            id: "late",
            trigger: Trigger::Timer {
                seconds: 0.5,
                after: Some("teach"),
            },
            actions: &[Action::Sfx("ping")],
        },
        StepDef {
            id: "clock",
            trigger: Trigger::Timer {
                seconds: 0.3,
                after: None,
            },
            actions: &[Action::Sfx("clock")],
        },
    ];
    const GT_FLOOR: FloorDef = FloorDef {
        spawns: &[],
        scenario: &GT_STEPS,
        ..T_FLOOR
    };

    const DT: f32 = 1.0 / 60.0;

    fn sim_for(floor: &'static FloorDef) -> (Simulation, ScenarioState) {
        (
            Simulation::from_world(world_for(floor)),
            ScenarioState::new(floor),
        )
    }

    fn teleport(sim: &mut Simulation, to: Vec2) {
        let p = sim.player().unwrap();
        *sim.world.get_component_mut::<Position>(p).unwrap() = Position::from_vec2(to);
    }

    /// Advance `frames` full frames; returns the checkpoint snapshot taken
    /// at the frame a `checkpoint` action requested one (if any).
    fn run(
        sim: &mut Simulation,
        sc: &mut ScenarioState,
        frames: usize,
    ) -> Option<(World, ScenarioState)> {
        let mut cp = None;
        for _ in 0..frames {
            if sim.scenario_step(sc, DT) {
                cp = Some((sim.world.clone(), sc.clone()));
            }
        }
        cp
    }

    #[test]
    fn gate_freezes_the_world_and_releases_on_the_punch() {
        let (mut sim, mut sc) = sim_for(&GT_FLOOR);
        // Walk into the zone: the teach step disarms, checkpoints, spawns
        // the target and freezes on the PUNCH gate.
        teleport(&mut sim, Vec2::new(650.0, 620.0));
        let cp = run(&mut sim, &mut sc, 1);
        assert!(sc.step_fired("teach"));
        let gate = sc.gate_view().expect("gate installed");
        assert_eq!(gate.input, GateInput::Punch);
        assert_eq!(gate.text, "LEFT CLICK — PUNCH");
        assert!(cp.is_some(), "the checkpoint action requested a snapshot");
        let player = sim.player().unwrap();
        assert!(
            sim.world.get_component::<Weapon>(player).is_none(),
            "disarmed"
        );
        assert_eq!(sc.objective, "start", "post-gate actions did NOT run yet");
        let t_frozen = sc.time();

        // Make the enemy hostile and shoving-fast: under the freeze it still
        // never moves and never attacks, and the clock never advances.
        let enemy = sim.world.query::<Enemy>()[0];
        sim.world.get_component_mut::<AI>(enemy).unwrap().state = AIState::SurePlayerSeen;
        sim.world
            .get_component_mut::<crate::components::Velocity>(enemy)
            .map(|v| {
                v.x = 150.0;
                v.y = 0.0;
            })
            .unwrap();
        let enemy_pos = *sim.world.get_component::<Position>(enemy).unwrap();
        let health_before = crate::game::get_player_health(&sim.world);
        run(&mut sim, &mut sc, 90); // 1.5 s frozen
        assert_eq!(sc.time(), t_frozen, "scenario clock frozen under the gate");
        assert!(
            !sc.step_fired("clock"),
            "absolute timers do not advance under a gate"
        );
        let now = *sim.world.get_component::<Position>(enemy).unwrap();
        assert_eq!((now.x, now.y), (enemy_pos.x, enemy_pos.y), "enemy pinned");
        assert_eq!(
            crate::game::get_player_health(&sim.world),
            health_before,
            "no enemy attacks under the freeze"
        );

        // The PLAYER still moves under the freeze (to close the distance).
        let before = sim.player_position().unwrap();
        sim.set_player_velocity(Vec2::new(0.0, 200.0));
        run(&mut sim, &mut sc, 6);
        assert!(sim.player_position().unwrap().y > before.y + 10.0);

        // The punch connects: the gate releases, the post-gate actions run.
        teleport(&mut sim, Vec2::new(650.0, 620.0));
        sim.set_player_velocity(Vec2::zero());
        assert!(sim.player_fire(Vec2::new(650.0, 650.0)), "punch landed");
        run(&mut sim, &mut sc, 1);
        assert!(sc.gate_view().is_none() || sc.gate_view().unwrap().input != GateInput::Punch);
        assert_eq!(sc.objective, "punched");
        assert!(
            sim.world.has_component::<crate::components::Stunned>(enemy),
            "knocked down"
        );

        // step_done chains only now: the FINISH gate installs on the next
        // tick, and the after-teach timer counts from the RELEASE.
        run(&mut sim, &mut sc, 1);
        assert!(sc.step_fired("next"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Finish);
        assert!(!sc.step_fired("late"));

        // The victim never gets back up under the freeze, however long the
        // player dawdles (KNOCKDOWN_SECS is 3).
        run(&mut sim, &mut sc, 300); // 5 s frozen
        assert!(sim.world.has_component::<crate::components::Stunned>(enemy));

        // Finish it: step over the body (the punch shoved it away) — the
        // finisher animation runs THROUGH the freeze.
        let ep = *sim.world.get_component::<Position>(enemy).unwrap();
        teleport(&mut sim, Vec2::new(ep.x - 30.0, ep.y));
        assert!(sim.player_finisher(), "finisher started on the downed bot");
        run(&mut sim, &mut sc, 60);
        assert!(sc.gate_view().is_none(), "finish gate released");
        assert_eq!(sc.objective, "finished");
        assert!(sim.world.get_component::<Health>(enemy).unwrap().is_dead());

        // Unfrozen again: both timers resume from the frozen clock.
        run(&mut sim, &mut sc, 40); // ~0.66 s
        assert!(sc.step_fired("late"), "0.5 s after teach's gate release");
        assert!(sc.step_fired("clock"));
    }

    #[test]
    fn gate_skip_releases_like_a_success() {
        let (mut sim, mut sc) = sim_for(&GT_FLOOR);
        teleport(&mut sim, Vec2::new(650.0, 620.0));
        run(&mut sim, &mut sc, 1);
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Punch);
        sc.gate_skip(&mut sim.world);
        assert!(sc.gate_view().is_none());
        assert_eq!(sc.objective, "punched");
        run(&mut sim, &mut sc, 1);
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Finish);
        sc.gate_skip(&mut sim.world);
        assert_eq!(sc.objective, "finished");
        run(&mut sim, &mut sc, 40);
        assert!(sc.step_fired("late"));
    }

    #[test]
    fn checkpoint_restores_the_run_on_death() {
        let (mut sim, mut sc) = sim_for(&GT_FLOOR);
        teleport(&mut sim, Vec2::new(650.0, 620.0));
        let (cp_world, cp_sc) = run(&mut sim, &mut sc, 1).expect("checkpoint requested");

        // Play on: punch the bot down, then "die".
        assert!(sim.player_fire(Vec2::new(650.0, 650.0)));
        run(&mut sim, &mut sc, 2);
        assert_eq!(sc.objective, "punched");
        let player = sim.player().unwrap();
        sim.world
            .get_component_mut::<Health>(player)
            .unwrap()
            .take_damage(9999);
        assert!(!sim.player_alive());

        // Death -> restore the snapshot: the run is back at the gate.
        sim.world = cp_world.clone();
        sc = cp_sc.clone();
        assert!(sim.player_alive());
        assert_eq!(
            crate::game::get_player_health(&sim.world),
            100,
            "health restored"
        );
        assert!(
            sim.world
                .get_component::<Weapon>(sim.player().unwrap())
                .is_none(),
            "still disarmed, as snapshotted"
        );
        assert!(
            sc.step_fired("teach"),
            "fired steps travel with the snapshot"
        );
        assert!(!sc.step_fired("next"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Punch);
        assert_eq!(sc.objective, "start", "post-gate objective rolled back");
        let enemy = sim.world.query::<Enemy>()[0];
        assert!(
            !sim.world.has_component::<crate::components::Stunned>(enemy),
            "the knockdown after the snapshot rolled back"
        );
        let pos = sim.world.get_component::<Position>(enemy).unwrap();
        assert_eq!((pos.x, pos.y), (650.0, 650.0), "enemy back at its spawn");
        // The restored run plays out normally.
        assert!(sim.player_fire(Vec2::new(650.0, 650.0)));
        run(&mut sim, &mut sc, 2);
        assert_eq!(sc.objective, "punched");
    }

    #[test]
    fn enter_zone_before_disarms_once_the_other_step_fires() {
        const B_STEPS: [StepDef; 2] = [
            StepDef {
                id: "blocked",
                trigger: Trigger::EnterZone {
                    zone: "z",
                    before: Some("point"),
                },
                actions: &[Action::Objective("blocked")],
            },
            StepDef {
                id: "point",
                trigger: Trigger::Timer {
                    seconds: 1.0,
                    after: None,
                },
                actions: &[Action::Sfx("pt")],
            },
        ];
        const B_FLOOR: FloorDef = FloorDef {
            spawns: &[],
            scenario: &B_STEPS,
            ..T_FLOOR
        };
        // Entering the zone BEFORE the point fires the step...
        let mut world = world_for(&B_FLOOR);
        let mut sc = ScenarioState::new(&B_FLOOR);
        move_player(&mut world, Vec2::new(650.0, 650.0));
        sc.tick(&mut world, 0.016);
        assert!(sc.step_fired("blocked"));

        // ...but once the point has fired, the step is disarmed forever.
        let mut world = world_for(&B_FLOOR);
        let mut sc = ScenarioState::new(&B_FLOOR);
        while sc.time() < 1.1 {
            sc.tick(&mut world, 0.1);
        }
        assert!(sc.step_fired("point"));
        move_player(&mut world, Vec2::new(650.0, 650.0));
        for _ in 0..20 {
            sc.tick(&mut world, 0.016);
        }
        assert!(!sc.step_fired("blocked"));
    }

    #[test]
    fn floor_1_tutorial_plays_through_headlessly() {
        let floor = crate::levels::floor_def(1);
        let mut sim = Simulation::new(1);
        let mut sc = ScenarioState::new(floor);
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("intro"));
        assert_eq!(sim.enemies_alive(), 4, "the passive lobby crowd");

        // Straight to the desk: the cover-blown conversation opens.
        teleport(&mut sim, Vec2::new(500.0, 420.0));
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("desk"));
        assert!(sc.dialogue_active());
        // Click through the CORRUPTOR scene.
        for _ in 0..30 {
            if !sc.dialogue_active() {
                break;
            }
            sc.dialogue_advance();
            run(&mut sim, &mut sc, 5);
        }
        assert!(!sc.dialogue_active(), "conversation dismissed");

        // 0.4 s later: disarm + checkpoint + the lone SENTINEL + PUNCH gate.
        let cp1 = run(&mut sim, &mut sc, 40);
        assert!(sc.step_fired("tut_punch"));
        assert!(cp1.is_some(), "first checkpoint taken");
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Punch);
        assert!(sim
            .world
            .get_component::<Weapon>(sim.player().unwrap())
            .is_none());
        let t_frozen = sc.time();
        run(&mut sim, &mut sc, 30);
        assert_eq!(sc.time(), t_frozen, "frozen on the prompt");

        // PUNCH the rushing SENTINEL (spawned at 580,380).
        teleport(&mut sim, Vec2::new(545.0, 380.0));
        assert!(sim.player_fire(Vec2::new(580.0, 380.0)));
        run(&mut sim, &mut sc, 2);
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Finish);

        // FINISH it (pound — bare hands): step over the shoved body first.
        let downed1 = sim
            .world
            .query::<Enemy>()
            .into_iter()
            .find(|&e| sim.world.has_component::<crate::components::Stunned>(e))
            .expect("the punched SENTINEL is down");
        let d1 = *sim.world.get_component::<Position>(downed1).unwrap();
        teleport(&mut sim, Vec2::new(d1.x - 30.0, d1.y));
        assert!(sim.player_finisher());
        run(&mut sim, &mut sc, 70);
        assert!(sc.step_fired("tut_bar"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Pickup);

        // THE TRAP THAT SOFT-LOCKED REAL PLAYERS: pressing E right where
        // the finisher victim died must grab NOTHING — tutorial victims
        // spawn `unarmed`, so no corpse pistol can release the pickup gate
        // and then dead-end the strike gate (a strike gate masks gun clicks).
        assert_eq!(sim.player_pickup(), None, "the corpse dropped nothing");
        run(&mut sim, &mut sc, 2);
        assert_eq!(
            sc.gate_view().unwrap().input,
            GateInput::Pickup,
            "the pickup gate is still waiting for the bar"
        );

        // TAKE THE BAR (the melee pickup by the desk).
        teleport(&mut sim, Vec2::new(420.0, 370.0));
        assert_eq!(sim.player_pickup(), Some(WeaponType::Melee));
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("tut_strike"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Strike);

        // SWING THE BAR at the second bot (600,320).
        teleport(&mut sim, Vec2::new(560.0, 320.0));
        assert!(sim.player_fire(Vec2::new(600.0, 320.0)));
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("tut_throw"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Throw);

        // THROW THE BAR at the third bot (400,300): flies under the freeze.
        teleport(&mut sim, Vec2::new(340.0, 300.0));
        assert!(sim.player_throw(Vec2::new(400.0, 300.0)));
        run(&mut sim, &mut sc, 30);
        assert!(sc.step_fired("tut_retrieve"), "the throw connected");
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Pickup);

        // GET IT BACK: the bar landed as a pickup where it struck.
        let bar = sim
            .world
            .query::<crate::components::WeaponPickup>()
            .into_iter()
            .find(|&e| {
                sim.world
                    .get_component::<crate::components::WeaponPickup>(e)
                    .is_some_and(|p| p.weapon_type == WeaponType::Melee)
            })
            .expect("the thrown bar is on the floor");
        let bar_pos = *sim.world.get_component::<Position>(bar).unwrap();
        teleport(&mut sim, Vec2::new(bar_pos.x, bar_pos.y));
        assert_eq!(sim.player_pickup(), Some(WeaponType::Melee));
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("tut_overhead"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Finish);

        // OVERHEAD on the downed bot (kept down by the frozen knockdown).
        let downed = sim
            .world
            .query::<Enemy>()
            .into_iter()
            .find(|&e| {
                sim.world.has_component::<crate::components::Stunned>(e)
                    && sim
                        .world
                        .get_component::<Health>(e)
                        .is_some_and(|h| h.is_alive())
            })
            .expect("the thrown-down bot");
        let dp = *sim.world.get_component::<Position>(downed).unwrap();
        teleport(&mut sim, Vec2::new(dp.x - 30.0, dp.y));
        assert!(sim.player_finisher());
        let cp2 = run(&mut sim, &mut sc, 80);
        assert!(sc.gate_view().is_none(), "tutorial complete");
        assert!(sc.step_fired("wake"));
        let cp2 = cp2.expect("second checkpoint at the free wave");
        assert_eq!(
            sim.enemies_alive(),
            7,
            "4 alerted lobby bots + the free wave of 3"
        );
        assert_eq!(sc.objective, "They know. Purge reception.");

        // Die in the free fight -> restore: straight back to the wave.
        let player = sim.player().unwrap();
        sim.world
            .get_component_mut::<Health>(player)
            .unwrap()
            .take_damage(9999);
        assert!(!sim.player_alive());
        sim.world = cp2.0.clone();
        sc = cp2.1.clone();
        assert!(sim.player_alive());
        assert!(sc.step_fired("wake"));
        assert_eq!(sim.enemies_alive(), 7, "the wave is inside the snapshot");
        assert_eq!(
            crate::game::get_player_weapon(&sim.world),
            Some(WeaponType::Melee),
            "the bar came back with the snapshot"
        );

        // Clear the floor: the lift opens.
        crate::game::purge_all_enemies(&mut sim.world);
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("clear"));
        assert!(exit_open(&sim.world, "lift"));
    }

    /// Regression for the floor-1 "LEFT CLICK — SWING THE BAR" soft-lock:
    /// replays the WHOLE tutorial through the browser's own gate input
    /// dispatch ([`crate::game::gated_player_input`] — the exact function
    /// `lib.rs` forwards `input::` state to), with realistic play: the player
    /// WALKS (velocity + the movement system, frame by frame, under the
    /// freeze) instead of being teleported onto each target, and both melee
    /// gates are first attempted from ~90 px — the distance at which two
    /// 60 px robot sprites LOOK adjacent on screen, where the pre-fix 50 px
    /// reach whiffed with no feedback and the gate never released.
    #[test]
    fn floor_1_tutorial_plays_through_the_browser_gate_dispatch() {
        use crate::components::Position;
        use crate::game::{gated_player_input, get_player_weapon, PlayerIntents};

        /// One browser frame: the gate dispatch (if a gate is up), then the
        /// engine tick + scenario tick + gate notify — the `lib.rs` order.
        fn frame(sim: &mut Simulation, sc: &mut ScenarioState, intents: &PlayerIntents) {
            if let Some(g) = sc.gate_view() {
                gated_player_input(&mut sim.world, g, intents);
            }
            sim.scenario_step(sc, DT);
        }

        fn idle() -> PlayerIntents {
            PlayerIntents {
                left_pressed: false,
                left_down: false,
                right_pressed: false,
                e_pressed: false,
                mouse_world: Vec2::zero(),
            }
        }
        fn swing_at(mouse: Vec2) -> PlayerIntents {
            PlayerIntents {
                left_pressed: true,
                left_down: true,
                mouse_world: mouse,
                ..idle()
            }
        }
        fn press_e() -> PlayerIntents {
            PlayerIntents {
                e_pressed: true,
                ..idle()
            }
        }
        fn throw_at(mouse: Vec2) -> PlayerIntents {
            PlayerIntents {
                right_pressed: true,
                mouse_world: mouse,
                ..idle()
            }
        }

        /// Walk the player to `to` at full speed, one frame at a time (the
        /// movement system does the moving — under a gate freeze too, like
        /// the browser). Holds / conversations lock movement, as `lib.rs`
        /// does.
        fn walk_to(sim: &mut Simulation, sc: &mut ScenarioState, to: Vec2) {
            for _ in 0..1500 {
                if sc.hold_active() || sc.dialogue_active() {
                    sim.set_player_velocity(Vec2::zero());
                    sim.scenario_step(sc, DT);
                    continue;
                }
                let pos = sim.player_position().unwrap();
                if (to - pos).length() <= 4.0 {
                    break;
                }
                sim.set_player_velocity((to - pos).normalize() * 200.0);
                sim.scenario_step(sc, DT);
            }
            sim.set_player_velocity(Vec2::zero());
            let pos = sim.player_position().unwrap();
            assert!(
                (to - pos).length() <= 4.0,
                "walked to ({}, {}) but stopped at ({}, {})",
                to.x,
                to.y,
                pos.x,
                pos.y
            );
        }

        let floor = crate::levels::floor_def(1);
        let mut sim = Simulation::new(1);
        let mut sc = ScenarioState::new(floor);
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("intro"));

        // WALK from the main doors straight up to just short of the desk
        // (the old scanner-arch "SIGNATURE CHECK" hold was cut — walking in
        // stays uninterrupted until the desk scene).
        walk_to(&mut sim, &mut sc, Vec2::new(500.0, 455.0));
        // Step onto the desk: the takeover conversation opens.
        for _ in 0..60 {
            if sc.dialogue_active() {
                break;
            }
            sim.set_player_velocity(Vec2::new(0.0, -200.0));
            sim.scenario_step(&mut sc, DT);
        }
        sim.set_player_velocity(Vec2::zero());
        assert!(sc.step_fired("desk"));
        assert!(sc.dialogue_active());
        for _ in 0..30 {
            if !sc.dialogue_active() {
                break;
            }
            sc.dialogue_advance();
            run(&mut sim, &mut sc, 5);
        }
        assert!(!sc.dialogue_active());

        // 0.4 s later: disarm + checkpoint + the PUNCH gate on the SENTINEL
        // spawned at (580, 380).
        run(&mut sim, &mut sc, 40);
        assert!(sc.step_fired("tut_punch"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Punch);
        assert!(get_player_weapon(&sim.world).is_none(), "disarmed");

        // A jab from ~90 px — sprites LOOK adjacent — whiffs: no release.
        // (Approach from the north: the cone stays clear of the passive
        // civilians milling around the desk.)
        walk_to(&mut sim, &mut sc, Vec2::new(580.0, 290.0));
        frame(&mut sim, &mut sc, &swing_at(Vec2::new(580.0, 380.0)));
        assert_eq!(
            sc.gate_view().unwrap().input,
            GateInput::Punch,
            "a 90 px jab still misses"
        );
        // Fist cooldown, step up to 50 px, jab again: down it goes.
        run(&mut sim, &mut sc, 30);
        walk_to(&mut sim, &mut sc, Vec2::new(580.0, 330.0));
        frame(&mut sim, &mut sc, &swing_at(Vec2::new(580.0, 380.0)));
        run(&mut sim, &mut sc, 2); // the released gate's follow-up step fires
        assert!(sc.step_fired("tut_finish"), "the jab connected");
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Finish);

        // FINISH: walk over the shoved body and click.
        let downed1 = sim
            .world
            .query::<Enemy>()
            .into_iter()
            .find(|&e| sim.world.has_component::<crate::components::Stunned>(e))
            .expect("the punched SENTINEL is down");
        let d1 = *sim.world.get_component::<Position>(downed1).unwrap();
        walk_to(&mut sim, &mut sc, Vec2::new(d1.x - 40.0, d1.y));
        frame(&mut sim, &mut sc, &swing_at(Vec2::new(d1.x, d1.y)));
        for _ in 0..70 {
            frame(&mut sim, &mut sc, &idle());
        }
        assert!(sc.step_fired("tut_bar"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Pickup);

        // TAKE THE BAR: walk to the pickup by the desk and press E.
        walk_to(&mut sim, &mut sc, Vec2::new(420.0, 370.0));
        frame(&mut sim, &mut sc, &press_e());
        assert_eq!(get_player_weapon(&sim.world), Some(WeaponType::Melee));
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("tut_strike"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Strike);
        let strike_bot = sim
            .world
            .query::<Enemy>()
            .into_iter()
            .find(|&e| {
                sim.world
                    .get_component::<Position>(e)
                    .is_some_and(|p| (p.x - 600.0).abs() < 1.0 && (p.y - 320.0).abs() < 1.0)
            })
            .expect("the strike target spawned at (600, 320)");

        // THE BUG: swing the bar from ~90 px — where the two sprites already
        // look adjacent on screen. It whiffs (only the whoosh, no impact, no
        // feedback), exactly what soft-locked browser players when the reach
        // was 50 px and nothing hinted they had to step INSIDE the target.
        walk_to(&mut sim, &mut sc, Vec2::new(510.0, 320.0));
        frame(&mut sim, &mut sc, &swing_at(Vec2::new(600.0, 320.0)));
        assert_eq!(
            sc.gate_view().unwrap().input,
            GateInput::Strike,
            "a 90 px swing still whiffs"
        );
        // Weapon cooldown, then close to arm's length (~65 px, sprites
        // touching): the 70 px reach lands the swing WITHOUT demanding that
        // the player physically overlap the frozen target.
        run(&mut sim, &mut sc, 32);
        walk_to(&mut sim, &mut sc, Vec2::new(535.0, 320.0));
        frame(&mut sim, &mut sc, &swing_at(Vec2::new(600.0, 320.0)));
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("tut_throw"), "the strike released the gate");
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Throw);
        // The 100-damage swing killed the bot: its corpse fell along the
        // blow (attacker west of it -> fall east), head away from the player.
        assert!(sim
            .world
            .get_component::<Health>(strike_bot)
            .unwrap()
            .is_dead());
        let fall = sim
            .world
            .get_component::<crate::components::Stunned>(strike_bot)
            .expect("the killing swing recorded the corpse's fall")
            .fall_angle;
        assert!(fall.abs() < 0.01, "fell due east, got {fall}");

        // THROW THE BAR at the third bot (400, 300).
        walk_to(&mut sim, &mut sc, Vec2::new(340.0, 300.0));
        frame(&mut sim, &mut sc, &throw_at(Vec2::new(400.0, 300.0)));
        run(&mut sim, &mut sc, 30);
        assert!(sc.step_fired("tut_retrieve"), "the throw connected");
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Pickup);

        // GET IT BACK: walk to where the bar landed and press E.
        let bar = sim
            .world
            .query::<crate::components::WeaponPickup>()
            .into_iter()
            .find(|&e| {
                sim.world
                    .get_component::<crate::components::WeaponPickup>(e)
                    .is_some_and(|p| p.weapon_type == WeaponType::Melee)
            })
            .expect("the thrown bar is on the floor");
        let bar_pos = *sim.world.get_component::<Position>(bar).unwrap();
        walk_to(&mut sim, &mut sc, Vec2::new(bar_pos.x, bar_pos.y));
        frame(&mut sim, &mut sc, &press_e());
        run(&mut sim, &mut sc, 2);
        assert!(sc.step_fired("tut_overhead"));
        assert_eq!(sc.gate_view().unwrap().input, GateInput::Finish);

        // PUT IT DOWN: overhead finisher on the thrown-down bot.
        let downed = sim
            .world
            .query::<Enemy>()
            .into_iter()
            .find(|&e| {
                sim.world.has_component::<crate::components::Stunned>(e)
                    && sim
                        .world
                        .get_component::<Health>(e)
                        .is_some_and(|h| h.is_alive())
            })
            .expect("the thrown-down bot");
        let dp = *sim.world.get_component::<Position>(downed).unwrap();
        walk_to(&mut sim, &mut sc, Vec2::new(dp.x - 40.0, dp.y));
        frame(&mut sim, &mut sc, &swing_at(Vec2::new(dp.x, dp.y)));
        for _ in 0..90 {
            frame(&mut sim, &mut sc, &idle());
        }
        assert!(sc.gate_view().is_none(), "tutorial complete");
        assert!(sc.step_fired("wake"));
        assert_eq!(
            sim.enemies_alive(),
            7,
            "4 alerted lobby bots + the free wave of 3"
        );
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
