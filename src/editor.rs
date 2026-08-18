//! The native level editor's DOCUMENT model (`?viz` → LEVELS tab, drawn by
//! `editor_ui.rs`): an owned, editable copy of a floor, its undo / redo
//! history, the validation rules of `docs/SCENARIO_FORMAT.md` and a
//! hand-written JSON writer that produces `levels/floor_NN.json` in the
//! exact formatting the web editor (`tools/levels-editor-format.js`) writes
//! — so a floor loaded from the compiled `FloorDef` and saved untouched is
//! byte-for-byte the checked-in file. Pure Rust, no browser: host-tested.
//!
//! The spatial content (walls, rooms, zones, spawns, pickups, entry / exits,
//! placed props) is editable; the SCENARIO steps are carried through
//! verbatim (`&'static [StepDef]`) — the web editor owns those.

use crate::components::{EnemyType, WeaponType};
use crate::props::{prop_kind_id, PROP_COUNT};
use crate::scenario::{
    Action, AlertTarget, ElevatorKind, FloorDef, PropPlacement, Rect, StepDef, Surface, Trigger,
    SURFACE_EXIT,
};

/// Undo history depth (snapshots).
pub const UNDO_DEPTH: usize = 100;

/// An elevator car (the entry, or an exit).
#[derive(Clone, Debug, PartialEq)]
pub struct Car {
    pub id: String,
    pub label: String,
    pub rect: Rect,
    /// Exits: floor id this car leads to ([`SURFACE_EXIT`] = the surface /
    /// end of the run). Entry: unused.
    pub to: usize,
    pub open: bool,
    /// Lift car / sliding door / open gate (rendering only).
    pub kind: ElevatorKind,
}

/// An annotation-only room.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub id: String,
    pub label: String,
    pub rect: Rect,
}

/// An `enter_zone` trigger region.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub id: String,
    pub rect: Rect,
}

/// A rogue placement.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    pub x: f32,
    pub y: f32,
    pub kind: EnemyType,
    /// `"type": "passive"` civilian bot (kind = its `look`), see docs/SCENARIO_FORMAT.md.
    pub passive: bool,
    /// Passive only: zone id to stroll into.
    pub walk_to: Option<String>,
    /// Passive only: heading (degrees) to settle on.
    pub face: Option<f32>,
    /// `alert { "group": id }` group.
    pub group: Option<String>,
}

/// A weapon on the floor at level start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pickup {
    pub x: f32,
    pub y: f32,
    pub weapon: WeaponType,
}

/// One thing on the map that can be selected / moved / deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Entry,
    Exit(usize),
    Wall(usize),
    Room(usize),
    Zone(usize),
    Spawn(usize),
    Pickup(usize),
    Prop(usize),
}

impl Item {
    /// Whether the item is a rectangle (movable AND resizable) rather than a
    /// point (spawns, pickups) or a placed prop.
    pub fn is_rect(&self) -> bool {
        matches!(
            self,
            Item::Entry | Item::Exit(_) | Item::Wall(_) | Item::Room(_) | Item::Zone(_)
        )
    }

    /// Display name of the item kind.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Item::Entry => "ENTRY",
            Item::Exit(_) => "EXIT",
            Item::Wall(_) => "WALL",
            Item::Room(_) => "ROOM",
            Item::Zone(_) => "ZONE",
            Item::Spawn(_) => "SPAWN",
            Item::Pickup(_) => "PICKUP",
            Item::Prop(_) => "PROP",
        }
    }
}

/// The JSON id of an enemy type (`spawns[].type`).
pub fn enemy_type_id(t: EnemyType) -> &'static str {
    match t {
        EnemyType::Idle => "idle",
        EnemyType::Wandering => "wandering",
        EnemyType::Patrolling => "patrolling",
    }
}

/// The next enemy type in the SPAWN tool's cycle.
pub fn next_enemy_type(t: EnemyType) -> EnemyType {
    match t {
        EnemyType::Idle => EnemyType::Wandering,
        EnemyType::Wandering => EnemyType::Patrolling,
        EnemyType::Patrolling => EnemyType::Idle,
    }
}

/// The JSON id of a weapon (`pickups[].weapon`).
pub fn weapon_id(w: WeaponType) -> &'static str {
    match w {
        WeaponType::Pistol => "pistol",
        WeaponType::Shotgun => "shotgun",
        WeaponType::MachineGun => "machinegun",
        WeaponType::Melee => "melee",
    }
}

/// The next weapon in the PICKUP tool's cycle.
pub fn next_weapon(w: WeaponType) -> WeaponType {
    match w {
        WeaponType::Pistol => WeaponType::Shotgun,
        WeaponType::Shotgun => WeaponType::MachineGun,
        WeaponType::MachineGun => WeaponType::Melee,
        WeaponType::Melee => WeaponType::Pistol,
    }
}

/// The prop index for a JSON kind id (`rack_closed` → 0), if it exists.
pub fn prop_kind_index(id: &str) -> Option<usize> {
    (0..PROP_COUNT).find(|&k| prop_kind_id(k) == id)
}

/// The floor file name for a floor id (`floor_NN.json`, 14 = `floor_13h.json`).
/// The JSON name of a portal kind (`lift` | `door` | `gate`).
pub fn kind_name(k: ElevatorKind) -> &'static str {
    match k {
        ElevatorKind::Lift => "lift",
        ElevatorKind::Door => "door",
        ElevatorKind::Gate => "gate",
    }
}

/// The JSON name of a ground surface.
pub fn surface_name(s: Surface) -> &'static str {
    match s {
        Surface::Checker => "checker",
        Surface::Asphalt => "asphalt",
        Surface::Marble => "marble",
        Surface::Concrete => "concrete",
        Surface::Grating => "grating",
    }
}

pub fn floor_file_name(id: usize) -> String {
    if id == 14 {
        "floor_13h.json".to_string()
    } else {
        format!("floor_{id:02}.json")
    }
}

