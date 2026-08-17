//! The NATIVE level editor: the `?viz` LEVELS tab, drawn with the game's own
//! renderer (floor tiles, walls, elevator cars and placed props exactly as
//! in-game; rooms / zones / spawns / pickups as overlays). Spatial editing
//! only — the scenario steps / dialogue side stays in the web editor
//! (`tools/levels.html`, opened over the map by SCENARIO (web)). The
//! document model, undo history, validation and the JSON writer live in
//! `editor.rs`; this file is the immediate-mode UI (wasm-only).
//!
//! Layout (canvas coordinates): the tab bar (drawn by `lib.rs`, y 14..60),
//! two header rows (floor picker + actions, then the tools) down to
//! [`MAP_TOP`], the map pane below it (right of it the properties / palette
//! panel, under it the status line). index.html positions the SCENARIO
//! (web) iframe at `MAP_TOP` so the header stays reachable.

use wasm_bindgen::prelude::*;

use crate::components::{EnemyType, WeaponType};
use crate::editor::{
    enemy_type_id, next_enemy_type, next_weapon, weapon_id, EditableFloor, EditorDoc, Item,
};
use crate::floor_props::draw_placed_prop;
use crate::graphics::Graphics;
use crate::input::{self, keys, mouse_buttons};
use crate::level::Level;
use crate::levels::{floor_def, floor_title, LEVEL_COUNT};
use crate::levels_data::FLOORS;
use crate::math::{Color, Vec2};
use crate::props::{draw_prop, family_range, prop_px, snap_size, PROP_FAMILIES, PROP_NAMES};
use crate::render::draw_wall;
use crate::render_comms::{car_back_side, draw_elevator_car, CarView};
use crate::scenario::{parse_hex_rgb, PropPlacement, Rect};

#[wasm_bindgen]
extern "C" {
    // index.html: open / hide the iframe panel (SCENARIO (web) = the
    // tools/levels.html editor over the map pane).
    #[wasm_bindgen(js_namespace = window, js_name = vizInspect)]
    fn viz_inspect(kind: &str);
    #[wasm_bindgen(js_namespace = window, js_name = vizInspectHide)]
    fn viz_inspect_hide();
    // index.html: PUT levels/<file> through serve.py's editor API (token
    // flow + result toast).
    #[wasm_bindgen(js_namespace = window, js_name = vizSaveLevel)]
    fn viz_save_level(file: &str, json: &str);
}

/// Top of the map pane; everything above is the tab bar + the editor's two
/// header rows. Mirrored by index.html (`#viz-panel.full.levels { top }`).
pub const MAP_TOP: f32 = 150.0;
/// Width of the right-hand properties / palette panel.
const PANEL_W: f32 = 260.0;
/// Height of the status line under the map.
const STATUS_H: f32 = 34.0;
/// The header rows.
const ROW1_Y: f32 = 66.0;
const ROW2_Y: f32 = 106.0;
const ROW_H: f32 = 32.0;
/// Snap step (world units).
const SNAP_STEP: f32 = 10.0;
/// Resize handle half-size (screen px) and grab tolerance.
const HANDLE: f32 = 4.0;
/// Approximate VT323 advance per font-size unit.
const CHAR_W: f32 = 0.42;

const BG: Color = Color::new(20.0 / 255.0, 12.0 / 255.0, 28.0 / 255.0, 1.0);
const PANEL_BG: Color = Color::new(0.09, 0.06, 0.12, 1.0);
const BORDER: Color = Color::new(0.4, 0.3, 0.45, 1.0);
const CORAL: Color = Color::new(217.0 / 255.0, 119.0 / 255.0, 87.0 / 255.0, 1.0);
const CYAN: Color = Color::new(0.2, 0.9, 0.9, 1.0);
const GOLD: Color = Color::new(1.0, 0.85, 0.3, 1.0);
const OK_GREEN: Color = Color::new(0.24, 0.88, 0.55, 1.0);
const BAD_RED: Color = Color::new(1.0, 0.18, 0.35, 1.0);

/// The editing tools (header row 2; digit shortcuts in this order).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Select,
    Wall,
    Room,
    Zone,
    Spawn,
    Pickup,
    Entry,
    Exit,
    Prop,
}

const TOOLS: [(Tool, &str); 9] = [
    (Tool::Select, "1 SELECT"),
    (Tool::Wall, "2 WALL"),
    (Tool::Room, "3 ROOM"),
    (Tool::Zone, "4 ZONE"),
    (Tool::Spawn, "5 SPAWN"),
    (Tool::Pickup, "6 PICKUP"),
    (Tool::Entry, "7 ENTRY"),
    (Tool::Exit, "8 EXIT"),
    (Tool::Prop, "9 PROP"),
];

/// An in-progress mouse gesture.
#[derive(Clone, Copy, Debug)]
enum Drag {
    /// Middle / right / Space+left drag: pan the view.
    Pan { last: Vec2 },
    /// Moving the selection (world position of the grab, its start rect).
    Move { item: Item, grab: Vec2, start: Rect },
    /// Resizing a rect item by handle 0..8 (TL, T, TR, R, BR, B, BL, L).
    Resize {
        item: Item,
        handle: usize,
        start: Rect,
        grab: Vec2,
    },
    /// Rubber-banding a new rectangle (WALL / ROOM / ZONE / ENTRY / EXIT).
    Rubber { start: Vec2 },
}

/// A text field of the properties strip (edits the SELECTED item).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Id,
    Label,
}

/// The editor state (one per `?viz` session; floors are loaded lazily and
/// keep their edits while you switch between them).
pub struct Editor {
    level: usize,
    docs: Vec<Option<EditorDoc>>,
    known_ids: Vec<usize>,
    /// View: screen = pan + world * zoom.
    pan: Vec2,
    zoom: f32,
    fitted: bool,
    tool: Tool,
    sel: Option<Item>,
    drag: Option<Drag>,
    grid: bool,
    snap: bool,
    /// PROP tool: palette page (family), the picked prop, its rotation and
    /// size for the next placement.
    family: usize,
    brush: usize,
    brush_rot: f32,
    brush_size: f32,
    /// SPAWN / PICKUP tools: what the next click places.
    spawn_kind: EnemyType,
    weapon: WeaponType,
    /// The focused text field and its buffer.
    field: Option<Field>,
    buf: String,
    /// The SCENARIO (web) iframe is open over the map.
    web_open: bool,
    tiles: Level,
    /// A transient message on the status line (and when it was set, ms).
    note: String,
    note_at: f64,
}

/// A viz-style button; returns whether the mouse is over it.
#[allow(clippy::too_many_arguments)]
fn button(
    g: &Graphics,
    mouse: Vec2,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    active: bool,
) -> bool {
    let over = mouse.x >= x && mouse.x <= x + w && mouse.y >= y && mouse.y <= y + h;
    let bg = if active {
        Color::new(1.0, 0.09, 0.26, 0.85)
    } else if over {
        Color::new(0.28, 0.22, 0.33, 1.0)
    } else {
        Color::new(0.14, 0.10, 0.18, 1.0)
    };
    g.draw_rectangle(Vec2::new(x, y), w, h, bg);
    g.draw_rectangle_lines(Vec2::new(x, y), w, h, 1.0, Color::new(0.45, 0.35, 0.5, 1.0));
    let fs = if h >= 30.0 { 16.0 } else { 13.0 };
    let tw = label.chars().count() as f32 * fs * CHAR_W;
    g.draw_text(
        label,
        Vec2::new(x + ((w - tw) / 2.0).max(4.0), y + h / 2.0 + fs * 0.35),
        fs,
        Color::WHITE,
    );
    over
}