/// An owned, editable floor.
#[derive(Clone, Debug, PartialEq)]
pub struct EditableFloor {
    pub id: usize,
    pub name: String,
    pub theme: String,
    pub accent: String,
    pub flavor: String,
    pub objective: String,
    pub width: f32,
    pub height: f32,
    /// Ground rendering (`checker` = the default, omitted from the JSON).
    pub surface: Surface,
    pub entry: Car,
    pub exits: Vec<Car>,
    pub walls: Vec<Rect>,
    pub rooms: Vec<Room>,
    pub zones: Vec<Zone>,
    pub spawns: Vec<Spawn>,
    pub pickups: Vec<Pickup>,
    pub props: Vec<PropPlacement>,
    /// The scenario steps, verbatim (not edited here).
    pub scenario: &'static [StepDef],
}

/// Half-size of the hit box around a spawn / pickup marker (world units).
pub const POINT_HALF: f32 = 12.0;

impl EditableFloor {
    /// An editable copy of a compiled floor.
    pub fn from_def(f: &FloorDef) -> Self {
        let car = |e: &crate::scenario::ElevatorDef| Car {
            id: e.id.to_string(),
            label: e.label.to_string(),
            rect: e.rect,
            to: e.to,
            open: e.open,
            kind: e.kind,
        };
        EditableFloor {
            id: f.id,
            name: f.name.to_string(),
            theme: f.theme.to_string(),
            accent: f.accent.to_string(),
            flavor: f.flavor.to_string(),
            objective: f.objective.to_string(),
            width: f.width,
            height: f.height,
            surface: f.surface,
            entry: car(&f.entry),
            exits: f.exits.iter().map(car).collect(),
            walls: f.walls.to_vec(),
            rooms: f
                .rooms
                .iter()
                .map(|r| Room {
                    id: r.id.to_string(),
                    label: r.label.to_string(),
                    rect: r.rect,
                })
                .collect(),
            zones: f
                .zones
                .iter()
                .map(|z| Zone {
                    id: z.id.to_string(),
                    rect: z.rect,
                })
                .collect(),
            spawns: f
                .spawns
                .iter()
                .map(|s| Spawn {
                    x: s.x,
                    y: s.y,
                    kind: s.kind,
                    passive: s.passive,
                    walk_to: s.walk_to.map(str::to_string),
                    face: s.face,
                    group: s.group.map(str::to_string),
                })
                .collect(),
            pickups: f
                .pickups
                .iter()
                .map(|p| Pickup {
                    x: p.x,
                    y: p.y,
                    weapon: p.weapon,
                })
                .collect(),
            props: f.props.to_vec(),
            scenario: f.scenario,
        }
    }

    /// `levels/<file>` this floor is saved as.
    pub fn file_name(&self) -> String {
        floor_file_name(self.id)
    }

    // ---- items --------------------------------------------------------

    /// The bounding rectangle of an item (points get a small box; props their
    /// unrotated square). `None` if the index is stale.
    pub fn rect_of(&self, item: Item) -> Option<Rect> {
        Some(match item {
            Item::Entry => self.entry.rect,
            Item::Exit(i) => self.exits.get(i)?.rect,
            Item::Wall(i) => *self.walls.get(i)?,
            Item::Room(i) => self.rooms.get(i)?.rect,
            Item::Zone(i) => self.zones.get(i)?.rect,
            Item::Spawn(i) => {
                let s = self.spawns.get(i)?;
                Rect::new(
                    s.x - POINT_HALF,
                    s.y - POINT_HALF,
                    2.0 * POINT_HALF,
                    2.0 * POINT_HALF,
                )
            }
            Item::Pickup(i) => {
                let p = self.pickups.get(i)?;
                Rect::new(
                    p.x - POINT_HALF,
                    p.y - POINT_HALF,
                    2.0 * POINT_HALF,
                    2.0 * POINT_HALF,
                )
            }
            Item::Prop(i) => {
                let p = self.props.get(i)?;
                let h = p.size / 2.0;
                Rect::new(p.x - h, p.y - h, p.size, p.size)
            }
        })
    }

    /// Set the rectangle of a rect item (`Item::is_rect`); points and props
    /// are re-centred on it. Sizes are clamped to at least 1 unit.
    pub fn set_rect(&mut self, item: Item, r: Rect) {
        let r = Rect::new(r.x, r.y, r.w.max(1.0), r.h.max(1.0));
        match item {
            Item::Entry => self.entry.rect = r,
            Item::Exit(i) => {
                if let Some(e) = self.exits.get_mut(i) {
                    e.rect = r;
                }
            }
            Item::Wall(i) => {
                if let Some(w) = self.walls.get_mut(i) {
                    *w = r;
                }
            }
            Item::Room(i) => {
                if let Some(x) = self.rooms.get_mut(i) {
                    x.rect = r;
                }
            }
            Item::Zone(i) => {
                if let Some(z) = self.zones.get_mut(i) {
                    z.rect = r;
                }
            }
            Item::Spawn(_) | Item::Pickup(_) | Item::Prop(_) => {
                let c = r.center();
                self.set_position(item, c.x, c.y);
            }
        }
    }

    /// Move an item so its centre (points / props) or its origin (rects, via
    /// `translate`) lands on `(x, y)`.
    pub fn set_position(&mut self, item: Item, x: f32, y: f32) {
        match item {
            Item::Spawn(i) => {
                if let Some(s) = self.spawns.get_mut(i) {
                    s.x = x;
                    s.y = y;
                }
            }
            Item::Pickup(i) => {
                if let Some(p) = self.pickups.get_mut(i) {
                    p.x = x;
                    p.y = y;
                }
            }
            Item::Prop(i) => {
                if let Some(p) = self.props.get_mut(i) {
                    p.x = x;
                    p.y = y;
                }
            }
            _ => {
                if let Some(r) = self.rect_of(item) {
                    self.set_rect(item, Rect::new(x, y, r.w, r.h));
                }
            }
        }
    }

    /// Shift an item by `(dx, dy)`.
    pub fn translate(&mut self, item: Item, dx: f32, dy: f32) {
        match item {
            Item::Spawn(_) | Item::Pickup(_) | Item::Prop(_) => {
                if let Some(r) = self.rect_of(item) {
                    let c = r.center();
                    self.set_position(item, c.x + dx, c.y + dy);
                }
            }
            _ => {
                if let Some(r) = self.rect_of(item) {
                    self.set_rect(item, Rect::new(r.x + dx, r.y + dy, r.w, r.h));
                }
            }
        }
    }

    /// Delete an item (the entry cannot be deleted). Returns whether
    /// anything was removed.
    pub fn delete(&mut self, item: Item) -> bool {
        fn take<T>(v: &mut Vec<T>, i: usize) -> bool {
            if i < v.len() {
                v.remove(i);
                true
            } else {
                false
            }
        }
        match item {
            Item::Entry => false,
            Item::Exit(i) => take(&mut self.exits, i),
            Item::Wall(i) => take(&mut self.walls, i),
            Item::Room(i) => take(&mut self.rooms, i),
            Item::Zone(i) => take(&mut self.zones, i),
            Item::Spawn(i) => take(&mut self.spawns, i),
            Item::Pickup(i) => take(&mut self.pickups, i),
            Item::Prop(i) => take(&mut self.props, i),
        }
    }

    /// The topmost item under a world point: small things first (props,
    /// spawns, pickups), then the cars, zones, rooms, and walls last.
    pub fn hit_test(&self, p: crate::math::Vec2) -> Option<Item> {
        let hit = |item: Item| self.rect_of(item).is_some_and(|r| r.contains(p));
        // Later-placed props sit on top.
        if let Some(i) = (0..self.props.len()).rev().find(|&i| hit(Item::Prop(i))) {
            return Some(Item::Prop(i));
        }
        if let Some(i) = (0..self.spawns.len()).rev().find(|&i| hit(Item::Spawn(i))) {
            return Some(Item::Spawn(i));
        }
        if let Some(i) = (0..self.pickups.len())
            .rev()
            .find(|&i| hit(Item::Pickup(i)))
        {
            return Some(Item::Pickup(i));
        }
        if let Some(i) = (0..self.exits.len()).rev().find(|&i| hit(Item::Exit(i))) {
            return Some(Item::Exit(i));
        }
        if hit(Item::Entry) {
            return Some(Item::Entry);
        }
        if let Some(i) = (0..self.walls.len()).rev().find(|&i| hit(Item::Wall(i))) {
            return Some(Item::Wall(i));
        }
        // Zones / rooms are big: prefer the SMALLEST one under the cursor so
        // nested regions stay reachable.
        let smallest = |items: Vec<Item>| {
            items.into_iter().filter(|&it| hit(it)).min_by(|a, b| {
                let (ra, rb) = (self.rect_of(*a).unwrap(), self.rect_of(*b).unwrap());
                (ra.w * ra.h).total_cmp(&(rb.w * rb.h))
            })
        };
        if let Some(z) = smallest((0..self.zones.len()).map(Item::Zone).collect()) {
            return Some(z);
        }
        smallest((0..self.rooms.len()).map(Item::Room).collect())
    }

    /// A short description of an item for the properties strip.
    pub fn describe(&self, item: Item) -> String {
        match item {
            Item::Entry => format!("ENTRY  \"{}\"", self.entry.label),
            Item::Exit(i) => self
                .exits
                .get(i)
                .map(|e| {
                    format!(
                        "EXIT {}  \"{}\"  -> {}  {}",
                        e.id,
                        e.label,
                        if e.to == SURFACE_EXIT {
                            "SURFACE".to_string()
                        } else {
                            format!("F{}", e.to)
                        },
                        if e.open { "open" } else { "closed" }
                    )
                })
                .unwrap_or_default(),
            Item::Wall(i) => format!("WALL #{i}"),
            Item::Room(i) => self
                .rooms
                .get(i)
                .map(|r| format!("ROOM {}  \"{}\"", r.id, r.label))
                .unwrap_or_default(),
            Item::Zone(i) => self
                .zones
                .get(i)
                .map(|z| format!("ZONE {}", z.id))
                .unwrap_or_default(),
            Item::Spawn(i) => self
                .spawns
                .get(i)
                .map(|s| format!("SPAWN {}", enemy_type_id(s.kind)))
                .unwrap_or_default(),
            Item::Pickup(i) => self
                .pickups
                .get(i)
                .map(|p| format!("PICKUP {}", weapon_id(p.weapon)))
                .unwrap_or_default(),
            Item::Prop(i) => self
                .props
                .get(i)
                .map(|p| {
                    format!(
                        "PROP {}  rot {}  size {}",
                        prop_kind_id(p.kind),
                        p.rot,
                        p.size
                    )
                })
                .unwrap_or_default(),
        }
    }

    // ---- creation ----------------------------------------------------

    /// `prefix1`, `prefix2`, … — the first id not used by any exit / room /
    /// zone of this floor.
    pub fn unique_id(&self, prefix: &str) -> String {
        let used = |id: &str| {
            self.exits.iter().any(|e| e.id == id)
                || self.rooms.iter().any(|r| r.id == id)
                || self.zones.iter().any(|z| z.id == id)
        };
        (1..)
            .map(|n| format!("{prefix}{n}"))
            .find(|id| !used(id))
            .unwrap()
    }

    pub fn add_wall(&mut self, r: Rect) -> Item {
        self.walls.push(r);
        Item::Wall(self.walls.len() - 1)
    }

    pub fn add_room(&mut self, r: Rect) -> Item {
        let id = self.unique_id("room");
        let label = id.to_uppercase();
        self.rooms.push(Room { id, label, rect: r });
        Item::Room(self.rooms.len() - 1)
    }

    pub fn add_zone(&mut self, r: Rect) -> Item {
        let id = self.unique_id("zone");
        self.zones.push(Zone { id, rect: r });
        Item::Zone(self.zones.len() - 1)
    }

    /// A new closed exit leading to the next floor (the surface after 13½).
    pub fn add_exit(&mut self, r: Rect) -> Item {
        let id = self.unique_id("exit");
        let to = if self.id >= 14 {
            SURFACE_EXIT
        } else {
            self.id + 1
        };
        self.exits.push(Car {
            label: id.to_uppercase(),
            id,
            rect: r,
            to,
            open: false,
            kind: ElevatorKind::Lift,
        });
        Item::Exit(self.exits.len() - 1)
    }