fn rgb(c: (u8, u8, u8), a: f32) -> Color {
    Color::new(
        c.0 as f32 / 255.0,
        c.1 as f32 / 255.0,
        c.2 as f32 / 255.0,
        a,
    )
}

fn spawn_color(t: EnemyType) -> Color {
    match t {
        EnemyType::Idle => rgb((224, 49, 66), 1.0),
        EnemyType::Wandering => rgb((150, 70, 210), 1.0),
        EnemyType::Patrolling => rgb((224, 40, 160), 1.0),
    }
}

fn weapon_letter(w: WeaponType) -> &'static str {
    match w {
        WeaponType::Pistol => "P",
        WeaponType::Shotgun => "S",
        WeaponType::MachineGun => "M",
        WeaponType::Melee => "C",
    }
}

/// The normalized rectangle spanned by two corners.
fn rect_between(a: Vec2, b: Vec2) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    Rect::new(x0, y0, (a.x - b.x).abs(), (a.y - b.y).abs())
}

/// Nicely printed world number (integers without a fraction).
fn num(v: f32) -> String {
    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

impl Editor {
    pub fn new() -> Self {
        let mut docs = Vec::with_capacity(LEVEL_COUNT);
        docs.resize_with(LEVEL_COUNT, || None);
        Editor {
            level: 0,
            docs,
            known_ids: FLOORS.iter().map(|f| f.id).collect(),
            pan: Vec2::zero(),
            zoom: 1.0,
            fitted: false,
            tool: Tool::Select,
            sel: None,
            drag: None,
            grid: true,
            snap: true,
            family: 0,
            brush: 0,
            brush_rot: 0.0,
            brush_size: 100.0,
            spawn_kind: EnemyType::Idle,
            weapon: WeaponType::Pistol,
            field: None,
            buf: String::new(),
            web_open: false,
            tiles: Level::new(),
            note: String::new(),
            note_at: 0.0,
        }
    }

    /// The tab was left: the iframe was hidden by the caller.
    pub fn hide_web(&mut self) {
        self.web_open = false;
    }

    fn doc(&mut self) -> &mut EditorDoc {
        let level = self.level;
        self.docs[level]
            .get_or_insert_with(|| EditorDoc::new(EditableFloor::from_def(floor_def(level))))
    }

    fn floor(&mut self) -> &EditableFloor {
        &self.doc().floor
    }

    fn to_screen(&self, p: Vec2) -> Vec2 {
        Vec2::new(self.pan.x + p.x * self.zoom, self.pan.y + p.y * self.zoom)
    }

    fn to_world(&self, s: Vec2) -> Vec2 {
        Vec2::new(
            (s.x - self.pan.x) / self.zoom,
            (s.y - self.pan.y) / self.zoom,
        )
    }

    fn snap_v(&self, v: f32) -> f32 {
        if self.snap {
            (v / SNAP_STEP).round() * SNAP_STEP
        } else {
            v.round()
        }
    }

    fn snap_p(&self, p: Vec2) -> Vec2 {
        Vec2::new(self.snap_v(p.x), self.snap_v(p.y))
    }

    fn set_level(&mut self, level: usize) {
        self.level = level.min(LEVEL_COUNT - 1);
        self.sel = None;
        self.drag = None;
        self.field = None;
        self.fitted = false;
    }

    /// Fit the whole floor into the map viewport.
    fn fit(&mut self, vp: Rect) {
        let (fw, fh) = {
            let f = self.floor();
            (f.width.max(1.0), f.height.max(1.0))
        };
        self.zoom = ((vp.w - 60.0) / fw).min((vp.h - 60.0) / fh).max(0.05);
        let c = vp.center();
        self.pan = Vec2::new(c.x - fw / 2.0 * self.zoom, c.y - fh / 2.0 * self.zoom);
        self.fitted = true;
    }

    fn zoom_at(&mut self, s: Vec2, factor: f32) {
        let z = (self.zoom * factor).clamp(0.1, 8.0);
        let f = z / self.zoom;
        self.pan = Vec2::new(s.x - (s.x - self.pan.x) * f, s.y - (s.y - self.pan.y) * f);
        self.zoom = z;
    }

    fn note(&mut self, text: &str, now: f64) {
        self.note = text.to_string();
        self.note_at = now;
    }

    /// The 8 resize handles of a rect, in screen space.
    fn handles(&self, r: Rect) -> [Vec2; 8] {
        let a = self.to_screen(Vec2::new(r.x, r.y));
        let b = self.to_screen(Vec2::new(r.x + r.w, r.y + r.h));
        let (mx, my) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        [
            Vec2::new(a.x, a.y),
            Vec2::new(mx, a.y),
            Vec2::new(b.x, a.y),
            Vec2::new(b.x, my),
            Vec2::new(b.x, b.y),
            Vec2::new(mx, b.y),
            Vec2::new(a.x, b.y),
            Vec2::new(a.x, my),
        ]
    }

    /// The handle under the mouse for the selected rect item, if any.
    fn handle_at(&mut self, mouse: Vec2) -> Option<(Item, usize, Rect)> {
        let item = self.sel.filter(|s| s.is_rect())?;
        let r = self.floor().rect_of(item)?;
        let hs = self.handles(r);
        let i = hs.iter().position(|h| {
            (h.x - mouse.x).abs() <= HANDLE + 3.0 && (h.y - mouse.y).abs() <= HANDLE + 3.0
        })?;
        Some((item, i, r))
    }

    /// Apply a resize handle drag: `start` moved by the snapped delta.
    fn resized(start: Rect, handle: usize, dx: f32, dy: f32) -> Rect {
        let (mut x0, mut y0, mut x1, mut y1) =
            (start.x, start.y, start.x + start.w, start.y + start.h);
        let (l, t, r, b) = match handle {
            0 => (true, true, false, false),
            1 => (false, true, false, false),
            2 => (false, true, true, false),
            3 => (false, false, true, false),
            4 => (false, false, true, true),
            5 => (false, false, false, true),
            6 => (true, false, false, true),
            _ => (true, false, false, false),
        };
        if l {
            x0 = (x0 + dx).min(x1 - 1.0);
        }
        if r {
            x1 = (x1 + dx).max(x0 + 1.0);
        }
        if t {
            y0 = (y0 + dy).min(y1 - 1.0);
        }
        if b {
            y1 = (y1 + dy).max(y0 + 1.0);
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    // ------------------------------------------------------------------
    // Frame
    // ------------------------------------------------------------------

    /// One frame: input, then the map, then the panels (drawn last so they
    /// cover whatever the map paints under them). `now` in ms.
    pub fn update(&mut self, g: &Graphics, mouse: Vec2, click: bool, now: f64) {
        let (w, h) = (g.width(), g.height());
        let vp = Rect::new(
            0.0,
            MAP_TOP,
            (w - PANEL_W).max(100.0),
            (h - MAP_TOP - STATUS_H).max(100.0),
        );
        if !self.fitted {
            self.fit(vp);
        }
        let time = (now / 1000.0) as f32;
        let over_map = vp.contains(mouse);
        let typing = self.field.is_some();

        // ---- keyboard ---------------------------------------------------
        if typing {
            self.update_field(now);
        } else {
            self.shortcuts(vp, now);
        }

        // ---- wheel zoom -------------------------------------------------
        let wheel = input::wheel_delta();
        if wheel != 0.0 && over_map {
            self.zoom_at(mouse, (-wheel / 400.0).exp().clamp(0.5, 2.0));
        }

        // ---- mouse on the map ------------------------------------------
        self.map_input(mouse, click, over_map, now);

        // ---- draw ------------------------------------------------------
        self.draw_world(g, vp, time);
        self.draw_overlays(g, vp, mouse, over_map);
        self.draw_header(g, mouse, click, vp, now);
        self.draw_panel(g, mouse, click, w, h, time, now);
        self.draw_status(g, vp, mouse, over_map, now);
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    fn shortcuts(&mut self, vp: Rect, now: f64) {
        let ctrl = input::is_key_down("Control") || input::is_key_down("Meta");
        let shift = input::is_key_down(keys::SHIFT);
        if ctrl {
            if input::is_key_pressed("z") {
                if shift {
                    self.redo(now);
                } else {
                    self.undo(now);
                }
            }
            if input::is_key_pressed("y") {
                self.redo(now);
            }
            return;
        }
        for (i, (tool, _)) in TOOLS.iter().enumerate() {
            if input::is_key_pressed(&(i + 1).to_string()) {
                self.tool = *tool;
            }
        }
        if input::is_key_pressed("Escape") {
            match self.drag {
                Some(Drag::Move { .. }) | Some(Drag::Resize { .. }) => {
                    self.doc().undo();
                    self.drag = None;
                }
                Some(_) => self.drag = None,
                None => {
                    if self.sel.is_some() {
                        self.sel = None;
                    } else {
                        self.tool = Tool::Select;
                    }
                }
            }
        }
        if input::is_key_pressed("f") {
            self.fit(vp);
        }
        if input::is_key_pressed("g") {
            self.grid = !self.grid;
        }
        if input::is_key_pressed("n") {
            self.snap = !self.snap;
        }
        if input::is_key_pressed("t") {
            self.cycle_spawn_kind();
        }
        if input::is_key_pressed("q") {
            self.cycle_weapon();
        }
        if input::is_key_pressed("Delete") || input::is_key_pressed("Backspace") {
            if let Some(item) = self.sel {
                let doc = self.doc();
                doc.begin_edit();
                if doc.floor.delete(item) {
                    self.sel = None;
                } else {
                    doc.undo();
                }
            }
        }
        // Nudge the selection with the arrows (Shift = 1 unit).
        let step = if shift {
            1.0
        } else if self.snap {
            SNAP_STEP
        } else {
            1.0
        };
        let mut nudge = Vec2::zero();
        if input::is_key_pressed(keys::ARROW_LEFT) {
            nudge.x -= step;
        }
        if input::is_key_pressed(keys::ARROW_RIGHT) {
            nudge.x += step;
        }
        if input::is_key_pressed(keys::ARROW_UP) {
            nudge.y -= step;
        }
        if input::is_key_pressed(keys::ARROW_DOWN) {
            nudge.y += step;
        }
        if nudge.x != 0.0 || nudge.y != 0.0 {
            if let Some(item) = self.sel {
                let doc = self.doc();
                doc.begin_edit();
                doc.floor.translate(item, nudge.x, nudge.y);
            }
        }
        // R = rotate the selected prop (else the brush) 90° (Shift: -90°);
        // [ ] = shrink / grow it by 10.
        if input::is_key_pressed("r") {
            self.rotate_prop(if shift { -90.0 } else { 90.0 });
        }
        if input::is_key_pressed("[") {
            self.resize_prop(-10.0);
        }
        if input::is_key_pressed("]") {
            self.resize_prop(10.0);
        }
    }

    fn cycle_spawn_kind(&mut self) {
        if let Some(Item::Spawn(i)) = self.sel {
            let doc = self.doc();
            doc.begin_edit();
            if let Some(s) = doc.floor.spawns.get_mut(i) {
                s.kind = next_enemy_type(s.kind);
                self.spawn_kind = s.kind;
            }
        } else {
            self.spawn_kind = next_enemy_type(self.spawn_kind);
        }
    }

    fn cycle_weapon(&mut self) {
        if let Some(Item::Pickup(i)) = self.sel {
            let doc = self.doc();
            doc.begin_edit();
            if let Some(p) = doc.floor.pickups.get_mut(i) {
                p.weapon = next_weapon(p.weapon);
                self.weapon = p.weapon;
            }
        } else {
            self.weapon = next_weapon(self.weapon);
        }
    }

    fn rotate_prop(&mut self, by: f32) {
        if let Some(Item::Prop(i)) = self.sel {
            let doc = self.doc();
            doc.begin_edit();
            if let Some(p) = doc.floor.props.get_mut(i) {
                p.rot = (p.rot + by).rem_euclid(360.0);
                self.brush_rot = p.rot;
            }
        } else {
            self.brush_rot = (self.brush_rot + by).rem_euclid(360.0);
        }
    }

    fn resize_prop(&mut self, by: f32) {
        if let Some(Item::Prop(i)) = self.sel {
            let doc = self.doc();
            doc.begin_edit();
            if let Some(p) = doc.floor.props.get_mut(i) {
                p.size = (p.size + by).max(10.0);
                self.brush_size = p.size;
            }
        } else {
            self.brush_size = (self.brush_size + by).max(10.0);
        }
    }

    fn undo(&mut self, now: f64) {
        self.drag = None;
        if self.doc().undo() {
            self.sel = None;
            self.note("undo", now);
        }
    }

    fn redo(&mut self, now: f64) {
        self.drag = None;
        if self.doc().redo() {
            self.sel = None;
            self.note("redo", now);
        }
    }

    /// The focused text field: typed characters, Backspace, Enter (commit)
    /// and Escape (cancel).
    fn update_field(&mut self, now: f64) {
        let typed = input::typed_text();
        for c in typed.chars() {
            if !c.is_control() {
                self.buf.push(c);
            }
        }
        if input::is_key_pressed("Backspace") {
            self.buf.pop();
        }
        if input::is_key_pressed("Escape") {
            self.field = None;
        }
        if input::is_key_pressed("Enter") {
            let (field, value) = (self.field.take().unwrap(), self.buf.trim().to_string());
            if let Some(item) = self.sel {
                let doc = self.doc();
                doc.begin_edit();
                let f = &mut doc.floor;
                match (item, field) {
                    (Item::Entry, Field::Label) => f.entry.label = value,
                    (Item::Entry, Field::Id) => f.entry.id = value,
                    (Item::Exit(i), Field::Id) => {
                        if let Some(e) = f.exits.get_mut(i) {
                            e.id = value;
                        }
                    }
                    (Item::Exit(i), Field::Label) => {
                        if let Some(e) = f.exits.get_mut(i) {
                            e.label = value;
                        }
                    }
                    (Item::Room(i), Field::Id) => {
                        if let Some(r) = f.rooms.get_mut(i) {
                            r.id = value;
                        }
                    }
                    (Item::Room(i), Field::Label) => {
                        if let Some(r) = f.rooms.get_mut(i) {
                            r.label = value;
                        }
                    }
                    (Item::Zone(i), Field::Id) => {
                        if let Some(z) = f.zones.get_mut(i) {
                            z.id = value;
                        }
                    }
                    _ => {
                        doc.undo();
                    }
                }
                self.note("field set", now);
            }
        }
    }

    fn map_input(&mut self, mouse: Vec2, click: bool, over_map: bool, now: f64) {
        let down = input::is_mouse_button_down(mouse_buttons::LEFT);
        let pan_btn = input::is_mouse_button_down(mouse_buttons::MIDDLE)
            || input::is_mouse_button_down(mouse_buttons::RIGHT);
        let space = input::is_key_down(keys::SPACE);
        let world = self.to_world(mouse);

        if let Some(drag) = self.drag {
            match drag {
                Drag::Pan { last } => {
                    if pan_btn || (down && space) {
                        self.pan =
                            Vec2::new(self.pan.x + mouse.x - last.x, self.pan.y + mouse.y - last.y);
                        self.drag = Some(Drag::Pan { last: mouse });
                    } else {
                        self.drag = None;
                    }
                }
                Drag::Move { item, grab, start } => {
                    let dx = self.snap_v(world.x - grab.x);
                    let dy = self.snap_v(world.y - grab.y);
                    let r = Rect::new(start.x + dx, start.y + dy, start.w, start.h);
                    self.doc().floor.set_rect(item, r);
                    if !down {
                        self.drag = None;
                    }
                }
                Drag::Resize {
                    item,
                    handle,
                    start,
                    grab,
                } => {
                    let dx = self.snap_v(world.x - grab.x);
                    let dy = self.snap_v(world.y - grab.y);
                    let r = Self::resized(start, handle, dx, dy);
                    self.doc().floor.set_rect(item, r);
                    if !down {
                        self.drag = None;
                    }
                }
                Drag::Rubber { start } => {
                    if !down {
                        self.drag = None;
                        let end = self.snap_p(world);
                        let r = rect_between(start, end);
                        if r.w >= 5.0 && r.h >= 5.0 {
                            let tool = self.tool;
                            let doc = self.doc();
                            doc.begin_edit();
                            let f = &mut doc.floor;
                            let item = match tool {
                                Tool::Wall => Some(f.add_wall(r)),
                                Tool::Room => Some(f.add_room(r)),
                                Tool::Zone => Some(f.add_zone(r)),
                                Tool::Exit => Some(f.add_exit(r)),
                                Tool::Entry => {
                                    f.entry.rect = r;
                                    Some(Item::Entry)
                                }
                                _ => None,
                            };
                            if item.is_none() {
                                doc.undo();
                            }
                            self.sel = item;
                            self.note(&format!("added {}", tool_name(tool)), now);
                        }
                    }
                }
            }
            return;
        }
        if !over_map {
            return;
        }
        let pan_start = input::is_mouse_button_pressed(mouse_buttons::MIDDLE)
            || input::is_mouse_button_pressed(mouse_buttons::RIGHT)
            || (click && space);
        if pan_start {
            self.drag = Some(Drag::Pan { last: mouse });
            return;
        }
        if !click {
            return;
        }
        // A click on the map takes the keyboard focus back from a text field.
        self.field = None;
        match self.tool {
            Tool::Select => {
                if let Some((item, handle, start)) = self.handle_at(mouse) {
                    self.doc().begin_edit();
                    self.drag = Some(Drag::Resize {
                        item,
                        handle,
                        start,
                        grab: world,
                    });
                    return;
                }
                let hit = self.floor().hit_test(world);
                self.sel = hit;
                if let Some(item) = hit {
                    let start = self
                        .floor()
                        .rect_of(item)
                        .unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
                    let doc = self.doc();
                    doc.begin_edit();
                    self.drag = Some(Drag::Move {
                        item,
                        grab: world,
                        start,
                    });
                }
            }
            Tool::Wall | Tool::Room | Tool::Zone | Tool::Entry | Tool::Exit => {
                self.drag = Some(Drag::Rubber {
                    start: self.snap_p(world),
                });
            }
            Tool::Spawn => {
                let p = self.snap_p(world);
                let kind = self.spawn_kind;
                let doc = self.doc();
                doc.begin_edit();
                self.sel = Some(doc.floor.add_spawn(p.x, p.y, kind));
            }
            Tool::Pickup => {
                let p = self.snap_p(world);
                let weapon = self.weapon;
                let doc = self.doc();
                doc.begin_edit();
                self.sel = Some(doc.floor.add_pickup(p.x, p.y, weapon));
            }
            Tool::Prop => {
                let p = self.snap_p(world);
                let placement = PropPlacement {
                    kind: self.brush,
                    x: p.x,
                    y: p.y,
                    rot: self.brush_rot,
                    size: self.brush_size,
                };
                let doc = self.doc();
                doc.begin_edit();
                self.sel = Some(doc.floor.add_prop(placement));
            }
        }
    }

    // ------------------------------------------------------------------
    // Drawing: the map
    // ------------------------------------------------------------------

    /// The floor in world space through the real renderer: tiles, grid,
    /// rooms (fills), walls, elevator cars, placed props.
    fn draw_world(&mut self, g: &Graphics, vp: Rect, time: f32) {
        let (pan, zoom) = (self.pan, self.zoom);
        let (fw, fh, accent, surface) = {
            let f = self.floor();
            (
                f.width,
                f.height,
                parse_hex_rgb(&f.accent).unwrap_or((217, 119, 87)),
                f.surface,
            )
        };
        // The floor's ground (asphalt / marble / …) as the game draws it.
        self.tiles.set_surface(surface);
        g.save();
        g.translate(pan.x, pan.y);
        g.scale(zoom, zoom);

        // Floor tiles, clipped to the floor rect (the tile renderer covers a
        // 2000x2000 area; only the floor's part is a floor).
        let vmin = self.to_world(Vec2::new(vp.x, vp.y));
        let vmax = self.to_world(Vec2::new(vp.x + vp.w, vp.y + vp.h));
        let view_min = Vec2::new(vmin.x.max(0.0), vmin.y.max(0.0));
        let view_max = Vec2::new(vmax.x.min(fw), vmax.y.min(fh));
        if view_max.x > view_min.x && view_max.y > view_min.y {
            self.tiles.render(g, view_min, view_max, None);
        }
        // Grid: the snap step when it is readable, else every 50 units.
        if self.grid {
            let step = if SNAP_STEP * zoom >= 9.0 {
                SNAP_STEP
            } else {
                50.0
            };
            let col = Color::new(1.0, 1.0, 1.0, if step >= 50.0 { 0.07 } else { 0.05 });
            let th = 1.0 / zoom;
            let mut x = (view_min.x / step).ceil() * step;
            while x <= view_max.x {
                g.draw_line(Vec2::new(x, view_min.y), Vec2::new(x, view_max.y), th, col);
                x += step;
            }
            let mut y = (view_min.y / step).ceil() * step;
            while y <= view_max.y {
                g.draw_line(Vec2::new(view_min.x, y), Vec2::new(view_max.x, y), th, col);
                y += step;
            }
        }
        let f = self.docs[self.level].as_ref().unwrap().floor.clone();
        // Rooms: a faint accent wash (labels are drawn in screen space).
        let acc = rgb(accent, 1.0);
        for r in &f.rooms {
            g.draw_rectangle(
                Vec2::new(r.rect.x, r.rect.y),
                r.rect.w,
                r.rect.h,
                Color::new(acc.r, acc.g, acc.b, 0.05),
            );
        }
        for wl in &f.walls {
            draw_wall(g, wl.x, wl.y, wl.w, wl.h);
        }
        // Elevator cars exactly as in-game (the shaft side is the wall behind).
        let in_wall = |p: Vec2| f.walls.iter().any(|w| w.contains(p));
        let cars = std::iter::once((&f.entry, false)).chain(f.exits.iter().map(|e| (e, true)));
        for (car, is_exit) in cars {
            let r = car.rect;
            let side = car_back_side(r.x, r.y, r.w, r.h, in_wall);
            let view = CarView {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                label: &car.label,
                is_exit,
                open: is_exit && car.open,
                dwell: 0.0,
                kind: car.kind,
            };
            draw_elevator_car(g, &view, side, accent, time);
        }
        for p in &f.props {
            draw_placed_prop(g, p, time);
        }
        // Floor outline.
        g.draw_rectangle_lines(
            Vec2::zero(),
            fw,
            fh,
            2.0 / zoom,
            Color::new(acc.r, acc.g, acc.b, 0.6),
        );
        g.restore();
    }

    /// Screen-space overlays: zone outlines + labels, room labels, spawn
    /// diamonds, pickups, the player start, the selection (with handles),
    /// the hover outline and the rubber band.
    fn draw_overlays(&mut self, g: &Graphics, vp: Rect, mouse: Vec2, over_map: bool) {
        let f = self.docs[self.level].as_ref().unwrap().floor.clone();
        let accent = parse_hex_rgb(&f.accent).unwrap_or((217, 119, 87));
        let acc = rgb(accent, 1.0);
        let sr = |r: Rect| -> (Vec2, f32, f32) {
            let a = self.to_screen(Vec2::new(r.x, r.y));
            (a, r.w * self.zoom, r.h * self.zoom)
        };
        for r in &f.rooms {
            let (p, w, h) = sr(r.rect);
            g.draw_rectangle_lines(p, w, h, 1.0, Color::new(acc.r, acc.g, acc.b, 0.35));
            g.draw_text(
                &r.label,
                Vec2::new(p.x + 4.0, p.y + 13.0),
                12.0,
                Color::new(0.85, 0.82, 1.0, 0.7),
            );
        }
        for z in &f.zones {
            let (p, w, h) = sr(z.rect);
            g.draw_rectangle_lines(p, w, h, 1.0, Color::new(0.2, 0.9, 0.9, 0.55));
            let tw = z.id.chars().count() as f32 * 12.0 * CHAR_W;
            g.draw_text(
                &z.id,
                Vec2::new(p.x + w - tw - 4.0, p.y + h - 4.0),
                12.0,
                Color::new(0.2, 0.9, 0.9, 0.85),
            );
        }
        for s in &f.spawns {
            let c = self.to_screen(Vec2::new(s.x, s.y));
            let col = spawn_color(s.kind);
            let r = 7.0;
            // A diamond: two triangles via lines is fiddly; a rotated square.
            g.save();
            g.translate(c.x, c.y);
            g.rotate(std::f32::consts::FRAC_PI_4);
            g.draw_rectangle(Vec2::new(-r * 0.7, -r * 0.7), r * 1.4, r * 1.4, col);
            g.draw_rectangle_lines(
                Vec2::new(-r * 0.7, -r * 0.7),
                r * 1.4,
                r * 1.4,
                1.0,
                Color::WHITE,
            );
            g.restore();
        }
        for p in &f.pickups {
            let c = self.to_screen(Vec2::new(p.x, p.y));
            g.draw_rectangle(Vec2::new(c.x - 7.0, c.y - 6.0), 14.0, 12.0, GOLD);
            g.draw_text(
                weapon_letter(p.weapon),
                Vec2::new(c.x - 3.0, c.y + 4.0),
                11.0,
                Color::new(0.2, 0.12, 0.05, 1.0),
            );
        }
        // Player start = the entry centre.
        let ps = self.to_screen(f.entry.rect.center());
        g.draw_circle(ps, 6.0, CORAL);
        g.draw_circle(ps, 2.5, Color::WHITE);

        // Hover (SELECT tool) and selection.
        if over_map && self.tool == Tool::Select && self.drag.is_none() {
            let world = self.to_world(mouse);
            if let Some(hit) = f.hit_test(world) {
                if Some(hit) != self.sel {
                    if let Some(r) = f.rect_of(hit) {
                        let (p, w, h) = sr(r);
                        g.draw_rectangle_lines(p, w, h, 1.0, Color::new(1.0, 1.0, 1.0, 0.45));
                    }
                }
            }
        }
        if let Some(sel) = self.sel {
            if let Some(r) = f.rect_of(sel) {
                let (p, w, h) = sr(r);
                g.draw_rectangle_lines(p, w, h, 2.0, Color::WHITE);
                if sel.is_rect() {
                    for hnd in self.handles(r) {
                        g.draw_rectangle(
                            Vec2::new(hnd.x - HANDLE, hnd.y - HANDLE),
                            2.0 * HANDLE,
                            2.0 * HANDLE,
                            Color::WHITE,
                        );
                    }
                }
                if let Item::Prop(i) = sel {
                    // The prop's facing (its local +y after rotation).
                    if let Some(pp) = f.props.get(i) {
                        let c = self.to_screen(Vec2::new(pp.x, pp.y));
                        let a = pp.rot.to_radians();
                        let len = pp.size * self.zoom * 0.5;
                        g.draw_line(
                            c,
                            Vec2::new(c.x - a.sin() * len, c.y + a.cos() * len),
                            1.5,
                            GOLD,
                        );
                    }
                }
            }
        }
        // Rubber band / prop ghost.
        if let Some(Drag::Rubber { start }) = self.drag {
            let end = self.snap_p(self.to_world(mouse));
            let (p, w, h) = sr(rect_between(start, end));
            g.draw_rectangle(p, w, h, Color::new(1.0, 1.0, 1.0, 0.08));
            g.draw_rectangle_lines(p, w, h, 1.5, Color::WHITE);
            g.draw_text(
                &format!("{} x {}", num(w / self.zoom), num(h / self.zoom)),
                Vec2::new(p.x + 4.0, p.y - 4.0),
                12.0,
                Color::WHITE,
            );
        }
        if over_map && self.drag.is_none() {
            let wp = self.snap_p(self.to_world(mouse));
            let c = self.to_screen(wp);
            match self.tool {
                Tool::Prop => {
                    let s = self.brush_size * self.zoom;
                    g.save();
                    g.translate(c.x, c.y);
                    g.rotate(self.brush_rot.to_radians());
                    g.draw_rectangle_lines(
                        Vec2::new(-s / 2.0, -s / 2.0),
                        s,
                        s,
                        1.0,
                        Color::new(1.0, 1.0, 1.0, 0.5),
                    );
                    g.restore();
                }
                Tool::Spawn => {
                    g.draw_circle(c, 6.0, Color::new(1.0, 1.0, 1.0, 0.35));
                }
                Tool::Pickup => {
                    g.draw_rectangle_lines(Vec2::new(c.x - 7.0, c.y - 6.0), 14.0, 12.0, 1.0, GOLD);
                }
                _ => {}
            }
        }
        // Keep the map's overflow away from the tab bar row: an opaque band
        // between the top of the canvas and the map (the header rows and the
        // tab bar draw over it).
        g.draw_rectangle(Vec2::zero(), g.width(), MAP_TOP, BG);
        let _ = vp;
    }

    // ------------------------------------------------------------------
    // Drawing: header rows, panel, status
    // ------------------------------------------------------------------

    fn draw_header(&mut self, g: &Graphics, mouse: Vec2, click: bool, vp: Rect, now: f64) {
        // Row 1: floor picker + actions.
        let y = ROW1_Y;
        if button(g, mouse, 20.0, y, 34.0, ROW_H, "<", false) && click {
            let l = if self.level == 0 {
                LEVEL_COUNT - 1
            } else {
                self.level - 1
            };
            self.set_level(l);
        }
        let dirty = self.doc().dirty();
        let title = format!(
            "{}  {}{}",
            floor_title(self.level),
            self.floor().name,
            if dirty { "  *" } else { "" }
        );
        g.draw_rectangle(
            Vec2::new(58.0, y),
            300.0,
            ROW_H,
            Color::new(0.07, 0.05, 0.10, 1.0),
        );
        g.draw_rectangle_lines(Vec2::new(58.0, y), 300.0, ROW_H, 1.0, BORDER);
        g.draw_text(&title, Vec2::new(66.0, y + 22.0), 18.0, Color::WHITE);
        if button(g, mouse, 362.0, y, 34.0, ROW_H, ">", false) && click {
            self.set_level((self.level + 1) % LEVEL_COUNT);
        }
        let mut x = 412.0;
        let mut act = |g: &Graphics, label: &str, active: bool, w: f32| -> bool {
            let hit = button(g, mouse, x, y, w, ROW_H, label, active) && click;
            x += w + 6.0;
            hit
        };
        if act(g, "FIT", false, 48.0) {
            self.fit(vp);
        }
        if act(g, "GRID", self.grid, 56.0) {
            self.grid = !self.grid;
        }
        if act(g, "SNAP", self.snap, 60.0) {
            self.snap = !self.snap;
        }
        let (cu, cr) = {
            let d = self.doc();
            (d.can_undo(), d.can_redo())
        };
        if act(g, "UNDO", false, 58.0) && cu {
            self.undo(now);
        }
        if act(g, "REDO", false, 58.0) && cr {
            self.redo(now);
        }
        if act(g, "SAVE", dirty, 66.0) {
            self.save(now);
        }
        let web_label = if self.web_open {
            "CLOSE SCENARIO"
        } else {
            "SCENARIO (web)"
        };
        if act(g, web_label, self.web_open, 150.0) {
            self.web_open = !self.web_open;
            if self.web_open {
                viz_inspect("levels");
            } else {
                viz_inspect_hide();
            }
        }

        // Row 2: tools + the active tool's option.
        let y = ROW2_Y;
        let mut x = 20.0;
        for (tool, label) in TOOLS.iter() {
            let w = 76.0;
            if button(g, mouse, x, y, w, ROW_H, label, self.tool == *tool) && click {
                self.tool = *tool;
                self.field = None;
            }
            x += w + 4.0;
        }
        x += 12.0;
        match self.tool {
            Tool::Spawn => {
                let label = format!("TYPE: {}  (T)", enemy_type_id(self.spawn_kind));
                let col = spawn_color(self.spawn_kind);
                g.draw_rectangle(Vec2::new(x, y + 8.0), 16.0, 16.0, col);
                if button(g, mouse, x + 22.0, y, 190.0, ROW_H, &label, false) && click {
                    self.spawn_kind = next_enemy_type(self.spawn_kind);
                }
            }
            Tool::Pickup => {
                let label = format!("WEAPON: {}  (Q)", weapon_id(self.weapon));
                if button(g, mouse, x, y, 200.0, ROW_H, &label, false) && click {
                    self.weapon = next_weapon(self.weapon);
                }
            }
            Tool::Prop => {
                // (The palette on the right picks the prop; a click on the
                // map places it.)
                g.draw_text(
                    &format!(
                        "{}  rot {}  size {}",
                        PROP_NAMES[self.brush % PROP_NAMES.len()],
                        num(self.brush_rot),
                        num(self.brush_size)
                    ),
                    Vec2::new(x, y + 21.0),
                    15.0,
                    Color::GRAY,
                );
            }
            Tool::Select => {
                g.draw_text(
                    "click = select · drag = move · handles = resize",
                    Vec2::new(x, y + 21.0),
                    14.0,
                    Color::GRAY,
                );
            }
            _ => {
                g.draw_text(
                    "drag out a rectangle on the map (snapped)",
                    Vec2::new(x, y + 21.0),
                    15.0,
                    Color::GRAY,
                );
            }
        }
    }

    fn save(&mut self, now: f64) {
        let (file, json, problems) = {
            let ids = self.known_ids.clone();
            let f = self.floor();
            (f.file_name(), f.to_json(), f.validate(&ids))
        };
        if !problems.is_empty() {
            self.note(&format!("NOT SAVED — fix first: {}", problems[0]), now);
            return;
        }
        viz_save_level(&file, &json);
        self.doc().mark_saved();
        self.note(&format!("saved {file} — now run: make gen-levels"), now);
    }

    /// The right-hand panel: the selection's properties, then the prop
    /// palette (PROP tool) or the keyboard map.
    #[allow(clippy::too_many_arguments)]
    fn draw_panel(
        &mut self,
        g: &Graphics,
        mouse: Vec2,
        click: bool,
        w: f32,
        h: f32,
        time: f32,
        now: f64,
    ) {
        let x0 = w - PANEL_W;
        let top = ROW1_Y - 4.0;
        g.draw_rectangle(Vec2::new(x0, top), PANEL_W, h - top, PANEL_BG);
        g.draw_line(Vec2::new(x0, top), Vec2::new(x0, h), 1.0, BORDER);
        let px = x0 + 12.0;
        let mut y = top + 20.0;

        // ---- properties of the selection ---------------------------------
        g.draw_text("SELECTION", Vec2::new(px, y), 14.0, Color::GRAY);
        y += 20.0;
        let sel = self.sel;
        let desc = match sel {
            Some(item) => self.floor().describe(item),
            None => "nothing selected".to_string(),
        };
        g.draw_text(&desc, Vec2::new(px, y), 15.0, Color::WHITE);
        y += 20.0;
        if let Some(item) = sel {
            let f = self.floor().clone();
            if let Some(r) = f.rect_of(item) {
                let geo = if item.is_rect() {
                    format!(
                        "x {}  y {}  w {}  h {}",
                        num(r.x),
                        num(r.y),
                        num(r.w),
                        num(r.h)
                    )
                } else {
                    let c = r.center();
                    format!("x {}  y {}", num(c.x), num(c.y))
                };
                g.draw_text(&geo, Vec2::new(px, y), 14.0, Color::GRAY);
                y += 22.0;
            }
            // Text fields / toggles per kind.
            let (id_val, label_val): (Option<String>, Option<String>) = match item {
                Item::Entry => (None, Some(f.entry.label.clone())),
                Item::Exit(i) => f
                    .exits
                    .get(i)
                    .map(|e| (Some(e.id.clone()), Some(e.label.clone())))
                    .unwrap_or((None, None)),
                Item::Room(i) => f
                    .rooms
                    .get(i)
                    .map(|r| (Some(r.id.clone()), Some(r.label.clone())))
                    .unwrap_or((None, None)),
                Item::Zone(i) => f
                    .zones
                    .get(i)
                    .map(|z| (Some(z.id.clone()), None))
                    .unwrap_or((None, None)),
                _ => (None, None),
            };
            if let Some(v) = id_val {
                self.text_field(g, mouse, click, px, y, "id", Field::Id, &v);
                y += 26.0;
            }
            if let Some(v) = label_val {
                self.text_field(g, mouse, click, px, y, "label", Field::Label, &v);
                y += 26.0;
            }
            match item {
                Item::Exit(i) => {
                    let (to, open) = f.exits.get(i).map(|e| (e.to, e.open)).unwrap_or((0, false));
                    g.draw_text("to", Vec2::new(px, y + 15.0), 13.0, Color::GRAY);
                    if button(g, mouse, px + 40.0, y, 24.0, 22.0, "-", false) && click && to > 0 {
                        let doc = self.doc();
                        doc.begin_edit();
                        doc.floor.exits[i].to = to - 1;
                    }
                    let to_txt = if to == 0 {
                        "SURFACE".to_string()
                    } else {
                        format!("F{to}")
                    };
                    g.draw_text(&to_txt, Vec2::new(px + 70.0, y + 16.0), 14.0, Color::WHITE);
                    if button(g, mouse, px + 130.0, y, 24.0, 22.0, "+", false) && click && to < 14 {
                        let doc = self.doc();
                        doc.begin_edit();
                        doc.floor.exits[i].to = to + 1;
                    }
                    if button(
                        g,
                        mouse,
                        px + 164.0,
                        y,
                        72.0,
                        22.0,
                        if open { "OPEN" } else { "CLOSED" },
                        open,
                    ) && click
                    {
                        let doc = self.doc();
                        doc.begin_edit();
                        doc.floor.exits[i].open = !open;
                    }
                    y += 28.0;
                }
                Item::Spawn(i) => {
                    let kind = f.spawns.get(i).map(|s| s.kind).unwrap_or(EnemyType::Idle);
                    g.draw_rectangle(Vec2::new(px, y + 3.0), 16.0, 16.0, spawn_color(kind));
                    if button(
                        g,
                        mouse,
                        px + 24.0,
                        y,
                        150.0,
                        22.0,
                        &format!("type: {}", enemy_type_id(kind)),
                        false,
                    ) && click
                    {
                        self.cycle_spawn_kind();
                    }
                    y += 28.0;
                }
                Item::Pickup(i) => {
                    let wp = f
                        .pickups
                        .get(i)
                        .map(|p| p.weapon)
                        .unwrap_or(WeaponType::Pistol);
                    if button(
                        g,
                        mouse,
                        px,
                        y,
                        170.0,
                        22.0,
                        &format!("weapon: {}", weapon_id(wp)),
                        false,
                    ) && click
                    {
                        self.cycle_weapon();
                    }
                    y += 28.0;
                }
                Item::Prop(_) => {
                    g.draw_text("rot", Vec2::new(px, y + 15.0), 13.0, Color::GRAY);
                    if button(g, mouse, px + 30.0, y, 34.0, 22.0, "-90", false) && click {
                        self.rotate_prop(-90.0);
                    }
                    if button(g, mouse, px + 68.0, y, 34.0, 22.0, "+90", false) && click {
                        self.rotate_prop(90.0);
                    }
                    g.draw_text("size", Vec2::new(px + 118.0, y + 15.0), 13.0, Color::GRAY);
                    if button(g, mouse, px + 152.0, y, 30.0, 22.0, "-", false) && click {
                        self.resize_prop(-10.0);
                    }
                    if button(g, mouse, px + 186.0, y, 30.0, 22.0, "+", false) && click {
                        self.resize_prop(10.0);
                    }
                    y += 28.0;
                }
                _ => {}
            }
            if item != Item::Entry && button(g, mouse, px, y, 80.0, 22.0, "DELETE", false) && click
            {
                let doc = self.doc();
                doc.begin_edit();
                doc.floor.delete(item);
                self.sel = None;
                self.note("deleted", now);
            }
            y += 30.0;
        }
        y = y.max(top + 150.0);
        g.draw_line(Vec2::new(x0 + 1.0, y), Vec2::new(w - 1.0, y), 1.0, BORDER);
        y += 16.0;

        if self.tool == Tool::Prop {
            self.draw_palette(g, mouse, click, px, y, time);
        } else {
            g.draw_text("KEYS", Vec2::new(px, y), 14.0, Color::GRAY);
            y += 18.0;
            for line in [
                "1-9        tools",
                "wheel      zoom   F fit",
                "mid/right  drag = pan (or Space+drag)",
                "G grid     N snap (10 units)",
                "click/drag select + move; handles resize",
                "arrows     nudge (Shift = 1 unit)",
                "Del        delete selection",
                "T / Q      spawn type / weapon",
                "R / [ ]    prop rotate / size",
                "Ctrl+Z / Ctrl+Y   undo / redo",
                "Esc        deselect / cancel",
                "Enter/Esc  commit / cancel a text field",
            ] {
                g.draw_text(
                    line,
                    Vec2::new(px, y),
                    13.0,
                    Color::new(0.75, 0.72, 0.85, 1.0),
                );
                y += 16.0;
            }
        }
    }

    /// The prop palette: family pages, a grid of live thumbnails (click to
    /// pick), and the brush rotation / size.
    fn draw_palette(
        &mut self,
        g: &Graphics,
        mouse: Vec2,
        click: bool,
        px: f32,
        mut y: f32,
        time: f32,
    ) {
        g.draw_text("PROP PALETTE", Vec2::new(px, y), 14.0, Color::GRAY);
        y += 8.0;
        for (fi, (name, first)) in PROP_FAMILIES.iter().enumerate() {
            let bx = px + fi as f32 * 78.0;
            if button(g, mouse, bx, y, 74.0, 22.0, name, self.family == fi)
                && click
                && self.family != fi
            {
                self.family = fi;
                self.brush = *first;
            }
        }
        y += 30.0;
        let cols = 4usize;
        let tile = 54.0;
        let mut hover_name: Option<&str> = None;
        for (slot, kind) in family_range(self.family).enumerate() {
            let bx = px + (slot % cols) as f32 * tile;
            let by = y + (slot / cols) as f32 * tile;
            let (bw, bh) = (tile - 4.0, tile - 4.0);
            let over = mouse.x >= bx && mouse.x <= bx + bw && mouse.y >= by && mouse.y <= by + bh;
            let picked = self.brush == kind;
            let bg = if picked {
                Color::new(1.0, 0.09, 0.26, 0.30)
            } else if over {
                Color::new(0.28, 0.22, 0.33, 1.0)
            } else {
                Color::new(0.13, 0.09, 0.17, 1.0)
            };
            g.draw_rectangle(Vec2::new(bx, by), bw, bh, bg);
            g.draw_rectangle_lines(
                Vec2::new(bx, by),
                bw,
                bh,
                1.0,
                if picked {
                    Color::new(1.0, 0.09, 0.26, 1.0)
                } else {
                    BORDER
                },
            );
            draw_prop(
                g,
                kind,
                Vec2::new(bx + bw / 2.0, by + bh / 2.0),
                snap_size(bh - 12.0, prop_px(kind)),
                time,
            );
            if over {
                hover_name = Some(PROP_NAMES[kind]);
                if click {
                    self.brush = kind;
                    self.tool = Tool::Prop;
                }
            }
        }
        let rows = family_range(self.family).len().div_ceil(cols);
        y += rows as f32 * tile + 6.0;
        g.draw_text(
            hover_name.unwrap_or(PROP_NAMES[self.brush % PROP_NAMES.len()]),
            Vec2::new(px, y + 12.0),
            14.0,
            Color::WHITE,
        );
        y += 22.0;
        g.draw_text("rot", Vec2::new(px, y + 15.0), 13.0, Color::GRAY);
        if button(g, mouse, px + 30.0, y, 34.0, 22.0, "-90", false) && click {
            self.rotate_prop(-90.0);
        }
        g.draw_text(
            &num(self.brush_rot),
            Vec2::new(px + 70.0, y + 16.0),
            14.0,
            Color::WHITE,
        );
        if button(g, mouse, px + 100.0, y, 34.0, 22.0, "+90", false) && click {
            self.rotate_prop(90.0);
        }
        g.draw_text("size", Vec2::new(px + 146.0, y + 15.0), 13.0, Color::GRAY);
        if button(g, mouse, px + 178.0, y, 26.0, 22.0, "-", false) && click {
            self.resize_prop(-10.0);
        }
        if button(g, mouse, px + 208.0, y, 26.0, 22.0, "+", false) && click {
            self.resize_prop(10.0);
        }
        y += 24.0;
        g.draw_text(
            &format!("size {}   (R rotates, [ ] resizes)", num(self.brush_size)),
            Vec2::new(px, y + 14.0),
            13.0,
            Color::GRAY,
        );
    }

    /// A one-line text field: click to focus (the buffer starts from the
    /// current value), Enter commits to the selection, Esc cancels.
    #[allow(clippy::too_many_arguments)]
    fn text_field(
        &mut self,
        g: &Graphics,
        mouse: Vec2,
        click: bool,
        x: f32,
        y: f32,
        name: &str,
        field: Field,
        value: &str,
    ) {
        let (bx, bw, bh) = (x + 44.0, PANEL_W - 24.0 - 44.0, 22.0);
        let focused = self.field == Some(field);
        g.draw_text(name, Vec2::new(x, y + 15.0), 13.0, Color::GRAY);
        let over = mouse.x >= bx && mouse.x <= bx + bw && mouse.y >= y && mouse.y <= y + bh;
        g.draw_rectangle(Vec2::new(bx, y), bw, bh, Color::new(0.05, 0.03, 0.08, 1.0));
        g.draw_rectangle_lines(
            Vec2::new(bx, y),
            bw,
            bh,
            1.0,
            if focused {
                CYAN
            } else if over {
                Color::WHITE
            } else {
                BORDER
            },
        );
        let shown = if focused {
            format!("{}_", self.buf)
        } else {
            value.to_string()
        };
        g.draw_text(&shown, Vec2::new(bx + 5.0, y + 16.0), 14.0, Color::WHITE);
        if over && click {
            self.field = Some(field);
            self.buf = value.to_string();
        }
    }

    /// The status line under the map: validation, counts, cursor position,
    /// and the transient note.
    fn draw_status(&mut self, g: &Graphics, vp: Rect, mouse: Vec2, over_map: bool, now: f64) {
        let y = vp.y + vp.h;
        g.draw_rectangle(Vec2::new(0.0, y), vp.w, STATUS_H, PANEL_BG);
        g.draw_line(Vec2::new(0.0, y), Vec2::new(vp.w, y), 1.0, BORDER);
        let ids = self.known_ids.clone();
        let (problems, counts) = {
            let f = self.floor();
            (
                f.validate(&ids),
                format!(
                    "{} walls · {} rooms · {} zones · {} spawns · {} pickups · {} props · {} steps",
                    f.walls.len(),
                    f.rooms.len(),
                    f.zones.len(),
                    f.spawns.len(),
                    f.pickups.len(),
                    f.props.len(),
                    f.scenario.len()
                ),
            )
        };
        let (msg, col) = if now - self.note_at < 4000.0 && !self.note.is_empty() {
            (self.note.clone(), GOLD)
        } else if problems.is_empty() {
            (format!("valid · {counts}"), OK_GREEN)
        } else {
            (
                format!(
                    "{} problem{}: {}",
                    problems.len(),
                    if problems.len() == 1 { "" } else { "s" },
                    problems[0]
                ),
                BAD_RED,
            )
        };
        g.draw_text(&msg, Vec2::new(14.0, y + 22.0), 15.0, col);
        if over_map {
            let wp = self.to_world(mouse);
            let pos = format!(
                "x {}  y {}   zoom {:.2}",
                num(wp.x.round()),
                num(wp.y.round()),
                self.zoom
            );
            let tw = pos.chars().count() as f32 * 14.0 * CHAR_W;
            g.draw_text(
                &pos,
                Vec2::new(vp.w - tw - 14.0, y + 22.0),
                14.0,
                Color::GRAY,
            );
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

fn tool_name(t: Tool) -> &'static str {
    match t {
        Tool::Select => "selection",
        Tool::Wall => "wall",
        Tool::Room => "room",
        Tool::Zone => "zone",
        Tool::Spawn => "spawn",
        Tool::Pickup => "pickup",
        Tool::Entry => "entry",
        Tool::Exit => "exit",
        Tool::Prop => "prop",
    }
}