    pub fn add_spawn(&mut self, x: f32, y: f32, kind: EnemyType) -> Item {
        self.spawns.push(Spawn {
            x,
            y,
            kind,
            passive: false,
            walk_to: None,
            face: None,
            group: None,
        });
        Item::Spawn(self.spawns.len() - 1)
    }

    pub fn add_pickup(&mut self, x: f32, y: f32, weapon: WeaponType) -> Item {
        self.pickups.push(Pickup { x, y, weapon });
        Item::Pickup(self.pickups.len() - 1)
    }

    pub fn add_prop(&mut self, p: PropPlacement) -> Item {
        self.props.push(p);
        Item::Prop(self.props.len() - 1)
    }

    // ---- validation --------------------------------------------------

    /// The problems that would make `tools/gen_levels.py` reject the floor
    /// (plus a few editor sanity checks). `known_ids` = every floor id in
    /// `levels/index.json` (exit targets must be one of them, or 0).
    pub fn validate(&self, known_ids: &[usize]) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() {
            out.push("name is empty".into());
        }
        if self.width <= 0.0 || self.height <= 0.0 {
            out.push("size must be positive".into());
        }
        if self.exits.is_empty() {
            out.push("at least one exit is required".into());
        }
        let inside = |r: &Rect| {
            r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= self.width && r.y + r.h <= self.height
        };
        if !inside(&self.entry.rect) {
            out.push("entry is outside the floor".into());
        }
        let mut seen: Vec<&str> = Vec::new();
        for e in &self.exits {
            if e.id.is_empty() {
                out.push("an exit has an empty id".into());
            } else if seen.contains(&e.id.as_str()) {
                out.push(format!("duplicate exit id \"{}\"", e.id));
            }
            seen.push(&e.id);
            if e.to != SURFACE_EXIT && !known_ids.contains(&e.to) {
                out.push(format!("exit \"{}\" leads to unknown floor {}", e.id, e.to));
            }
            if !inside(&e.rect) {
                out.push(format!("exit \"{}\" is outside the floor", e.id));
            }
        }
        seen.clear();
        for z in &self.zones {
            if z.id.is_empty() {
                out.push("a zone has an empty id".into());
            } else if seen.contains(&z.id.as_str()) {
                out.push(format!("duplicate zone id \"{}\"", z.id));
            }
            seen.push(&z.id);
        }
        seen.clear();
        for r in &self.rooms {
            if r.id.is_empty() {
                out.push("a room has an empty id".into());
            } else if seen.contains(&r.id.as_str()) {
                out.push(format!("duplicate room id \"{}\"", r.id));
            }
            seen.push(&r.id);
        }
        for p in &self.props {
            if p.kind >= PROP_COUNT {
                out.push(format!("prop #{} has an unknown kind", p.kind));
            }
            if p.size <= 0.0 {
                out.push(format!("prop {} has size <= 0", prop_kind_id(p.kind)));
            }
        }
        // Scenario references into the spatial content edited here.
        let has_exit = |id: &str| self.exits.iter().any(|e| e.id == id);
        let has_zone = |id: &str| self.zones.iter().any(|z| z.id == id);
        for s in self.scenario {
            match s.trigger {
                Trigger::EnterZone(z) if !has_zone(z) => {
                    out.push(format!("step \"{}\": zone \"{}\" does not exist", s.id, z))
                }
                Trigger::ExitOpen(Some(e)) if !has_exit(e) => {
                    out.push(format!("step \"{}\": exit \"{}\" does not exist", s.id, e))
                }
                _ => {}
            }
            for a in s.actions {
                if let Action::OpenExit(e) | Action::CloseExit(e) = a {
                    if !has_exit(e) {
                        out.push(format!("step \"{}\": exit \"{}\" does not exist", s.id, e));
                    }
                }
            }
        }
        // Actors must not start inside a wall (the engine's level tests).
        let in_wall = |x: f32, y: f32, r: f32| {
            self.walls.iter().any(|w| {
                let cx = x.clamp(w.x, w.x + w.w);
                let cy = y.clamp(w.y, w.y + w.h);
                (cx - x) * (cx - x) + (cy - y) * (cy - y) < r * r
            })
        };
        let ps = self.entry.rect.center();
        if in_wall(ps.x, ps.y, 25.0) {
            out.push("the player spawn (entry centre) overlaps a wall".into());
        }
        for (i, s) in self.spawns.iter().enumerate() {
            if in_wall(s.x, s.y, 12.0) {
                out.push(format!("spawn #{i} overlaps a wall"));
            }
        }
        out
    }

    // ---- JSON ---------------------------------------------------------

    /// The floor as a JSON tree in the documented key order
    /// (`docs/SCENARIO_FORMAT.md`; the web editor's `ORDER` table).
    pub fn to_json_value(&self) -> Json {
        use Json::*;
        let s = |v: &str| Str(v.to_string());
        let n = |v: f32| Num(v);
        let rect_kv = |r: &Rect| {
            vec![
                ("x".into(), n(r.x)),
                ("y".into(), n(r.y)),
                ("w".into(), n(r.w)),
                ("h".into(), n(r.h)),
            ]
        };
        let mut entry = rect_kv(&self.entry.rect);
        entry.push(("label".into(), s(&self.entry.label)));
        if self.entry.kind != ElevatorKind::Lift {
            entry.push(("kind".into(), s(kind_name(self.entry.kind))));
        }
        entry.push(("id".into(), s(&self.entry.id)));
        let exits = self
            .exits
            .iter()
            .map(|e| {
                let mut kv = vec![("id".to_string(), s(&e.id))];
                kv.extend(rect_kv(&e.rect));
                kv.push(("label".into(), s(&e.label)));
                kv.push((
                    "to".into(),
                    if e.to == SURFACE_EXIT {
                        s("surface")
                    } else {
                        Num(e.to as f32)
                    },
                ));
                kv.push(("open".into(), Bool(e.open)));
                if e.kind != ElevatorKind::Lift {
                    kv.push(("kind".into(), s(kind_name(e.kind))));
                }
                Obj(kv)
            })
            .collect();
        let walls = self.walls.iter().map(|w| Obj(rect_kv(w))).collect();
        let rooms = self
            .rooms
            .iter()
            .map(|r| {
                let mut kv = vec![("id".to_string(), s(&r.id)), ("label".into(), s(&r.label))];
                kv.extend(rect_kv(&r.rect));
                Obj(kv)
            })
            .collect();
        let zones = self
            .zones
            .iter()
            .map(|z| {
                let mut kv = vec![("id".to_string(), s(&z.id))];
                kv.extend(rect_kv(&z.rect));
                Obj(kv)
            })
            .collect();
        // Spawns: hostile = {x, y, type}; passive civilians = {x, y, type: "passive",
        // look, walk_to?, face?}; either may carry a `group`.
        let spawn_json = |x: f32,
                          y: f32,
                          kind: EnemyType,
                          passive: bool,
                          walk_to: Option<&str>,
                          face: Option<f32>,
                          group: Option<&str>| {
            let mut kv = vec![("x".to_string(), n(x)), ("y".into(), n(y))];
            if passive {
                kv.push(("type".into(), s("passive")));
                if let Some(z) = walk_to {
                    kv.push(("walk_to".into(), s(z)));
                }
                if let Some(f) = face {
                    kv.push(("face".into(), n(f)));
                }
                kv.push(("look".into(), s(enemy_type_id(kind))));
            } else {
                kv.push(("type".into(), s(enemy_type_id(kind))));
            }
            if let Some(g) = group {
                kv.push(("group".into(), s(g)));
            }
            Obj(kv)
        };
        let spawns = self
            .spawns
            .iter()
            .map(|sp| {
                spawn_json(
                    sp.x,
                    sp.y,
                    sp.kind,
                    sp.passive,
                    sp.walk_to.as_deref(),
                    sp.face,
                    sp.group.as_deref(),
                )
            })
            .collect();
        let pickups = self
            .pickups
            .iter()
            .map(|p| {
                Obj(vec![
                    ("x".into(), n(p.x)),
                    ("y".into(), n(p.y)),
                    ("weapon".into(), s(weapon_id(p.weapon))),
                ])
            })
            .collect();
        let props: Vec<Json> = self
            .props
            .iter()
            .map(|p| {
                Obj(vec![
                    ("kind".into(), s(&prop_kind_id(p.kind))),
                    ("x".into(), n(p.x)),
                    ("y".into(), n(p.y)),
                    ("rot".into(), n(p.rot)),
                    ("size".into(), n(p.size)),
                ])
            })
            .collect();
        let scenario = self
            .scenario
            .iter()
            .map(|st| {
                let trigger = match st.trigger {
                    Trigger::Start => vec![("kind".to_string(), s("start"))],
                    Trigger::EnterZone(z) => {
                        vec![("kind".into(), s("enter_zone")), ("zone".into(), s(z))]
                    }
                    Trigger::Kills(c) => {
                        vec![("kind".into(), s("kills")), ("count".into(), Num(c as f32))]
                    }
                    Trigger::AllDead => vec![("kind".into(), s("all_dead"))],
                    Trigger::Timer { seconds, after } => {
                        let mut kv = vec![
                            ("kind".to_string(), s("timer")),
                            ("seconds".into(), n(seconds)),
                        ];
                        if let Some(a) = after {
                            kv.push(("after".into(), s(a)));
                        }
                        kv
                    }
                    Trigger::ExitOpen(e) => {
                        let mut kv = vec![("kind".to_string(), s("exit_open"))];
                        if let Some(e) = e {
                            kv.push(("exit".into(), s(e)));
                        }
                        kv
                    }
                    Trigger::StepDone(x) => {
                        vec![("kind".into(), s("step_done")), ("step".into(), s(x))]
                    }
                    Trigger::BossDead => vec![("kind".into(), s("boss_dead"))],
                    Trigger::Extracted => vec![("kind".into(), s("extracted"))],
                };
                let actions = st
                    .actions
                    .iter()
                    .map(|a| match a {
                        Action::Say(say) => {
                            let mut kv = vec![
                                ("who".to_string(), s(say.who)),
                                ("text".into(), s(say.text)),
                            ];
                            if say.delay != 0.0 {
                                kv.push(("delay".into(), n(say.delay)));
                            }
                            Obj(vec![("say".into(), Obj(kv))])
                        }
                        Action::Talk(t) => Obj(vec![(
                            "talk".into(),
                            Obj(vec![("who".into(), s(t.who)), ("text".into(), s(t.text))]),
                        )]),
                        Action::Spawn(wave) => Obj(vec![(
                            "spawn".into(),
                            Arr(wave
                                .iter()
                                .map(|w| {
                                    spawn_json(
                                        w.x, w.y, w.kind, w.passive, w.walk_to, w.face, w.group,
                                    )
                                })
                                .collect()),
                        )]),
                        Action::OpenExit(e) => Obj(vec![("open_exit".into(), s(e))]),
                        Action::CloseExit(e) => Obj(vec![("close_exit".into(), s(e))]),
                        Action::Objective(o) => Obj(vec![("objective".into(), s(o))]),
                        Action::Sfx(x) => Obj(vec![("sfx".into(), s(x))]),
                        Action::Alert(t) => Obj(vec![(
                            "alert".into(),
                            match t {
                                AlertTarget::All => s("all"),
                                AlertTarget::Zone(z) => Obj(vec![("zone".into(), s(z))]),
                                AlertTarget::Group(g) => Obj(vec![("group".into(), s(g))]),
                            },
                        )]),
                        Action::Hold(h) => {
                            let mut kv = vec![("seconds".to_string(), n(h.seconds))];
                            if h.until_comms_idle {
                                kv.push(("until_comms_idle".into(), Bool(true)));
                            }
                            if let Some(t) = h.text {
                                kv.push(("text".into(), s(t)));
                            }
                            Obj(vec![("hold".into(), Obj(kv))])
                        }
                        Action::LookAt(l) => Obj(vec![(
                            "look_at".into(),
                            Obj(vec![
                                ("x".into(), n(l.x)),
                                ("y".into(), n(l.y)),
                                ("seconds".into(), n(l.seconds)),
                            ]),
                        )]),
                    })
                    .collect();
                Obj(vec![
                    ("id".into(), s(st.id)),
                    ("trigger".into(), Obj(trigger)),
                    ("actions".into(), Arr(actions)),
                ])
            })
            .collect();
        let mut top = vec![
            ("id".to_string(), Num(self.id as f32)),
            ("name".into(), s(&self.name)),
            ("theme".into(), s(&self.theme)),
            ("accent".into(), s(&self.accent)),
            ("flavor".into(), s(&self.flavor)),
            ("objective".into(), s(&self.objective)),
            (
                "size".into(),
                Obj(vec![
                    ("w".into(), n(self.width)),
                    ("h".into(), n(self.height)),
                ]),
            ),
        ];
        if self.surface != Surface::Checker {
            top.push(("surface".into(), s(surface_name(self.surface))));
        }
        top.extend([
            ("entry".into(), Obj(entry)),
            ("exits".into(), Arr(exits)),
            ("walls".into(), Arr(walls)),
            ("rooms".into(), Arr(rooms)),
            ("zones".into(), Arr(zones)),
            ("spawns".into(), Arr(spawns)),
            ("pickups".into(), Arr(pickups)),
        ]);
        // `props` is optional in the format: a floor without any keeps the
        // key out (as the checked-in files without props do).
        if !props.is_empty() {
            top.push(("props".into(), Arr(props)));
        }
        top.push(("scenario".into(), Arr(scenario)));
        Obj(top)
    }

    /// The `levels/floor_NN.json` text.
    pub fn to_json(&self) -> String {
        write_json(&self.to_json_value())
    }
}

// ---------------------------------------------------------------------------
// Undo / redo
// ---------------------------------------------------------------------------

/// The open floor plus its history. Call [`EditorDoc::begin_edit`] BEFORE
/// mutating `floor` (once per user-level operation: a drag, a delete, a
/// placement) so it can be undone as one step.
#[derive(Clone, Debug)]
pub struct EditorDoc {
    pub floor: EditableFloor,
    undo: Vec<EditableFloor>,
    redo: Vec<EditableFloor>,
    /// The state as loaded / last saved (for the dirty flag).
    baseline: EditableFloor,
}

impl EditorDoc {
    pub fn new(floor: EditableFloor) -> Self {
        EditorDoc {
            baseline: floor.clone(),
            floor,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Snapshot the current state onto the undo stack (and drop the redo
    /// stack): the next mutations form one undoable step.
    pub fn begin_edit(&mut self) {
        self.undo.push(self.floor.clone());
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                let cur = std::mem::replace(&mut self.floor, prev);
                self.redo.push(cur);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                let cur = std::mem::replace(&mut self.floor, next);
                self.undo.push(cur);
                true
            }
            None => false,
        }
    }

    /// Whether the floor differs from the loaded / last-saved state.
    pub fn dirty(&self) -> bool {
        self.floor != self.baseline
    }

    /// Mark the current state as saved.
    pub fn mark_saved(&mut self) {
        self.baseline = self.floor.clone();
    }
}

// ---------------------------------------------------------------------------
// JSON writer
// ---------------------------------------------------------------------------

/// A JSON value (what the writer serializes). Object keys keep their order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f32),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// The web editor's line width for inlining an object / array.
const MAX_COL: usize = 100;

impl Json {
    /// Nesting depth: primitives 0, a container 1 + its deepest child.
    fn depth(&self) -> usize {
        match self {
            Json::Arr(items) => 1 + items.iter().map(Json::depth).max().unwrap_or(0),
            Json::Obj(kv) => 1 + kv.iter().map(|(_, v)| v.depth()).max().unwrap_or(0),
            _ => 0,
        }
    }

    /// Whether any array below (or at) this value holds objects / arrays.
    fn has_array_of_objects(&self) -> bool {
        match self {
            Json::Arr(items) => {
                items
                    .iter()
                    .any(|v| matches!(v, Json::Arr(_) | Json::Obj(_)))
                    || items.iter().any(Json::has_array_of_objects)
            }
            Json::Obj(kv) => kv.iter().any(|(_, v)| v.has_array_of_objects()),
            _ => false,
        }
    }

    fn is_empty_container(&self) -> bool {
        matches!(self, Json::Arr(v) if v.is_empty()) || matches!(self, Json::Obj(v) if v.is_empty())
    }
}

/// A JSON string literal, escaped like `JSON.stringify` (non-ASCII passes
/// through untouched).
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number like `JSON.stringify` prints it: integers without a
/// fraction, others in the shortest form that round-trips (an `f32`).
pub fn json_num(v: f32) -> String {
    if !v.is_finite() {
        return "null".to_string();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The single-line form of a value.
fn inline_text(v: &Json) -> String {
    match v {
        Json::Null => "null".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => json_num(*n),
        Json::Str(s) => json_str(s),
        Json::Arr(items) => {
            if items.is_empty() {
                "[]".to_string()
            } else {
                let parts: Vec<String> = items.iter().map(inline_text).collect();
                format!("[ {} ]", parts.join(", "))
            }
        }
        Json::Obj(kv) => {
            if kv.is_empty() {
                "{}".to_string()
            } else {
                let parts: Vec<String> = kv
                    .iter()
                    .map(|(k, v)| format!("{}: {}", json_str(k), inline_text(v)))
                    .collect();
                format!("{{ {} }}", parts.join(", "))
            }
        }
    }
}

/// Pretty-print like `tools/levels-editor-format.js`'s `fmt`: 2-space
/// indent, one entry per line; a container is written on ONE line when it
/// nests at most two levels, holds no array of objects and its inline text
/// fits in 100 columns (indent included).
fn fmt(v: &Json, indent: usize, out: &mut String) {
    if !matches!(v, Json::Arr(_) | Json::Obj(_)) || v.is_empty_container() {
        out.push_str(&inline_text(v));
        return;
    }
    if v.depth() <= 2 && !v.has_array_of_objects() {
        let t = inline_text(v);
        if indent + t.chars().count() <= MAX_COL {
            out.push_str(&t);
            return;
        }
    }
    let ind = " ".repeat(indent + 2);
    match v {
        Json::Arr(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&ind);
                fmt(item, indent + 2, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&" ".repeat(indent));
            out.push(']');
        }
        Json::Obj(kv) => {
            out.push_str("{\n");
            for (i, (k, item)) in kv.iter().enumerate() {
                out.push_str(&ind);
                out.push_str(&json_str(k));
                out.push_str(": ");
                fmt(item, indent + 2, out);
                out.push_str(if i + 1 < kv.len() { ",\n" } else { "\n" });
            }
            out.push_str(&" ".repeat(indent));
            out.push('}');
        }
        _ => unreachable!(),
    }
}

/// The document text (ends with a single newline).
pub fn write_json(v: &Json) -> String {
    let mut out = String::new();
    fmt(v, 0, &mut out);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::{floor_def, LEVEL_COUNT};
    use crate::levels_data::FLOORS;
    use crate::math::Vec2;

    // ---- a tiny JSON reader (tests only): proves the writer's output is
    // well-formed and lets edits be checked structurally.
    struct Parser<'a> {
        s: &'a [u8],
        i: usize,
    }

    impl<'a> Parser<'a> {
        fn ws(&mut self) {
            while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: u8) {
            self.ws();
            assert_eq!(self.s[self.i], c, "expected {:?} at {}", c as char, self.i);
            self.i += 1;
        }
        fn value(&mut self) -> Json {
            self.ws();
            match self.s[self.i] {
                b'{' => {
                    self.i += 1;
                    let mut kv = Vec::new();
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        return Json::Obj(kv);
                    }
                    loop {
                        self.ws();
                        let k = match self.value() {
                            Json::Str(k) => k,
                            other => panic!("bad key {other:?}"),
                        };
                        self.eat(b':');
                        let v = self.value();
                        kv.push((k, v));
                        self.ws();
                        if self.s[self.i] == b',' {
                            self.i += 1;
                        } else {
                            self.eat(b'}');
                            return Json::Obj(kv);
                        }
                    }
                }
                b'[' => {
                    self.i += 1;
                    let mut items = Vec::new();
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        return Json::Arr(items);
                    }
                    loop {
                        items.push(self.value());
                        self.ws();
                        if self.s[self.i] == b',' {
                            self.i += 1;
                        } else {
                            self.eat(b']');
                            return Json::Arr(items);
                        }
                    }
                }
                b'"' => {
                    self.i += 1;
                    let mut out = String::new();
                    loop {
                        let c = self.s[self.i];
                        self.i += 1;
                        match c {
                            b'"' => break,
                            b'\\' => {
                                let e = self.s[self.i];
                                self.i += 1;
                                out.push(match e {
                                    b'n' => '\n',
                                    b't' => '\t',
                                    b'r' => '\r',
                                    b'b' => '\u{8}',
                                    b'f' => '\u{c}',
                                    b'u' => {
                                        let hex = std::str::from_utf8(&self.s[self.i..self.i + 4])
                                            .unwrap();
                                        self.i += 4;
                                        char::from_u32(u32::from_str_radix(hex, 16).unwrap())
                                            .unwrap()
                                    }
                                    other => other as char,
                                });
                            }
                            _ => {
                                // Re-assemble UTF-8 sequences byte by byte.
                                let start = self.i - 1;
                                let len = match c {
                                    0x00..=0x7f => 1,
                                    0xc0..=0xdf => 2,
                                    0xe0..=0xef => 3,
                                    _ => 4,
                                };
                                self.i = start + len;
                                out.push_str(std::str::from_utf8(&self.s[start..self.i]).unwrap());
                            }
                        }
                    }
                    Json::Str(out)
                }
                b't' => {
                    self.i += 4;
                    Json::Bool(true)
                }
                b'f' => {
                    self.i += 5;
                    Json::Bool(false)
                }
                b'n' => {
                    self.i += 4;
                    Json::Null
                }
                _ => {
                    let start = self.i;
                    while self.i < self.s.len()
                        && (self.s[self.i] == b'-'
                            || self.s[self.i] == b'.'
                            || self.s[self.i] == b'e'
                            || self.s[self.i] == b'+'
                            || self.s[self.i].is_ascii_digit())
                    {
                        self.i += 1;
                    }
                    let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                    Json::Num(
                        t.parse::<f32>()
                            .unwrap_or_else(|_| panic!("bad number {t:?}")),
                    )
                }
            }
        }
    }

    fn parse(text: &str) -> Json {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        let v = p.value();
        p.ws();
        assert_eq!(p.i, text.len(), "trailing garbage");
        v
    }

    fn get<'a>(v: &'a Json, key: &str) -> &'a Json {
        match v {
            Json::Obj(kv) => &kv.iter().find(|(k, _)| k == key).expect(key).1,
            _ => panic!("not an object"),
        }
    }

    fn arr(v: &Json) -> &Vec<Json> {
        match v {
            Json::Arr(a) => a,
            _ => panic!("not an array"),
        }
    }

    #[test]
    fn writer_inlines_small_objects_and_breaks_arrays_of_objects() {
        let v = Json::Obj(vec![
            (
                "size".into(),
                Json::Obj(vec![
                    ("w".into(), Json::Num(1000.0)),
                    ("h".into(), Json::Num(800.0)),
                ]),
            ),
            ("empty".into(), Json::Arr(vec![])),
            (
                "walls".into(),
                Json::Arr(vec![Json::Obj(vec![
                    ("x".into(), Json::Num(0.5)),
                    ("y".into(), Json::Num(-2.0)),
                ])]),
            ),
            ("flag".into(), Json::Bool(false)),
            ("text".into(), Json::Str("a \"quoted\" line\n— ½".into())),
        ]);
        let text = write_json(&v);
        assert_eq!(
            text,
            "{\n  \"size\": { \"w\": 1000, \"h\": 800 },\n  \"empty\": [],\n  \"walls\": [\n    { \"x\": 0.5, \"y\": -2 }\n  ],\n  \"flag\": false,\n  \"text\": \"a \\\"quoted\\\" line\\n— ½\"\n}\n"
        );
        assert_eq!(parse(&text), v);
    }

    #[test]
    fn writer_breaks_lines_past_100_columns() {
        let long = "x".repeat(90);
        let v = Json::Obj(vec![(
            "say".into(),
            Json::Obj(vec![("text".into(), Json::Str(long.clone()))]),
        )]);
        let text = write_json(&v);
        // Neither the outer nor the inner object fits on one line.
        assert!(text.starts_with("{\n  \"say\": {\n    \"text\": \"xxx"));
        assert_eq!(parse(&text), v);
    }

    #[test]
    fn levels_round_trip_byte_for_byte() {
        // Every compiled floor, saved untouched, must be exactly the checked-in
        // JSON the web editor wrote (proves format compatibility, and that
        // `make gen-levels` accepts what the native editor saves).
        let root = env!("CARGO_MANIFEST_DIR");
        let mut mismatches = Vec::new();
        for f in FLOORS.iter() {
            let ef = EditableFloor::from_def(f);
            let path = format!("{root}/levels/{}", ef.file_name());
            let disk = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let ours = ef.to_json();
            if disk != ours {
                let line = disk
                    .lines()
                    .zip(ours.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                mismatches.push(format!(
                    "{}: differs at line {} — disk {:?} vs ours {:?}",
                    ef.file_name(),
                    line + 1,
                    disk.lines().nth(line).unwrap_or(""),
                    ours.lines().nth(line).unwrap_or("")
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn every_floor_validates_clean() {
        let ids: Vec<usize> = FLOORS.iter().map(|f| f.id).collect();
        for f in FLOORS.iter() {
            let ef = EditableFloor::from_def(f);
            assert_eq!(ef.validate(&ids), Vec::<String>::new(), "floor {}", f.id);
        }
    }

    #[test]
    fn edits_show_up_in_the_json_and_undo_reverts_them() {
        let mut doc = EditorDoc::new(EditableFloor::from_def(floor_def(0)));
        let n_walls = doc.floor.walls.len();
        let n_props = doc.floor.props.len();
        assert!(!doc.dirty());

        doc.begin_edit();
        let wall = doc.floor.add_wall(Rect::new(100.0, 550.0, 100.0, 20.0));
        doc.begin_edit();
        let prop = doc.floor.add_prop(PropPlacement {
            kind: prop_kind_index("crac_cooler").unwrap(),
            x: 500.0,
            y: 500.0,
            rot: 90.0,
            size: 60.0,
        });
        doc.begin_edit();
        doc.floor.translate(prop, 10.0, -10.0);
        assert!(doc.dirty());
        assert_eq!(
            doc.floor.rect_of(wall),
            Some(Rect::new(100.0, 550.0, 100.0, 20.0))
        );
        assert_eq!(doc.floor.hit_test(Vec2::new(510.0, 490.0)), Some(prop));
        assert_eq!(doc.floor.hit_test(Vec2::new(150.0, 560.0)), Some(wall));

        let json = parse(&doc.floor.to_json());
        assert_eq!(arr(get(&json, "walls")).len(), n_walls + 1);
        let props = arr(get(&json, "props"));
        assert_eq!(props.len(), n_props + 1);
        let last = props.last().unwrap();
        assert_eq!(get(last, "kind"), &Json::Str("crac_cooler".into()));
        assert_eq!(get(last, "x"), &Json::Num(510.0));
        assert_eq!(get(last, "rot"), &Json::Num(90.0));
        // The steps are carried through verbatim.
        assert_eq!(
            arr(get(&json, "scenario")).len(),
            floor_def(0).scenario.len()
        );

        assert!(doc.undo()); // the move
        assert_eq!(doc.floor.props.last().unwrap().x, 500.0);
        assert!(doc.undo()); // the prop
        assert_eq!(doc.floor.props.len(), n_props);
        assert!(doc.redo());
        assert_eq!(doc.floor.props.len(), n_props + 1);
        assert!(doc.undo());
        assert!(doc.undo()); // the wall
        assert!(!doc.undo());
        assert!(!doc.dirty());
        assert_eq!(doc.floor.walls.len(), n_walls);
    }

    #[test]
    fn undo_depth_is_capped() {
        let mut doc = EditorDoc::new(EditableFloor::from_def(floor_def(1)));
        for i in 0..(UNDO_DEPTH + 20) {
            doc.begin_edit();
            doc.floor.add_spawn(i as f32, 0.0, EnemyType::Idle);
        }
        let mut n = 0;
        while doc.undo() {
            n += 1;
        }
        assert_eq!(n, UNDO_DEPTH);
    }

    #[test]
    fn creation_helpers_pick_unique_ids_and_validation_catches_bad_refs() {
        let ids: Vec<usize> = (0..LEVEL_COUNT).collect();
        // Level index 1 = floor 1, the lobby (index 0 is floor 0, the gate).
        let mut f = EditableFloor::from_def(floor_def(1));
        let n0 = f.zones.len();
        let z = f.add_zone(Rect::new(10.0, 10.0, 50.0, 50.0));
        let z2 = f.add_zone(Rect::new(10.0, 10.0, 50.0, 50.0));
        assert_ne!(f.zones[0].id, "zone1"); // floor 1 has a zone "desk"; new ones count from 1
        assert_eq!(f.zones.len(), n0 + 2);
        assert_ne!(f.rect_of(z), None);
        assert!(f.delete(z2));
        assert!(!f.delete(Item::Entry));
        // Deleting the zone a step enters is caught.
        let desk = f.zones.iter().position(|z| z.id == "desk").unwrap();
        f.zones.remove(desk);
        let problems = f.validate(&ids);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("zone \"desk\" does not exist")),
            "{problems:?}"
        );
        // An exit to a floor that does not exist too.
        let e = f.add_exit(Rect::new(500.0, 20.0, 90.0, 60.0));
        if let Item::Exit(i) = e {
            f.exits[i].to = 99;
        }
        assert!(f
            .validate(&ids)
            .iter()
            .any(|p| p.contains("unknown floor 99")));
        assert_eq!(f.exits.last().unwrap().id, "exit1");
        assert_eq!(floor_file_name(14), "floor_13h.json");
        assert_eq!(floor_file_name(3), "floor_03.json");
        assert_eq!(prop_kind_index("nope"), None);
    }
}
