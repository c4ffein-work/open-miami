//! The prop sprite library: TOP-DOWN set dressing drawn entirely from the 2D
//! command-stream primitives (no assets, no new dependencies) and animated by
//! the continuous clock, in three FAMILIES ([`PROP_FAMILIES`], contiguous id
//! ranges): DATACENTER — the server floors' racks, switching, cooling, power,
//! storage and hazard furniture (blinking LEDs, spinning roof fans, rising
//! bubbles, a patrolling tape-picker arm) —, OUTDOOR — the night-time gate /
//! parking lot of the planned floor 00 (autonomous cars with their lidar
//! pucks, a charging pad, the main gate's swing arm, a guard booth, lamp
//! posts, road decals, a landed drone, a holo billboard) — and LOBBY — the
//! ground-floor welcome hall (reception desk, turnstiles, scanner arch,
//! benches, holo screens, vending, lockers, the floor logo).
//!
//! Every prop is seen from straight above, matching the game camera: what you
//! draw is the object's top surface. Conventions shared by the set — the
//! machine's front face is the bottom edge (+y), where its status LEDs live;
//! light comes from the top-left, so tall props cast a small drop shadow
//! down-right; screens are edge-on slabs that wash their light across the
//! floor or desk in front of them.
//!
//! Each prop is designed in a local 100x100 box centred on the origin
//! (y down) and drawn through the transform stack, so it can be placed at
//! any size.
//!
//! # Layers
//!
//! A prop is a list of LAYERS ([`PROP_LAYERS`], one [`LayerDef`] each),
//! drawn bottom to top. A layer is drawn in its OWN local frame — origin at
//! its `pivot` (prop coordinates), unrotated — and the driver applies the
//! layer's rotation ([`LayerRot`]: none, a static angle, a spin, a sway or an
//! arbitrary `fn(t) -> rad`) around that pivot. This is what makes props
//! pixelatable per layer: with a pixel size `px` (in design units, so an art
//! pixel scales with the prop) each layer is rasterized in its own pixel-art
//! group (`Graphics::pixel_begin` / `pixel_end`) either
//!
//! * BEFORE its rotation ([`PixelMode::Before`]): the layer is rasterized
//!   unrotated on its own grid and the finished pixel image is rotated as a
//!   whole (a rotated sprite: bodies, lids, panels), or
//! * AFTER it ([`PixelMode::After`]): the group is opened in the parent's
//!   frame and the rotation happens inside it, so the layer is re-rasterized
//!   on the parent's grid every frame (fans, camera heads, needles: the
//!   blades animate through a fixed grid).
//!
//! `px <= 1` draws the layers directly (no groups) and is pixel-for-pixel
//! the pre-layer look. The saved per-prop `px` and per-layer modes live in
//! `props/props.json` (compiled to `src/props_data.rs` by `make gen-props`,
//! see `docs/PROPS_FORMAT.md`); [`draw_prop`] uses them, [`draw_prop_ex`]
//! takes them from the caller (the `?viz` PROPS page edits and saves them).

use crate::props_data::PROP_SETTINGS;
use std::f32::consts::{PI, TAU};

/// Display names, indexed by prop id (the order of the library).
pub const PROP_NAMES: [&str; 60] = [
    "RACK / CLOSED",
    "RACK / OPEN",
    "RACK / BURNT",
    "BLADE STACK",
    "CORE SWITCH",
    "CABLE JUNCTION",
    "OPERATOR DESK",
    "CONTROL CONSOLE",
    "HOLO TABLE",
    "CRAC COOLER",
    "FLOOR VENT",
    "EXHAUST FAN",
    "COOLANT TANK",
    "PIPE RUN",
    "UPS CABINET",
    "GENERATOR",
    "CABLE TRAY",
    "CABLE COIL",
    "TAPE LIBRARY",
    "SUPPLY CRATE",
    "SECURITY CAM",
    "FIRE SUPPRESSOR",
    "HAZARD PAD",
    "UPLINK OBELISK",
    // ---- OUTDOOR: the gate / parking lot (floor 00, night, neon) ---------
    "CAR / POD",
    "CAR / SEDAN",
    "CAR / OPEN",
    "DELIVERY VAN",
    "CHARGE PAD",
    "CAR / CHARGING",
    "MAIN GATE",
    "GUARD BOOTH",
    "BOLLARDS",
    "PLANTER",
    "LAMP POST",
    "EV BAY",
    "CROSSWALK",
    "DRONE PAD",
    "SCOOTER RACK",
    "DRAIN GRATE",
    "HOLO BILLBOARD",
    "DUMPSTER",
    // ---- LOBBY: the ground-floor welcome hall -----------------------------
    "RECEPTION DESK",
    "TURNSTILES",
    "SCANNER ARCH",
    "BENCH / LONG",
    "BENCH / SHORT",
    "POTTED PLANT",
    "LOBBY HOLO",
    "DIRECTORY TOTEM",
    "VENDING MACHINE",
    "COFFEE CORNER",
    "CHARGE LOCKERS",
    "FLOOR LOGO",
    "CALL PANEL",
    "VELVET ROPE",
    "EXTINGUISHER",
    "CREDIT KIOSK",
    "WALL CLOCK",
    "WELCOME MAT",
];

/// Number of props in the library.
pub const PROP_COUNT: usize = PROP_NAMES.len();

/// The prop FAMILIES (the `?viz` PROPS gallery pages): display name and the
/// id of the family's first prop. Families are contiguous id ranges, in
/// library order — DATACENTER (the server floors), OUTDOOR (the gate /
/// parking lot of the planned floor 00) and LOBBY (the ground-floor welcome
/// hall).
pub const PROP_FAMILIES: [(&str, usize); 3] = [("DATACENTER", 0), ("OUTDOOR", 24), ("LOBBY", 42)];

/// The id range of family `family` (see [`PROP_FAMILIES`]).
pub fn family_range(family: usize) -> std::ops::Range<usize> {
    let f = family % PROP_FAMILIES.len();
    let start = PROP_FAMILIES[f].1;
    let end = PROP_FAMILIES.get(f + 1).map(|n| n.1).unwrap_or(PROP_COUNT);
    start..end
}

/// The family of prop `kind`.
pub fn prop_family(kind: usize) -> usize {
    let kind = kind % PROP_COUNT;
    (0..PROP_FAMILIES.len())
        .rev()
        .find(|&f| PROP_FAMILIES[f].1 <= kind)
        .unwrap_or(0)
}

/// Most props in any one family (sizes the gallery grid).
pub fn largest_family() -> usize {
    (0..PROP_FAMILIES.len())
        .map(|f| family_range(f).len())
        .max()
        .unwrap_or(0)
}

/// Upper bound on layers per prop (the per-layer settings arrays / the
/// visibility bitmask are sized by it).
pub const MAX_LAYERS: usize = 8;

/// Largest saved / selectable art-pixel size (design units; 1 = off).
pub const MAX_PX: u32 = 10;

/// When a layer's pixel-art group is rasterized relative to its rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelMode {
    /// Rasterize the layer unrotated on its own grid, then rotate the pixel
    /// image as a whole (a rotated sprite).
    Before,
    /// Rotate first, then rasterize on the parent's grid (re-rasterized every
    /// frame as it turns).
    After,
}

impl PixelMode {
    /// The JSON id (`"before"` / `"after"`).
    pub fn id(self) -> &'static str {
        match self {
            PixelMode::Before => "before",
            PixelMode::After => "after",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "before" => Some(PixelMode::Before),
            "after" => Some(PixelMode::After),
            _ => None,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            PixelMode::Before => PixelMode::After,
            PixelMode::After => PixelMode::Before,
        }
    }
}

/// How a layer rotates around its pivot, as a function of the clock.
#[derive(Clone, Copy)]
pub enum LayerRot {
    /// Fixed to the parent.
    None,
    /// A constant angle (degrees).
    Static(f32),
    /// Continuous spin at `hz` revolutions per second (negative = the other
    /// way).
    Spin { hz: f32 },
    /// Oscillation of ± `deg` degrees at `hz` cycles per second.
    Sway { deg: f32, hz: f32 },
    /// Anything else: the angle in radians from the clock.
    Anim(fn(f32) -> f32),
}

impl LayerRot {
    /// The layer's angle (radians) at clock `t`.
    pub fn angle(&self, t: f32) -> f32 {
        match *self {
            LayerRot::None => 0.0,
            LayerRot::Static(deg) => deg * (PI / 180.0),
            LayerRot::Spin { hz } => t * hz * TAU,
            LayerRot::Sway { deg, hz } => deg * (PI / 180.0) * (t * hz * TAU).sin(),
            LayerRot::Anim(f) => f(t),
        }
    }

    /// Short human description for the inspector.
    pub fn label(&self) -> String {
        match *self {
            LayerRot::None => "fixed".to_string(),
            LayerRot::Static(deg) => format!("static {:+.0}°", deg),
            LayerRot::Spin { hz } => format!("spin {:+.2} Hz", hz),
            LayerRot::Sway { deg, hz } => format!("sway ±{:.0}° {:.2} Hz", deg, hz),
            LayerRot::Anim(_) => "anim".to_string(),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, LayerRot::None)
    }
}

/// One drawable layer of a prop.
#[derive(Clone, Copy)]
pub struct LayerDef {
    /// Unique within the prop; the key in `props/props.json`.
    pub name: &'static str,
    /// The layer's origin / rotation centre, in prop coordinates.
    pub pivot: (f32, f32),
    /// Local AABB `(x, y, w, h)` of what the layer draws, in ITS frame
    /// (relative to the pivot, unrotated): sizes its pixel group.
    pub bounds: (f32, f32, f32, f32),
    pub rot: LayerRot,
    /// The default pixel mode (`props.json` may override it).
    pub pixel: PixelMode,
}

/// Degrees per radian, for the static angles the props were designed in.
const DEG: f32 = 180.0 / PI;

const fn layer(
    name: &'static str,
    pivot: (f32, f32),
    bounds: (f32, f32, f32, f32),
    rot: LayerRot,
    pixel: PixelMode,
) -> LayerDef {
    LayerDef {
        name,
        pivot,
        bounds,
        rot,
        pixel,
    }
}

use PixelMode::{After, Before};

// ---- the shared animation curves of the OUTDOOR / LOBBY layers -----------

/// Smooth 0..1 ramp of `x` over `a..b` (constant outside).
fn smooth(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The main gate's swing arm: closed across the lane, then a slow 90° swing
/// open along the lane (radians, negative = towards -y), a hold, and back —
/// a 14 s cycle.
fn gate_angle(t: f32) -> f32 {
    let c = t.rem_euclid(14.0);
    let open = smooth(c, 6.0, 8.2) - smooth(c, 12.0, 13.8);
    -open * (PI / 2.0)
}

/// The turnstile's free lane: the arm swings 75° with the walker (towards
/// -y) and drops back — a 6 s cycle, most of it closed.
fn turnstile_angle(t: f32) -> f32 {
    let c = t.rem_euclid(6.0);
    let open = smooth(c, 3.0, 3.6) - smooth(c, 5.2, 5.8);
    open * (75.0 * PI / 180.0)
}

/// A landed drone's rotor idling: still, with a short shiver now and then
/// (each rotor on its own phase).
fn rotor_twitch(t: f32, phase: f32) -> f32 {
    if (t * 0.45 + phase).sin() > 0.82 {
        0.55 * (t * 31.0 + phase).sin()
    } else {
        0.0
    }
}

/// The layers of every prop, bottom to top (index = prop id, see
/// [`PROP_NAMES`]). The drawing itself is `draw_prop_layer` (wasm only).
pub static PROP_LAYERS: [&[LayerDef]; PROP_COUNT] = [
    // 0 RACK / CLOSED
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-36.0, -50.0, 76.0, 100.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "fan a",
            (0.0, -15.0),
            (-14.0, -14.0, 28.0, 28.0),
            LayerRot::Spin { hz: 3.4 / TAU },
            After,
        ),
        layer(
            "fan b",
            (0.0, 15.0),
            (-14.0, -14.0, 28.0, 28.0),
            LayerRot::Anim(|t| (t + 0.4) * -2.9),
            After,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-35.0, 37.0, 70.0, 8.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 1 RACK / OPEN
    &[
        layer(
            "chassis",
            (0.0, 0.0),
            (-36.0, -46.0, 76.0, 96.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "fan",
            (0.0, -12.0),
            (-11.0, -11.0, 22.0, 22.0),
            LayerRot::Spin { hz: 4.6 / TAU },
            After,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-26.0, 24.0, 3.0, 13.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 2 RACK / BURNT
    &[
        layer(
            "lid",
            (36.0, 8.0),
            (-9.0, -28.0, 18.0, 56.0),
            LayerRot::Static(0.25 * DEG),
            Before,
        ),
        layer(
            "hull",
            (0.0, 0.0),
            (-41.0, -46.0, 70.0, 96.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "sparks",
            (0.0, 0.0),
            (-25.0, -33.0, 40.0, 50.0),
            LayerRot::None,
            After,
        ),
    ],
    // 3 BLADE STACK
    &[
        layer(
            "enclosure",
            (0.0, 0.0),
            (-41.0, -43.0, 86.0, 90.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-40.0, 34.0, 80.0, 8.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 4 CORE SWITCH
    &[
        layer(
            "chassis",
            (0.0, 0.0),
            (-46.0, -23.0, 96.0, 46.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "ports",
            (0.0, 0.0),
            (-45.0, 12.0, 90.0, 6.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "cables",
            (0.0, 0.0),
            (-40.0, 18.0, 90.0, 26.0),
            LayerRot::None,
            After,
        ),
    ],
    // 5 CABLE JUNCTION
    &[
        layer(
            "boxes",
            (0.0, 0.0),
            (-46.0, -31.0, 96.0, 66.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "cables",
            (0.0, 0.0),
            (-28.0, -27.0, 56.0, 54.0),
            LayerRot::None,
            After,
        ),
    ],
    // 6 OPERATOR DESK
    &[
        layer(
            "desk",
            (0.0, 0.0),
            (-46.0, -29.0, 96.0, 58.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "paper a",
            (-33.0, 4.0),
            (-8.0, -10.0, 16.0, 20.0),
            LayerRot::Static(-0.2 * DEG),
            Before,
        ),
        layer(
            "paper b",
            (-30.0, 7.0),
            (-8.0, -10.0, 16.0, 20.0),
            LayerRot::Static(0.15 * DEG),
            Before,
        ),
        layer(
            "mug",
            (0.0, 0.0),
            (30.0, 8.0, 15.0, 12.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 7 CONTROL CONSOLE
    &[
        layer(
            "shadow",
            (0.0, 0.0),
            (-30.0, -30.0, 68.0, 26.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "wing l",
            (-40.0, -16.0),
            (-16.0, -12.0, 32.0, 24.0),
            LayerRot::Static(0.55 * DEG),
            Before,
        ),
        layer(
            "wing r",
            (40.0, -16.0),
            (-16.0, -12.0, 32.0, 24.0),
            LayerRot::Static(-0.55 * DEG),
            Before,
        ),
        layer(
            "desk",
            (0.0, 0.0),
            (-34.0, -34.0, 68.0, 26.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "screen l",
            (-27.0, -24.0),
            (-16.0, -3.0, 32.0, 26.0),
            LayerRot::Static(0.5 * DEG),
            Before,
        ),
        layer(
            "screen c",
            (0.0, -24.0),
            (-16.0, -3.0, 32.0, 26.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "screen r",
            (27.0, -24.0),
            (-16.0, -3.0, 32.0, 26.0),
            LayerRot::Static(-0.5 * DEG),
            Before,
        ),
        layer(
            "chair",
            (0.0, 0.0),
            (-16.0, 12.0, 34.0, 30.0),
            LayerRot::None,
            After,
        ),
    ],
    // 8 HOLO TABLE
    &[
        layer(
            "pedestal",
            (0.0, 0.0),
            (-28.0, -14.0, 60.0, 60.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "hologram",
            (0.0, 0.0),
            (-27.0, -29.0, 54.0, 42.0),
            LayerRot::None,
            After,
        ),
    ],
    // 9 CRAC COOLER
    &[
        layer(
            "housing",
            (0.0, 0.0),
            (-41.0, -46.0, 86.0, 96.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "blower",
            (0.0, 10.0),
            (-27.0, -27.0, 54.0, 54.0),
            LayerRot::Spin { hz: 4.0 / TAU },
            After,
        ),
        layer(
            "led",
            (0.0, 0.0),
            (-40.0, 39.0, 80.0, 6.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 10 FLOOR VENT
    &[
        layer(
            "grille",
            (0.0, 0.0),
            (-43.0, -43.0, 86.0, 86.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "airflow",
            (0.0, 0.0),
            (-39.0, -39.0, 78.0, 78.0),
            LayerRot::None,
            After,
        ),
    ],
    // 11 EXHAUST FAN
    &[
        layer(
            "duct",
            (0.0, 0.0),
            (-43.0, -43.0, 86.0, 86.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "blades",
            (0.0, 0.0),
            (-33.0, -33.0, 66.0, 66.0),
            LayerRot::Spin { hz: 3.2 / TAU },
            After,
        ),
        layer(
            "guard",
            (0.0, 0.0),
            (-37.0, -37.0, 74.0, 74.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 12 COOLANT TANK
    &[
        layer(
            "tank",
            (0.0, 0.0),
            (-39.0, -39.0, 82.0, 82.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "bubbles",
            (0.0, 0.0),
            (-22.0, -24.0, 44.0, 48.0),
            LayerRot::None,
            After,
        ),
        layer(
            "hatch",
            (0.0, 0.0),
            (-10.0, -10.0, 20.0, 20.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 13 PIPE RUN
    &[
        layer(
            "pipes",
            (0.0, 0.0),
            (-50.0, -23.0, 100.0, 50.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "valve",
            (-34.0, 13.0),
            (-10.0, -10.0, 20.0, 20.0),
            LayerRot::Sway {
                deg: 0.3 * DEG,
                hz: 0.6 / TAU,
            },
            After,
        ),
    ],
    // 14 UPS CABINET
    &[
        layer(
            "cabinet",
            (0.0, 0.0),
            (-31.0, -50.0, 66.0, 100.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "bolt",
            (0.0, 0.0),
            (-6.0, 14.0, 12.0, 20.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "charge",
            (0.0, 0.0),
            (-30.0, 37.0, 60.0, 8.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 15 GENERATOR
    &[
        layer(
            "block",
            (0.0, 0.0),
            (-46.0, -26.0, 96.0, 70.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "smoke",
            (-31.0, -36.0),
            (-21.0, -21.0, 42.0, 42.0),
            LayerRot::None,
            After,
        ),
        layer(
            "stack",
            (0.0, 0.0),
            (-38.0, -43.0, 14.0, 21.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "gauge",
            (-12.0, 8.0),
            (-7.0, -7.0, 14.0, 14.0),
            LayerRot::Anim(|t| -1.9 + 0.12 * (t * 9.0).sin()),
            After,
        ),
    ],
    // 16 CABLE TRAY
    &[
        layer(
            "tray",
            (0.0, 0.0),
            (-48.0, -24.0, 96.0, 48.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "cables",
            (0.0, 0.0),
            (-47.0, -17.0, 94.0, 36.0),
            LayerRot::None,
            After,
        ),
    ],
    // 17 CABLE COIL
    &[
        layer(
            "coil",
            (0.0, 0.0),
            (-34.0, -34.0, 72.0, 72.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glints",
            (0.0, 0.0),
            (-30.0, -30.0, 60.0, 60.0),
            LayerRot::Spin { hz: 0.4 / TAU },
            After,
        ),
        layer(
            "lead",
            (0.0, 0.0),
            (26.0, 12.0, 23.0, 21.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 18 TAPE LIBRARY
    &[
        layer(
            "chassis",
            (0.0, 0.0),
            (-43.0, -46.0, 90.0, 96.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "picker",
            (0.0, 0.0),
            (-24.0, -37.0, 48.0, 74.0),
            LayerRot::None,
            After,
        ),
    ],
    // 19 SUPPLY CRATE
    &[
        layer(
            "crate",
            (0.0, 0.0),
            (-39.0, -33.0, 82.0, 70.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "stencil",
            (0.0, 0.0),
            (-24.0, 10.0, 50.0, 16.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 20 SECURITY CAM
    &[
        layer(
            "mount",
            (0.0, 0.0),
            (-17.0, -49.0, 34.0, 12.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "cone",
            (0.0, -30.0),
            (-20.0, 0.0, 40.0, 65.0),
            LayerRot::Sway {
                deg: 0.35 * DEG,
                hz: 0.5 / TAU,
            },
            After,
        ),
        layer(
            "head",
            (0.0, -30.0),
            (-8.0, -7.0, 16.0, 31.0),
            LayerRot::Sway {
                deg: 0.35 * DEG,
                hz: 0.5 / TAU,
            },
            After,
        ),
    ],
    // 21 FIRE SUPPRESSOR
    &[
        layer(
            "manifold",
            (0.0, 0.0),
            (-31.0, -45.0, 62.0, 34.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "tanks",
            (0.0, 0.0),
            (-36.0, -10.0, 76.0, 40.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "wheel l",
            (-18.0, 8.0),
            (-9.0, -9.0, 18.0, 18.0),
            LayerRot::Anim(|t| 0.25 * (t * 0.5).sin()),
            After,
        ),
        layer(
            "wheel r",
            (18.0, 8.0),
            (-9.0, -9.0, 18.0, 18.0),
            LayerRot::Anim(|t| 0.25 * (t * 0.8).sin() + 1.0),
            After,
        ),
        layer(
            "tag",
            (0.0, 0.0),
            (-4.0, 32.0, 8.0, 10.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 22 HAZARD PAD
    &[
        layer(
            "pad",
            (0.0, 0.0),
            (-44.0, -44.0, 88.0, 88.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "sign",
            (0.0, 0.0),
            (-18.0, -20.0, 36.0, 34.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 23 UPLINK OBELISK
    &[
        layer(
            "aura",
            (0.0, 0.0),
            (-44.0, -44.0, 88.0, 88.0),
            LayerRot::None,
            After,
        ),
        layer(
            "shadow",
            (4.0, 4.0),
            (-16.0, -16.0, 32.0, 32.0),
            LayerRot::Static(45.0),
            Before,
        ),
        layer(
            "monolith",
            (0.0, 0.0),
            (-17.0, -17.0, 34.0, 34.0),
            LayerRot::Static(45.0),
            Before,
        ),
        layer(
            "seams",
            (0.0, 0.0),
            (-22.0, -22.0, 44.0, 44.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "escort",
            (0.0, 0.0),
            (-35.0, -39.0, 70.0, 78.0),
            LayerRot::None,
            After,
        ),
    ],
    // ======================= OUTDOOR: gate / parking lot =======================
    // 24 CAR / POD
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-21.0, -30.0, 46.0, 64.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glass",
            (0.0, 0.0),
            (-15.0, -22.0, 30.0, 44.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lidar",
            (0.0, -2.0),
            (-6.0, -6.0, 12.0, 12.0),
            LayerRot::Spin { hz: 0.8 },
            After,
        ),
        layer(
            "lights",
            (0.0, 0.0),
            (-21.0, -31.0, 42.0, 66.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 25 CAR / SEDAN
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-23.0, -44.0, 50.0, 84.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glass",
            (0.0, 0.0),
            (-19.0, -30.0, 38.0, 58.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lidar",
            (0.0, -8.0),
            (-6.0, -6.0, 12.0, 12.0),
            LayerRot::Spin { hz: 1.2 },
            After,
        ),
        layer(
            "lights",
            (0.0, 0.0),
            (-31.0, -45.0, 62.0, 100.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 26 CAR / OPEN
    &[
        layer(
            "spill",
            (0.0, 0.0),
            (-48.0, -26.0, 96.0, 52.0),
            LayerRot::None,
            After,
        ),
        layer(
            "body",
            (0.0, 0.0),
            (-23.0, -44.0, 50.0, 84.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "door l",
            (-21.0, -6.0),
            (-3.0, -1.0, 6.0, 26.0),
            LayerRot::Static(70.0),
            Before,
        ),
        layer(
            "door r",
            (21.0, -6.0),
            (-3.0, -1.0, 6.0, 26.0),
            LayerRot::Static(-70.0),
            Before,
        ),
        layer(
            "cabin",
            (0.0, 0.0),
            (-19.0, -30.0, 38.0, 58.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lidar",
            (0.0, -8.0),
            (-6.0, -6.0, 12.0, 12.0),
            LayerRot::Spin { hz: 0.25 },
            After,
        ),
        layer(
            "lights",
            (0.0, 0.0),
            (-23.0, -45.0, 46.0, 84.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 27 DELIVERY VAN
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-25.0, -46.0, 54.0, 96.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glass",
            (0.0, 0.0),
            (-25.0, 20.0, 50.0, 22.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "puck",
            (0.0, 26.0),
            (-5.0, -5.0, 10.0, 10.0),
            LayerRot::Spin { hz: 0.6 },
            After,
        ),
        layer(
            "lights",
            (0.0, 0.0),
            (-31.0, -47.0, 62.0, 100.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 28 CHARGE PAD
    &[
        layer(
            "pad",
            (0.0, 0.0),
            (-31.0, -46.0, 62.0, 80.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "rings",
            (0.0, 0.0),
            (-28.0, -28.0, 56.0, 56.0),
            LayerRot::None,
            After,
        ),
    ],
    // 29 CAR / CHARGING
    &[
        layer(
            "pad",
            (0.0, 0.0),
            (-31.0, -46.0, 62.0, 80.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "rings",
            (0.0, 0.0),
            (-32.0, -38.0, 64.0, 64.0),
            LayerRot::None,
            After,
        ),
        layer(
            "body",
            (0.0, 0.0),
            (-21.0, -36.0, 46.0, 64.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glass",
            (0.0, 0.0),
            (-15.0, -28.0, 30.0, 44.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lidar",
            (0.0, -8.0),
            (-6.0, -6.0, 12.0, 12.0),
            LayerRot::Spin { hz: 0.15 },
            After,
        ),
        layer(
            "charge",
            (0.0, 0.0),
            (-24.0, -38.0, 48.0, 62.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 30 MAIN GATE
    &[
        layer(
            "lane",
            (0.0, 0.0),
            (-34.0, -50.0, 68.0, 100.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "posts",
            (0.0, 0.0),
            (-48.0, 4.0, 96.0, 20.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "scan",
            (0.0, 0.0),
            (-46.0, 6.0, 92.0, 12.0),
            LayerRot::None,
            After,
        ),
        layer(
            "arm shadow",
            (-32.0, 16.0),
            (-1.0, -3.0, 64.0, 6.0),
            LayerRot::Anim(gate_angle),
            After,
        ),
        layer(
            "arm",
            (-36.0, 12.0),
            (-6.0, -6.0, 70.0, 12.0),
            LayerRot::Anim(gate_angle),
            After,
        ),
    ],
    // 31 GUARD BOOTH
    &[
        layer(
            "wash",
            (0.0, 0.0),
            (20.0, -30.0, 30.0, 60.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "booth",
            (0.0, 0.0),
            (-30.0, -30.0, 58.0, 66.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "ac fan",
            (-14.0, -12.0),
            (-7.0, -7.0, 14.0, 14.0),
            LayerRot::Spin { hz: 2.0 },
            After,
        ),
        layer(
            "beacon",
            (12.0, -18.0),
            (-14.0, -14.0, 28.0, 28.0),
            LayerRot::Spin { hz: 0.7 },
            After,
        ),
    ],
    // 32 BOLLARDS
    &[
        layer(
            "posts",
            (0.0, 0.0),
            (-42.0, -10.0, 90.0, 26.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-36.0, -4.0, 72.0, 8.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 33 PLANTER
    &[
        layer(
            "box",
            (0.0, 0.0),
            (-36.0, -20.0, 76.0, 46.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "shrub",
            (0.0, -2.0),
            (-34.0, -16.0, 68.0, 32.0),
            LayerRot::Sway { deg: 2.0, hz: 0.25 },
            After,
        ),
        layer(
            "lamp",
            (0.0, 0.0),
            (26.0, -18.0, 12.0, 12.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 34 LAMP POST
    &[
        layer(
            "pool",
            (0.0, 0.0),
            (-36.0, -52.0, 92.0, 92.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "shadow",
            (0.0, 0.0),
            (-20.0, 8.0, 56.0, 44.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "mast",
            (0.0, 0.0),
            (-26.0, -14.0, 42.0, 42.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "head",
            (0.0, 0.0),
            (0.0, -14.0, 20.0, 16.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "moths",
            (10.0, -6.0),
            (-16.0, -16.0, 32.0, 32.0),
            LayerRot::None,
            After,
        ),
    ],
    // 35 EV BAY
    &[
        layer(
            "asphalt",
            (0.0, 0.0),
            (-45.0, -45.0, 90.0, 90.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lines",
            (0.0, 0.0),
            (-40.0, -44.0, 80.0, 84.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glyph",
            (0.0, 0.0),
            (-20.0, -30.0, 40.0, 56.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 36 CROSSWALK
    &[
        layer(
            "asphalt",
            (0.0, 0.0),
            (-45.0, -45.0, 90.0, 90.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "stripes",
            (0.0, 0.0),
            (-44.0, -30.0, 88.0, 60.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 37 DRONE PAD
    &[
        layer(
            "pad",
            (0.0, 0.0),
            (-44.0, -44.0, 88.0, 88.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "beacons",
            (0.0, 0.0),
            (-34.0, -34.0, 68.0, 68.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "drone",
            (0.0, 0.0),
            (-22.0, -22.0, 48.0, 48.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "rotor a",
            (-16.0, -16.0),
            (-11.0, -2.0, 22.0, 4.0),
            LayerRot::Anim(|t| rotor_twitch(t, 0.0)),
            After,
        ),
        layer(
            "rotor b",
            (16.0, -16.0),
            (-11.0, -2.0, 22.0, 4.0),
            LayerRot::Anim(|t| rotor_twitch(t, 1.7)),
            After,
        ),
        layer(
            "rotor c",
            (-16.0, 16.0),
            (-11.0, -2.0, 22.0, 4.0),
            LayerRot::Anim(|t| rotor_twitch(t, 3.9)),
            After,
        ),
        layer(
            "rotor d",
            (16.0, 16.0),
            (-11.0, -2.0, 22.0, 4.0),
            LayerRot::Anim(|t| rotor_twitch(t, 5.1)),
            After,
        ),
    ],
    // 38 SCOOTER RACK
    &[
        layer(
            "rail",
            (0.0, 0.0),
            (-44.0, -36.0, 92.0, 16.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "scooter a",
            (-28.0, 0.0),
            (-9.0, -31.0, 22.0, 66.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "scooter b",
            (0.0, 0.0),
            (-9.0, -31.0, 22.0, 66.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "scooter c",
            (28.0, 2.0),
            (-9.0, -31.0, 22.0, 66.0),
            LayerRot::Static(9.0),
            Before,
        ),
    ],
    // 39 DRAIN GRATE
    &[
        layer(
            "puddle",
            (0.0, 0.0),
            (-30.0, -28.0, 66.0, 66.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "grate",
            (0.0, 0.0),
            (-24.0, -14.0, 48.0, 28.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "sheen",
            (0.0, 0.0),
            (-30.0, -28.0, 66.0, 66.0),
            LayerRot::None,
            After,
        ),
    ],
    // 40 HOLO BILLBOARD
    &[
        layer(
            "wash",
            (0.0, 0.0),
            (-50.0, -26.0, 100.0, 76.0),
            LayerRot::None,
            After,
        ),
        layer(
            "shadow",
            (0.0, 0.0),
            (-32.0, -32.0, 96.0, 24.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "slab",
            (0.0, 0.0),
            (-42.0, -46.0, 84.0, 24.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glyphs",
            (0.0, 0.0),
            (-42.0, -33.0, 84.0, 9.0),
            LayerRot::None,
            After,
        ),
    ],
    // 41 DUMPSTER
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-30.0, -22.0, 66.0, 50.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lid l",
            (0.0, 0.0),
            (-30.0, -22.0, 30.0, 44.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "lid r",
            (0.0, -22.0),
            (0.0, -9.0, 30.0, 9.0),
            LayerRot::Static(-4.0),
            Before,
        ),
        layer(
            "flies",
            (0.0, 0.0),
            (0.0, -18.0, 32.0, 32.0),
            LayerRot::None,
            After,
        ),
    ],
    // ========================= LOBBY: the welcome hall =========================
    // 42 RECEPTION DESK
    &[
        layer(
            "shadow",
            (0.0, 0.0),
            (-40.0, -10.0, 84.0, 24.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "wing l",
            (-40.0, 0.0),
            (-26.0, -10.0, 30.0, 24.0),
            LayerRot::Static(-25.0),
            Before,
        ),
        layer(
            "wing r",
            (40.0, 0.0),
            (-4.0, -10.0, 34.0, 24.0),
            LayerRot::Static(25.0),
            Before,
        ),
        layer(
            "desk",
            (0.0, 0.0),
            (-40.0, -18.0, 80.0, 30.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "terminal",
            (0.0, -4.0),
            (-21.0, -26.0, 42.0, 32.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "chair",
            (0.0, 0.0),
            (-16.0, -42.0, 34.0, 32.0),
            LayerRot::None,
            After,
        ),
    ],
    // 43 TURNSTILES
    &[
        layer(
            "floor",
            (0.0, 0.0),
            (-46.0, -30.0, 92.0, 62.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "housings",
            (0.0, 0.0),
            (-46.0, -20.0, 96.0, 48.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "arm l",
            (-34.0, 0.0),
            (-4.0, -4.0, 34.0, 8.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "arm r",
            (34.0, 0.0),
            (-30.0, -4.0, 34.0, 8.0),
            LayerRot::Anim(turnstile_angle),
            After,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-44.0, 12.0, 88.0, 6.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 44 SCANNER ARCH
    &[
        layer(
            "mat",
            (0.0, 0.0),
            (-40.0, -26.0, 80.0, 52.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "arch",
            (0.0, 0.0),
            (-36.0, -20.0, 80.0, 50.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "sweep",
            (0.0, 0.0),
            (-24.0, -19.0, 48.0, 38.0),
            LayerRot::None,
            After,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-34.0, -19.0, 68.0, 8.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 45 BENCH / LONG
    &[
        layer(
            "bench",
            (0.0, 0.0),
            (-44.0, -20.0, 92.0, 42.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "items",
            (0.0, 0.0),
            (-30.0, -8.0, 60.0, 16.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 46 BENCH / SHORT
    &[
        layer(
            "bench",
            (0.0, 0.0),
            (-30.0, -20.0, 64.0, 42.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "items",
            (0.0, 0.0),
            (-20.0, -10.0, 40.0, 20.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 47 POTTED PLANT
    &[
        layer(
            "pot",
            (0.0, 0.0),
            (-20.0, -20.0, 44.0, 44.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "leaves",
            (0.0, 0.0),
            (-31.0, -31.0, 62.0, 62.0),
            LayerRot::Sway { deg: 3.0, hz: 0.2 },
            After,
        ),
        layer(
            "tag",
            (0.0, 0.0),
            (8.0, 4.0, 12.0, 14.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 48 LOBBY HOLO
    &[
        layer(
            "wash",
            (0.0, 0.0),
            (-50.0, -28.0, 100.0, 72.0),
            LayerRot::None,
            After,
        ),
        layer(
            "slab",
            (0.0, 0.0),
            (-44.0, -44.0, 88.0, 18.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "glyphs",
            (0.0, 0.0),
            (-44.0, -37.0, 88.0, 10.0),
            LayerRot::None,
            After,
        ),
    ],
    // 49 DIRECTORY TOTEM
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-10.0, -22.0, 25.0, 49.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "screen",
            (0.0, 0.0),
            (-8.0, -18.0, 16.0, 30.0),
            LayerRot::None,
            After,
        ),
        layer(
            "wash",
            (0.0, 0.0),
            (-16.0, 20.0, 32.0, 18.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 50 VENDING MACHINE
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-24.0, -20.0, 53.0, 45.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "front",
            (0.0, 0.0),
            (-22.0, 13.0, 44.0, 8.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "wash",
            (0.0, 0.0),
            (-30.0, 20.0, 60.0, 20.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 51 COFFEE CORNER
    &[
        layer(
            "counter",
            (0.0, 0.0),
            (-35.0, -14.0, 74.0, 32.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "machine",
            (0.0, 0.0),
            (-30.0, -12.0, 28.0, 26.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "cups",
            (0.0, 0.0),
            (0.0, -12.0, 34.0, 24.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "steam",
            (-17.0, -12.0),
            (-8.0, -20.0, 16.0, 22.0),
            LayerRot::None,
            After,
        ),
    ],
    // 52 CHARGE LOCKERS
    &[
        layer(
            "cabinet",
            (0.0, 0.0),
            (-40.0, -16.0, 84.0, 36.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "leds",
            (0.0, 0.0),
            (-40.0, 10.0, 80.0, 8.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "cable",
            (0.0, 0.0),
            (10.0, 16.0, 30.0, 22.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 53 FLOOR LOGO
    &[
        layer(
            "inlay",
            (0.0, 0.0),
            (-40.0, -40.0, 80.0, 80.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "mark",
            (0.0, 0.0),
            (-13.0, -13.0, 26.0, 26.0),
            LayerRot::Static(45.0),
            Before,
        ),
        layer(
            "seams",
            (0.0, 0.0),
            (-26.0, -26.0, 52.0, 52.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 54 CALL PANEL
    &[
        layer(
            "wall",
            (0.0, 0.0),
            (-30.0, -46.0, 60.0, 14.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "panel",
            (0.0, 0.0),
            (11.0, -35.0, 16.0, 12.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "arrows",
            (0.0, 0.0),
            (10.0, -34.0, 18.0, 20.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 55 VELVET ROPE
    &[
        layer(
            "shadow",
            (0.0, 0.0),
            (-34.0, -4.0, 80.0, 18.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "rope",
            (0.0, 0.0),
            (-36.0, -3.0, 72.0, 12.0),
            LayerRot::None,
            After,
        ),
        layer(
            "posts",
            (0.0, 0.0),
            (-44.0, -8.0, 92.0, 16.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 56 EXTINGUISHER
    &[
        layer(
            "mount",
            (0.0, 0.0),
            (-16.0, -46.0, 32.0, 14.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "tank",
            (0.0, 0.0),
            (-12.0, -38.0, 28.0, 28.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "sign",
            (0.0, 0.0),
            (-10.0, -6.0, 20.0, 20.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 57 CREDIT KIOSK
    &[
        layer(
            "body",
            (0.0, 0.0),
            (-15.0, -20.0, 35.0, 45.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "screen",
            (0.0, 0.0),
            (-11.0, -16.0, 22.0, 14.0),
            LayerRot::None,
            After,
        ),
        layer(
            "wash",
            (0.0, 0.0),
            (-21.0, 20.0, 42.0, 16.0),
            LayerRot::None,
            Before,
        ),
    ],
    // 58 WALL CLOCK
    &[
        layer(
            "face",
            (0.0, 0.0),
            (-36.0, -36.0, 72.0, 72.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "hour",
            (0.0, 0.0),
            (-3.0, -20.0, 6.0, 24.0),
            LayerRot::Static(-50.0),
            After,
        ),
        layer(
            "minute",
            (0.0, 0.0),
            (-2.0, -28.0, 4.0, 32.0),
            LayerRot::Spin { hz: 1.0 / 40.0 },
            After,
        ),
        layer(
            "second",
            (0.0, 0.0),
            (-1.0, -30.0, 2.0, 36.0),
            LayerRot::Spin { hz: 1.0 / 8.0 },
            After,
        ),
    ],
    // 59 WELCOME MAT
    &[
        layer(
            "mat",
            (0.0, 0.0),
            (-35.0, -20.0, 70.0, 40.0),
            LayerRot::None,
            Before,
        ),
        layer(
            "pattern",
            (0.0, 0.0),
            (-31.0, -16.0, 62.0, 32.0),
            LayerRot::None,
            Before,
        ),
    ],
];

/// The layers of prop `kind`.
pub fn prop_layers(kind: usize) -> &'static [LayerDef] {
    PROP_LAYERS[kind % PROP_COUNT]
}

/// The JSON id of prop `kind`: `PROP_NAMES` lower-cased, every run of
/// non-alphanumerics collapsed to one `_` (`"RACK / CLOSED"` -> `rack_closed`).
pub fn prop_kind_id(kind: usize) -> String {
    let mut out = String::new();
    for ch in PROP_NAMES[kind % PROP_COUNT].chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

// ---- persisted settings (generated: src/props_data.rs) --------------------

/// A layer's saved pixel mode (`props/props.json` -> `PROP_SETTINGS`).
pub struct LayerSetting {
    pub name: &'static str,
    pub pixel: PixelMode,
}

/// A prop's saved settings: its art-pixel size (design units, 1 = off) and
/// the layers whose pixel mode differs from / is pinned against the
/// [`LayerDef`] default. Unknown layer names are ignored, missing ones use
/// the default.
pub struct PropSettings {
    pub kind: &'static str,
    pub px: u32,
    pub layers: &'static [LayerSetting],
}

/// The saved art-pixel size of prop `kind` (1 = off).
pub fn prop_px(kind: usize) -> u32 {
    PROP_SETTINGS[kind % PROP_COUNT].px.clamp(1, MAX_PX)
}

/// The per-layer pixel modes of prop `kind`: the [`LayerDef`] defaults with
/// the saved overrides applied (entries past the prop's layer count are
/// `Before`).
pub fn prop_modes(kind: usize) -> [PixelMode; MAX_LAYERS] {
    let kind = kind % PROP_COUNT;
    let mut modes = [Before; MAX_LAYERS];
    for (i, l) in PROP_LAYERS[kind].iter().enumerate() {
        modes[i] = PROP_SETTINGS[kind]
            .layers
            .iter()
            .find(|s| s.name == l.name)
            .map(|s| s.pixel)
            .unwrap_or(l.pixel);
    }
    modes
}

/// Serialize per-prop `(px, modes)` (one entry per prop, in id order) to the
/// `props/props.json` document (`docs/PROPS_FORMAT.md`), in the checked-in
/// formatting: one prop per line.
pub fn settings_json(props: &[(u32, [PixelMode; MAX_LAYERS])]) -> String {
    let mut out = String::from("{\n  \"props\": [\n");
    for (kind, (px, modes)) in props.iter().enumerate().take(PROP_COUNT) {
        let layers: Vec<String> = PROP_LAYERS[kind]
            .iter()
            .enumerate()
            .map(|(i, l)| {
                format!(
                    "{{\"name\": \"{}\", \"pixel\": \"{}\"}}",
                    l.name,
                    modes[i].id()
                )
            })
            .collect();
        out.push_str(&format!(
            "    {{\"kind\": \"{}\", \"px\": {}, \"layers\": [{}]}}",
            prop_kind_id(kind),
            (*px).clamp(1, MAX_PX),
            layers.join(", ")
        ));
        out.push_str(if kind + 1 < PROP_COUNT.min(props.len()) {
            ",\n"
        } else {
            "\n"
        });
    }
    out.push_str("  ]\n}\n");
    out
}

/// Per-draw options of [`draw_prop_ex`].
#[derive(Clone, Copy)]
pub struct PropDrawOpts {
    /// Bit `i` set = layer `i` is drawn.
    pub visible: u32,
    /// Pixel mode per layer (only used with `px >= 2`).
    pub modes: [PixelMode; MAX_LAYERS],
}

impl PropDrawOpts {
    /// Every layer visible, the saved modes of prop `kind`.
    pub fn saved(kind: usize) -> Self {
        PropDrawOpts {
            visible: u32::MAX,
            modes: prop_modes(kind),
        }
    }
}

/// The on-screen size to draw a prop at so its art pixels map to an INTEGER
/// number of device pixels: the largest `k` art texels -> `k` px that fits in
/// `size_px` (`100 / px` art texels across the prop). NEAREST upscaling then
/// never straddles texel rows unevenly, and every layer's group snaps to the
/// same device grid. `px <= 1`, or a prop smaller than one px per texel,
/// returns `size_px` unchanged.
pub fn snap_size(size_px: f32, px: u32) -> f32 {
    if px <= 1 {
        return size_px;
    }
    let texels = 100.0 / px as f32;
    let k = (size_px / texels).floor();
    if k >= 1.0 {
        texels * k
    } else {
        size_px
    }
}

/// The pixel group box for a layer AABB: origin snapped down to the layer
/// frame's `px` grid and the far edge up, so layers sharing a pivot share one
/// grid (and edges designed on multiples of `px` land on texel edges).
pub fn snap_box(bounds: (f32, f32, f32, f32), px: f32) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = bounds;
    let gx = (x / px).floor() * px;
    let gy = (y / px).floor() * px;
    let gw = ((x + w) / px).ceil() * px - gx;
    let gh = ((y + h) / px).ceil() * px - gy;
    (gx, gy, gw, gh)
}

/// The square group box that holds `bounds` at ANY rotation about the
/// origin (for `After` layers that turn): radius = the farthest corner.
pub fn rot_box(bounds: (f32, f32, f32, f32), px: f32) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = bounds;
    let r = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
        .iter()
        .map(|&(cx, cy)| (cx * cx + cy * cy).sqrt())
        .fold(0.0f32, f32::max);
    snap_box((-r, -r, 2.0 * r, 2.0 * r), px)
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{draw_prop, draw_prop_ex, draw_prop_layer};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{
        gate_angle, prop_px, rot_box, snap_box, turnstile_angle, PixelMode, PropDrawOpts,
        MAX_LAYERS, PROP_COUNT, PROP_LAYERS,
    };
    use crate::graphics::Graphics;
    use crate::math::{Color, Vec2};
    use std::f32::consts::{FRAC_PI_2, PI, TAU};

    // ---- the shared datacenter palette --------------------------------------
    const STEEL_DARK: Color = Color::new(0.15, 0.14, 0.20, 1.0);
    const STEEL: Color = Color::new(0.24, 0.22, 0.30, 1.0);
    const TRIM: Color = Color::new(0.38, 0.34, 0.46, 1.0);
    const PANEL: Color = Color::new(0.08, 0.07, 0.12, 1.0);
    const LED_GREEN: Color = Color::new(0.30, 1.0, 0.55, 1.0);
    const LED_AMBER: Color = Color::new(1.0, 0.72, 0.20, 1.0);
    const LED_RED: Color = Color::new(1.0, 0.22, 0.28, 1.0);
    const GLOW_CYAN: Color = Color::new(0.30, 0.95, 1.0, 1.0);
    const GLOW_MAGENTA: Color = Color::new(0.95, 0.28, 0.78, 1.0);
    const CREAM: Color = Color::new(0.96, 0.93, 0.86, 1.0);
    const HAZARD_YELLOW: Color = Color::new(0.95, 0.78, 0.10, 1.0);
    const COPPER: Color = Color::new(0.62, 0.38, 0.28, 1.0);
    const SHADOW: Color = Color::new(0.0, 0.0, 0.0, 0.30);

    // ---- tiny drawing / animation helpers -----------------------------------
    fn rect(g: &Graphics, x: f32, y: f32, w: f32, h: f32, c: Color) {
        g.draw_rectangle(Vec2::new(x, y), w, h, c);
    }
    fn frame(g: &Graphics, x: f32, y: f32, w: f32, h: f32, t: f32, c: Color) {
        g.draw_rectangle_lines(Vec2::new(x, y), w, h, t, c);
    }
    fn circle(g: &Graphics, x: f32, y: f32, r: f32, c: Color) {
        g.draw_circle(Vec2::new(x, y), r, c);
    }
    fn line(g: &Graphics, x1: f32, y1: f32, x2: f32, y2: f32, th: f32, c: Color) {
        g.draw_line(Vec2::new(x1, y1), Vec2::new(x2, y2), th, c);
    }
    fn alpha(c: Color, a: f32) -> Color {
        Color::new(c.r, c.g, c.b, a)
    }
    /// Drop shadow under a tall box footprint (light from the top-left).
    fn shadow_rect(g: &Graphics, x: f32, y: f32, w: f32, h: f32) {
        rect(g, x + 4.0, y + 4.0, w, h, SHADOW);
    }
    /// Drop shadow under a tall round footprint.
    fn shadow_circle(g: &Graphics, x: f32, y: f32, r: f32) {
        circle(g, x + 4.0, y + 4.0, r, SHADOW);
    }
    /// Small deterministic hash -> 0..1, for per-cell variety that stays put.
    fn rnd(a: u32, b: u32) -> f32 {
        let mut x = a
            .wrapping_mul(374_761_393)
            .wrapping_add(b.wrapping_mul(668_265_263));
        x = (x ^ (x >> 13)).wrapping_mul(1_274_126_177);
        ((x ^ (x >> 16)) & 0xff_ffff) as f32 / 0xff_ffff as f32
    }
    /// Square-wave blink with `duty` fraction on, at `hz`, offset by `phase`.
    fn blink(time: f32, hz: f32, phase: f32, duty: f32) -> bool {
        (time * hz + phase).fract() < duty
    }
    /// A fan set into a top panel, seen from above, in ITS OWN frame (the
    /// layer's rotation spins it): dark well, four blades, hub.
    fn fan(g: &Graphics, r: f32) {
        circle(g, 0.0, 0.0, r, PANEL);
        for k in 0..4 {
            let a = k as f32 * FRAC_PI_2;
            g.draw_arc(Vec2::new(0.0, 0.0), r - 1.5, a, a + 0.62, STEEL_DARK);
        }
        circle(g, 0.0, 0.0, r * 0.24, TRIM);
    }
    /// A row of status LEDs winking on a front-edge service strip.
    fn front_leds(g: &Graphics, x0: f32, y: f32, n: u32, time: f32) {
        for i in 0..n {
            let on = blink(time, 1.2 + rnd(i, 3) * 3.5, rnd(7, i), 0.6);
            let c = if !on {
                PANEL
            } else if rnd(i, 5) > 0.35 {
                LED_GREEN
            } else {
                LED_AMBER
            };
            rect(g, x0 + i as f32 * 8.0, y, 5.0, 4.0, c);
        }
    }

    // ---- the OUTDOOR / LOBBY additions to the palette -----------------------
    const PEARL: Color = Color::new(0.85, 0.86, 0.90, 1.0);
    const TEAL: Color = Color::new(0.14, 0.36, 0.44, 1.0);
    const ORANGE: Color = Color::new(0.90, 0.46, 0.16, 1.0);
    const TYRE: Color = Color::new(0.06, 0.06, 0.08, 1.0);
    const GLASS: Color = Color::new(0.10, 0.14, 0.20, 1.0);
    const GLASS_HI: Color = Color::new(0.45, 0.65, 0.80, 1.0);
    const ASPHALT: Color = Color::new(0.11, 0.11, 0.14, 1.0);
    const PAINT: Color = Color::new(0.90, 0.90, 0.86, 1.0);
    const CONCRETE: Color = Color::new(0.52, 0.50, 0.48, 1.0);
    const CONCRETE_DARK: Color = Color::new(0.36, 0.35, 0.34, 1.0);
    const LEAF: Color = Color::new(0.18, 0.44, 0.24, 1.0);
    const LEAF_LIGHT: Color = Color::new(0.30, 0.60, 0.34, 1.0);
    const LEAF_DARK: Color = Color::new(0.10, 0.28, 0.16, 1.0);
    const NEON_PINK: Color = Color::new(1.0, 0.32, 0.62, 1.0);
    const WARM_LIGHT: Color = Color::new(1.0, 0.85, 0.60, 1.0);
    const MARBLE: Color = Color::new(0.72, 0.68, 0.64, 1.0);
    const MARBLE_DARK: Color = Color::new(0.58, 0.54, 0.52, 1.0);
    const WALNUT: Color = Color::new(0.36, 0.24, 0.16, 1.0);
    const WALNUT_LIGHT: Color = Color::new(0.48, 0.33, 0.22, 1.0);
    const BRASS: Color = Color::new(0.80, 0.64, 0.30, 1.0);
    const VELVET: Color = Color::new(0.62, 0.10, 0.20, 1.0);
    const CHROME: Color = Color::new(0.72, 0.74, 0.80, 1.0);
    const RUBBER: Color = Color::new(0.10, 0.10, 0.12, 1.0);

    // ---- helpers shared by the OUTDOOR / LOBBY families ---------------------
    /// A ring (outline circle) as short chords, so it works over any
    /// background (a filled circle pair would need a known fill).
    fn ring(g: &Graphics, x: f32, y: f32, r: f32, th: f32, c: Color) {
        let n = 20;
        for k in 0..n {
            let a0 = k as f32 / n as f32 * TAU;
            let a1 = (k + 1) as f32 / n as f32 * TAU;
            line(
                g,
                x + a0.cos() * r,
                y + a0.sin() * r,
                x + a1.cos() * r,
                y + a1.sin() * r,
                th,
                c,
            );
        }
    }
    /// The edge-on light wash of a screen / lamp: a beam that starts `w0`
    /// wide at `y0` (centred on `cx`) and spreads as it fades over `len`
    /// (negative `len` = towards -y), in four steps.
    fn wash_v(g: &Graphics, cx: f32, y0: f32, w0: f32, len: f32, c: Color, a0: f32) {
        let steps = 4;
        let step = len / steps as f32;
        let spread = len.abs() * 0.12;
        for i in 0..steps {
            let t = i as f32;
            let y = y0 + t * step;
            let (ya, h) = if step >= 0.0 {
                (y, step)
            } else {
                (y + step, -step)
            };
            rect(
                g,
                cx - w0 / 2.0 - t * spread,
                ya,
                w0 + 2.0 * t * spread,
                h,
                alpha(c, a0 * (1.0 - t / steps as f32)),
            );
        }
    }
    /// The same wash sideways: from `x0`, `h0` tall (centred on `cy`),
    /// spreading over `len` towards +x (negative = -x).
    fn wash_h(g: &Graphics, x0: f32, cy: f32, h0: f32, len: f32, c: Color, a0: f32) {
        let steps = 4;
        let step = len / steps as f32;
        let spread = len.abs() * 0.12;
        for i in 0..steps {
            let t = i as f32;
            let x = x0 + t * step;
            let (xa, w) = if step >= 0.0 {
                (x, step)
            } else {
                (x + step, -step)
            };
            rect(
                g,
                xa,
                cy - h0 / 2.0 - t * spread,
                w,
                h0 + 2.0 * t * spread,
                alpha(c, a0 * (1.0 - t / steps as f32)),
            );
        }
    }
    /// A car seen from above, nose towards +y: drop shadow, the wheel arches
    /// peeking out at the sides, the shell with rounded ends, the roof panel.
    fn car_shell(g: &Graphics, x: f32, y: f32, w: f32, h: f32, body: Color, roof: Color) {
        shadow_rect(g, x, y, w, h);
        for &(ax, ay) in &[
            (x - 2.0, y + 8.0),
            (x + w - 1.0, y + 8.0),
            (x - 2.0, y + h - 18.0),
            (x + w - 1.0, y + h - 18.0),
        ] {
            rect(g, ax, ay, 3.0, 10.0, TYRE);
        }
        rect(g, x, y + 4.0, w, h - 8.0, body);
        rect(g, x + 4.0, y, w - 8.0, 4.0, body);
        rect(g, x + 4.0, y + h - 4.0, w - 8.0, 4.0, body);
        // Panel seams: hood / trunk lines and the roof panel.
        rect(g, x + 4.0, y + h * 0.30, w - 8.0, h * 0.42, roof);
        line(
            g,
            x + 3.0,
            y + 8.0,
            x + w - 3.0,
            y + 8.0,
            1.0,
            alpha(PANEL, 0.35),
        );
        line(
            g,
            x + 3.0,
            y + h - 8.0,
            x + w - 3.0,
            y + h - 8.0,
            1.0,
            alpha(PANEL, 0.35),
        );
    }
    /// A hazard LED pair blinking together on a car's four corners.
    fn hazards(g: &Graphics, x: f32, y: f32, w: f32, h: f32, time: f32) {
        if blink(time, 1.0, 0.0, 0.5) {
            for &(cx, cy) in &[
                (x + 1.0, y + 1.0),
                (x + w - 5.0, y + 1.0),
                (x + 1.0, y + h - 4.0),
                (x + w - 5.0, y + h - 4.0),
            ] {
                rect(g, cx, cy, 4.0, 3.0, LED_AMBER);
                circle(g, cx + 2.0, cy + 1.5, 4.0, alpha(LED_AMBER, 0.18));
            }
        }
    }
    /// A roof sensor puck seen from above, in ITS OWN frame: the layer spin
    /// turns the lidar sweep.
    fn lidar(g: &Graphics, r: f32) {
        circle(g, 0.0, 0.0, r, STEEL_DARK);
        circle(g, 0.0, 0.0, r - 1.5, PANEL);
        g.draw_arc(
            Vec2::new(0.0, 0.0),
            r - 1.0,
            -0.35,
            0.35,
            alpha(GLOW_CYAN, 0.55),
        );
        line(g, 0.0, 0.0, r - 1.0, 0.0, 1.2, GLOW_CYAN);
        circle(g, 0.0, 0.0, 1.5, TRIM);
    }
    /// A tall post seen from above (bollard, rope post): shadow, base plate,
    /// cap.
    fn post(g: &Graphics, x: f32, y: f32, r: f32, base: Color, cap: Color) {
        shadow_circle(g, x, y, r + 2.0);
        circle(g, x, y, r + 2.0, base);
        circle(g, x, y, r, cap);
        circle(g, x - r * 0.3, y - r * 0.3, r * 0.3, alpha(CREAM, 0.35));
    }
    /// A block of the neon glyph strip on a holo sign: pixel blocks
    /// scrolling along the front face between `x0` and `x1` at row `y`.
    fn glyph_strip(g: &Graphics, x0: f32, x1: f32, y: f32, time: f32, c: Color) {
        let span = x1 - x0;
        for k in 0..9u32 {
            let x = x0 + (time * 14.0 + k as f32 * 13.0).rem_euclid(span);
            let w = (3.0 + rnd(k, 2) * 4.0).min(x1 - x);
            let h = 2.0 + rnd(k, 4) * 3.0;
            let col = if rnd(k, 6) > 0.7 { CREAM } else { c };
            rect(g, x, y + 5.0 - h, w, h, alpha(col, 0.85));
        }
    }
    /// A chevron (arrow head) pointing towards -y, centred on (x, y).
    fn chevron(g: &Graphics, x: f32, y: f32, s: f32, th: f32, c: Color) {
        line(g, x - s, y + s * 0.6, x, y - s * 0.6, th, c);
        line(g, x, y - s * 0.6, x + s, y + s * 0.6, th, c);
    }
    /// A round chair from above, seat facing +y (backrest at -y).
    fn chair(g: &Graphics, x: f32, y: f32) {
        shadow_circle(g, x, y, 11.0);
        g.draw_arc(Vec2::new(x, y), 12.0, PI + 0.35, TAU - 0.35, STEEL_DARK);
        circle(g, x, y, 9.5, Color::new(0.30, 0.26, 0.36, 1.0));
        circle(g, x, y, 4.0, STEEL_DARK);
    }

    /// Draw prop `idx` (see [`super::PROP_NAMES`]) centred on `center` at
    /// `size_px` px, animated by the continuous clock `time` (seconds), with
    /// its SAVED art-pixel size and layer modes (`props/props.json`).
    pub fn draw_prop(g: &Graphics, idx: usize, center: Vec2, size_px: f32, time: f32) {
        let idx = idx % PROP_COUNT;
        draw_prop_ex(
            g,
            idx,
            center,
            size_px,
            time,
            prop_px(idx),
            &PropDrawOpts::saved(idx),
        );
    }

    /// Draw prop `idx` layer by layer: each visible layer in its own frame
    /// (pivot, rotation from its [`super::LayerRot`] at `time`), and — when
    /// `px >= 2` (design units: an art pixel is `px / 100` of the prop's
    /// box) — inside its own pixel-art group, rasterized BEFORE or AFTER its
    /// rotation per `opts.modes` (see the module docs). `px <= 1` draws the
    /// layers directly. This is the call a floor renderer makes.
    pub fn draw_prop_ex(
        g: &Graphics,
        idx: usize,
        center: Vec2,
        size_px: f32,
        time: f32,
        px: u32,
        opts: &PropDrawOpts,
    ) {
        let idx = idx % PROP_COUNT;
        g.save();
        g.translate(center.x, center.y);
        let s = size_px / 100.0;
        g.scale(s, s);
        for (li, l) in PROP_LAYERS[idx].iter().enumerate().take(MAX_LAYERS) {
            if opts.visible & (1 << li) == 0 {
                continue;
            }
            let angle = l.rot.angle(time);
            g.save();
            g.translate(l.pivot.0, l.pivot.1);
            if px <= 1 {
                if angle != 0.0 {
                    g.rotate(angle);
                }
                draw_prop_layer(g, idx, li, time);
            } else {
                let pxf = px as f32;
                match opts.modes[li] {
                    PixelMode::Before => {
                        // Rotate, THEN open the group: the layer is rasterized
                        // unrotated on its own grid (the group resets the
                        // transform) and PIX_END's quad — drawn through the
                        // rotated outer transform — turns the pixel image.
                        if angle != 0.0 {
                            g.rotate(angle);
                        }
                        let (gx, gy, gw, gh) = snap_box(l.bounds, pxf);
                        g.pixel_begin(pxf, gw, gh);
                        g.translate(-gx, -gy);
                        draw_prop_layer(g, idx, li, time);
                        g.pixel_end(gx, gy);
                    }
                    PixelMode::After => {
                        // Open the group in the parent's frame (sized to hold
                        // the layer at any angle if it turns), rotate INSIDE
                        // it: re-rasterized on the parent's grid every frame.
                        let (gx, gy, gw, gh) = if l.rot.is_none() {
                            snap_box(l.bounds, pxf)
                        } else {
                            rot_box(l.bounds, pxf)
                        };
                        g.pixel_begin(pxf, gw, gh);
                        g.translate(-gx, -gy);
                        if angle != 0.0 {
                            g.rotate(angle);
                        }
                        draw_prop_layer(g, idx, li, time);
                        g.pixel_end(gx, gy);
                    }
                }
            }
            g.restore();
        }
        g.restore();
    }

    /// Draw layer `layer` of prop `idx` in the layer's own frame: origin at
    /// its pivot, unrotated (the caller applies the [`super::LayerRot`]).
    pub fn draw_prop_layer(g: &Graphics, idx: usize, layer: usize, time: f32) {
        match idx % PROP_COUNT {
            0 => rack_closed(g, layer, time),
            1 => rack_open(g, layer, time),
            2 => rack_burnt(g, layer, time),
            3 => blade_stack(g, layer, time),
            4 => core_switch(g, layer, time),
            5 => cable_junction(g, layer, time),
            6 => operator_desk(g, layer, time),
            7 => control_console(g, layer, time),
            8 => holo_table(g, layer, time),
            9 => crac_cooler(g, layer, time),
            10 => floor_vent(g, layer, time),
            11 => exhaust_fan(g, layer, time),
            12 => coolant_tank(g, layer, time),
            13 => pipe_run(g, layer, time),
            14 => ups_cabinet(g, layer, time),
            15 => generator(g, layer, time),
            16 => cable_tray(g, layer, time),
            17 => cable_coil(g, layer, time),
            18 => tape_library(g, layer, time),
            19 => supply_crate(g, layer, time),
            20 => security_cam(g, layer, time),
            21 => fire_suppressor(g, layer, time),
            22 => hazard_pad(g, layer, time),
            23 => uplink_obelisk(g, layer, time),
            24 => car_pod(g, layer, time),
            25 => car_sedan(g, layer, time),
            26 => car_open(g, layer, time),
            27 => delivery_van(g, layer, time),
            28 => charge_pad(g, layer, time),
            29 => car_charging(g, layer, time),
            30 => main_gate(g, layer, time),
            31 => guard_booth(g, layer, time),
            32 => bollards(g, layer, time),
            33 => planter(g, layer, time),
            34 => lamp_post(g, layer, time),
            35 => ev_bay(g, layer, time),
            36 => crosswalk(g, layer, time),
            37 => drone_pad(g, layer, time),
            38 => scooter_rack(g, layer, time),
            39 => drain_grate(g, layer, time),
            40 => holo_billboard(g, layer, time),
            41 => dumpster(g, layer, time),
            42 => reception_desk(g, layer, time),
            43 => turnstiles(g, layer, time),
            44 => scanner_arch(g, layer, time),
            45 => bench_long(g, layer, time),
            46 => bench_short(g, layer, time),
            47 => potted_plant(g, layer, time),
            48 => lobby_holo(g, layer, time),
            49 => directory_totem(g, layer, time),
            50 => vending_machine(g, layer, time),
            51 => coffee_corner(g, layer, time),
            52 => charge_lockers(g, layer, time),
            53 => floor_logo(g, layer, time),
            54 => call_panel(g, layer, time),
            55 => velvet_rope(g, layer, time),
            56 => extinguisher(g, layer, time),
            57 => credit_kiosk(g, layer, time),
            58 => wall_clock(g, layer, time),
            _ => welcome_mat(g, layer, time),
        }
    }

    /// 0 — closed rack seen from above: sealed top panel, twin exhaust fans,
    /// cabling ducking out the back, status LEDs on the front edge.
    /// Layers: body, fan a, fan b (spinning), leds.
    fn rack_closed(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -35.0, -45.0, 70.0, 90.0);
                rect(g, -35.0, -45.0, 70.0, 90.0, STEEL);
                frame(g, -35.0, -45.0, 70.0, 90.0, 2.0, TRIM);
                // Lid seam, then the rear cable cutout with feeds heading off
                // to the trunking behind.
                frame(g, -29.0, -37.0, 58.0, 72.0, 1.0, alpha(PANEL, 0.6));
                rect(g, -14.0, -45.0, 28.0, 7.0, PANEL);
                line(g, -8.0, -42.0, -12.0, -49.0, 2.0, COPPER);
                line(g, 3.0, -42.0, 6.0, -49.0, 2.0, GLOW_CYAN);
            }
            // Twin roof fans, counter-rotating (the layer rotation spins them).
            1 | 2 => fan(g, 13.0),
            _ => {
                // Front service strip.
                rect(g, -35.0, 37.0, 70.0, 8.0, STEEL_DARK);
                front_leds(g, -30.0, 39.0, 5, time);
            }
        }
    }

    /// 1 — rack with the lid off, looking straight down into the chassis:
    /// board, chip grid, finned heatsink, loose cabling, a live internal fan.
    /// Layers: chassis, fan (spinning), leds.
    fn rack_open(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -35.0, -45.0, 70.0, 90.0);
                rect(g, -35.0, -45.0, 70.0, 90.0, STEEL_DARK); // chassis walls
                frame(g, -35.0, -45.0, 70.0, 90.0, 2.0, TRIM);
                rect(g, -30.0, -40.0, 60.0, 80.0, PANEL); // the open bay
                                                          // Main board and its chip grid.
                rect(
                    g,
                    -27.0,
                    -37.0,
                    42.0,
                    74.0,
                    Color::new(0.07, 0.17, 0.11, 1.0),
                );
                for row in 0..4u32 {
                    for col in 0..3u32 {
                        if rnd(row * 5 + col, 3) > 0.75 {
                            continue; // an unpopulated pad
                        }
                        rect(
                            g,
                            -23.0 + col as f32 * 13.0,
                            -33.0 + row as f32 * 12.0,
                            8.0,
                            6.0,
                            Color::new(0.11, 0.24, 0.16, 1.0),
                        );
                    }
                }
                // CPU under a finned heatsink.
                rect(g, -21.0, 15.0, 24.0, 20.0, STEEL);
                for i in 0..5 {
                    rect(g, -19.5 + i as f32 * 4.6, 16.0, 2.0, 18.0, STEEL_DARK);
                }
                // Loose cabling along the right wall.
                line(g, 20.0, -36.0, 25.0, -10.0, 2.0, COPPER);
                line(g, 25.0, -10.0, 19.0, 22.0, 2.0, COPPER);
                line(g, 23.0, -36.0, 20.0, 8.0, 1.5, GLOW_CYAN);
            }
            // The internal fan, still running with the lid off.
            1 => fan(g, 10.0),
            _ => {
                // Board LEDs.
                for i in 0..3u32 {
                    let on = blink(time, 2.0 + rnd(i, 9) * 4.0, rnd(i, 4), 0.5);
                    rect(
                        g,
                        -26.0,
                        24.0 + i as f32 * 5.0,
                        3.0,
                        3.0,
                        if on { LED_GREEN } else { PANEL },
                    );
                }
            }
        }
    }

    /// 2 — burnt-out rack from above: charred top blown open, an ember still
    /// cooling in the hole, the lid flat on the floor beside it, stray sparks.
    /// Layers: lid (static tilt), hull, sparks.
    fn rack_burnt(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                // The blown-off lid lies on the floor to the right (flat: no
                // shadow); its tilt is the layer's static rotation.
                rect(
                    g,
                    -8.0,
                    -27.0,
                    16.0,
                    54.0,
                    Color::new(0.11, 0.09, 0.13, 1.0),
                );
                frame(
                    g,
                    -8.0,
                    -27.0,
                    16.0,
                    54.0,
                    1.5,
                    Color::new(0.20, 0.17, 0.22, 1.0),
                );
            }
            1 => {
                shadow_rect(g, -40.0, -45.0, 62.0, 90.0);
                rect(
                    g,
                    -40.0,
                    -45.0,
                    62.0,
                    90.0,
                    Color::new(0.09, 0.08, 0.11, 1.0),
                );
                frame(
                    g,
                    -40.0,
                    -45.0,
                    62.0,
                    90.0,
                    2.0,
                    Color::new(0.20, 0.17, 0.22, 1.0),
                );
                // The blast hole through the top, soot streaked outward.
                for k in 0..4u32 {
                    let a = rnd(k, 1) * TAU;
                    line(
                        g,
                        -9.0 + a.cos() * 12.0,
                        -6.0 + a.sin() * 12.0,
                        -9.0 + a.cos() * 26.0,
                        -6.0 + a.sin() * 26.0,
                        4.0,
                        Color::new(0.04, 0.03, 0.05, 0.8),
                    );
                }
                circle(g, -9.0, -6.0, 16.0, Color::new(0.03, 0.02, 0.04, 1.0));
                circle(g, 0.0, 6.0, 9.0, Color::new(0.03, 0.02, 0.04, 1.0));
                let ember = 0.25 + 0.20 * (time * 3.3).sin();
                circle(g, -9.0, -5.0, 6.0, Color::new(0.9, 0.25, 0.08, ember));
            }
            _ => {
                // Something in there still shorts now and then.
                for k in 0..3u32 {
                    if ((time * (11.0 + k as f32 * 3.7) + k as f32 * 1.9).sin()) > 0.94 {
                        let x = -24.0 + rnd(k, 5) * 30.0;
                        let y = -28.0 + rnd(k, 9) * 44.0;
                        rect(g, x, y, 3.0, 3.0, Color::new(1.0, 0.92, 0.4, 0.95));
                        line(g, x + 1.0, y + 1.0, x + 5.0, y - 4.0, 1.0, LED_AMBER);
                    }
                }
            }
        }
    }

    /// 3 — open blade enclosure from above: vertical blade fins with the hot
    /// exhaust glow breathing in the gaps, service strip along the front.
    /// Layers: enclosure, leds.
    fn blade_stack(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -40.0, -42.0, 80.0, 84.0);
                rect(g, -40.0, -42.0, 80.0, 84.0, STEEL_DARK);
                frame(g, -40.0, -42.0, 80.0, 84.0, 2.0, TRIM);
                for i in 0..8u32 {
                    let x = -34.0 + i as f32 * 9.0;
                    // The hot aisle glow in the gap before each fin.
                    let pulse = 0.22 + 0.18 * (time * 2.2 + i as f32 * 0.7).sin();
                    rect(g, x - 2.5, -34.0, 2.5, 66.0, alpha(GLOW_CYAN, pulse));
                    let fin = if i % 2 == 0 {
                        STEEL
                    } else {
                        Color::new(0.28, 0.26, 0.35, 1.0)
                    };
                    rect(g, x, -36.0, 6.0, 70.0, fin);
                }
            }
            _ => {
                rect(g, -40.0, 34.0, 80.0, 8.0, PANEL);
                front_leds(g, -34.0, 36.0, 8, time);
            }
        }
    }

    /// 4 — core switch from above: low vented top, uplinks breathing, the
    /// port field blinking along the front edge, cables snaking off across
    /// the floor. Layers: chassis, ports, cables.
    fn core_switch(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -45.0, -22.0, 90.0, 40.0);
                rect(g, -45.0, -22.0, 90.0, 40.0, STEEL);
                frame(g, -45.0, -22.0, 90.0, 40.0, 2.0, TRIM);
                for i in 0..3 {
                    rect(g, -38.0, -16.0 + i as f32 * 7.0, 60.0, 2.5, PANEL);
                }
                // The uplink pair, breathing cyan on the top surface.
                let pulse = 0.5 + 0.5 * (time * 3.0).sin();
                rect(
                    g,
                    28.0,
                    -15.0,
                    9.0,
                    9.0,
                    alpha(GLOW_CYAN, 0.35 + 0.6 * pulse),
                );
                rect(
                    g,
                    28.0,
                    -2.0,
                    9.0,
                    9.0,
                    alpha(GLOW_CYAN, 0.95 - 0.6 * pulse),
                );
            }
            1 => {
                // Front edge: the port field, traffic winking at the cable heads.
                rect(g, -45.0, 12.0, 90.0, 6.0, STEEL_DARK);
                let tick = (time * 6.0) as u32;
                for i in 0..10u32 {
                    let x = -41.0 + i as f32 * 8.4;
                    let r = rnd(i, tick);
                    let c = if r > 0.6 {
                        LED_GREEN
                    } else if r > 0.45 {
                        LED_AMBER
                    } else {
                        PANEL
                    };
                    rect(g, x, 13.5, 4.0, 3.0, c);
                }
            }
            _ => {
                // Patched cables dropping off the front edge onto the floor.
                let cols = [COPPER, GLOW_CYAN, GLOW_MAGENTA, LED_GREEN];
                for (k, &c) in cols.iter().enumerate() {
                    let x = -34.0 + k as f32 * 21.0;
                    let sway = (time * 0.8 + k as f32).sin() * 2.0;
                    line(g, x, 18.0, x + 4.0 + sway, 30.0, 2.0, c);
                    line(g, x + 4.0 + sway, 30.0, x - 2.0 + sway, 42.0, 2.0, c);
                }
            }
        }
    }

    /// 5 — cable junction: colour-coded runs crossing the floor between two
    /// pull boxes. Layers: boxes, cables.
    fn cable_junction(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                for &x in &[-45.0f32, 27.0] {
                    shadow_rect(g, x, -30.0, 18.0, 60.0);
                    rect(g, x, -30.0, 18.0, 60.0, STEEL);
                    frame(g, x, -30.0, 18.0, 60.0, 2.0, TRIM);
                    for i in 0..6 {
                        circle(g, x + 9.0, -24.0 + i as f32 * 9.5, 2.5, PANEL);
                    }
                }
            }
            _ => {
                let cable_colors = [COPPER, GLOW_CYAN, GLOW_MAGENTA, LED_GREEN, LED_AMBER, TRIM];
                // A fixed shuffle: left gland k runs to right gland (k * 5 + 2) % 6.
                for (k, &c) in cable_colors.iter().enumerate() {
                    let ly = -24.0 + k as f32 * 9.5;
                    let ry = -24.0 + ((k * 5 + 2) % 6) as f32 * 9.5;
                    let slack = 2.0 * ((time * 0.5 + k as f32).sin()); // loose runs shift
                    let mid = (ly + ry) / 2.0 + slack;
                    line(g, -27.0, ly, 0.0, mid, 2.0, c);
                    line(g, 0.0, mid, 27.0, ry, 2.0, c);
                }
            }
        }
    }

    /// 6 — operator desk from above: an edge-on monitor slab washing terminal
    /// light across the desk, keyboard, mouse, paperwork, the mug that never
    /// gets finished. Layers: desk, paper a, paper b (static tilts), mug.
    fn operator_desk(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -45.0, -28.0, 90.0, 52.0);
                rect(
                    g,
                    -45.0,
                    -28.0,
                    90.0,
                    52.0,
                    Color::new(0.28, 0.20, 0.24, 1.0),
                );
                frame(
                    g,
                    -45.0,
                    -28.0,
                    90.0,
                    52.0,
                    2.0,
                    Color::new(0.40, 0.30, 0.34, 1.0),
                );
                // Screen light spilling toward the operator: a beam that
                // starts at the screen's own width and spreads as it fades.
                let flick = 0.9 + 0.1 * (time * 13.0).sin();
                for i in 0..3 {
                    let t = i as f32;
                    rect(
                        g,
                        -16.0 - t * 3.0,
                        -14.0 + t * 6.0,
                        32.0 + t * 6.0,
                        6.0,
                        Color::new(0.25, 0.85, 0.45, (0.20 - t * 0.06) * flick),
                    );
                }
                rect(g, -5.0, -26.0, 10.0, 5.0, STEEL_DARK); // stand foot behind
                rect(g, -17.0, -22.0, 34.0, 7.0, PANEL); // the monitor slab
                rect(
                    g,
                    -17.0,
                    -15.5,
                    34.0,
                    2.0,
                    Color::new(0.35, 0.95, 0.55, 0.8 * flick),
                );
                // Keyboard + mouse.
                rect(g, -18.0, 2.0, 30.0, 12.0, STEEL_DARK);
                for r in 0..2 {
                    for i in 0..7 {
                        rect(
                            g,
                            -16.0 + i as f32 * 4.0,
                            4.0 + r as f32 * 5.0,
                            2.5,
                            3.0,
                            TRIM,
                        );
                    }
                }
                circle(g, 21.0, 8.0, 3.0, TRIM);
            }
            // Paperwork drift on the left (each sheet a tilted layer).
            1 => rect(g, -8.0, -10.0, 16.0, 20.0, alpha(CREAM, 0.8)),
            2 => {
                rect(g, -8.0, -10.0, 16.0, 20.0, alpha(CREAM, 0.9));
                for i in 0..4 {
                    let y = -6.0 + i as f32 * 4.0;
                    line(g, -5.0, y, 5.0, y, 1.0, alpha(PANEL, 0.5));
                }
            }
            _ => {
                // The mug, seen from above, handle out.
                circle(g, 36.0, 14.0, 5.5, Color::new(0.85, 0.47, 0.34, 1.0));
                circle(g, 42.0, 14.0, 2.0, Color::new(0.85, 0.47, 0.34, 1.0));
                circle(g, 36.0, 14.0, 3.5, Color::new(0.16, 0.09, 0.07, 1.0));
            }
        }
    }

    /// 7 — control console from above: three angled screen slabs washing
    /// green / amber / static across a winged desk, the chair pushed back.
    /// Layers: shadow, wing l, wing r (static angles), desk, screen l / c / r
    /// (static angles), chair.
    fn control_console(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => shadow_rect(g, -34.0, -34.0, 68.0, 26.0),
            // The desk wings (angled by the layer rotation).
            1 | 2 => rect(
                g,
                -16.0,
                -12.0,
                32.0,
                24.0,
                Color::new(0.22, 0.19, 0.27, 1.0),
            ),
            3 => rect(
                g,
                -34.0,
                -34.0,
                68.0,
                26.0,
                Color::new(0.24, 0.21, 0.29, 1.0),
            ),
            4..=6 => {
                // The three feeds; the right one has dropped to flickering static.
                let k = layer - 4;
                let c = match k {
                    0 => Color::new(0.30, 0.90, 0.50, 1.0),
                    1 => Color::new(1.0, 0.72, 0.20, 1.0),
                    _ => Color::new(0.30, 0.95, 1.0, 1.0),
                };
                let fl = if k == 2 {
                    0.4 + 0.6 * rnd(3, (time * 14.0) as u32)
                } else {
                    0.9 + 0.1 * (time * (9.0 + k as f32 * 2.0)).sin()
                };
                // The wash: the screen's width at the slab, spreading out.
                for i in 0..3 {
                    let t = i as f32;
                    rect(
                        g,
                        -11.0 - t * 2.5,
                        4.0 + t * 6.0,
                        22.0 + t * 5.0,
                        6.0,
                        alpha(c, (0.18 - t * 0.05) * fl),
                    );
                }
                rect(g, -12.0, -3.0, 24.0, 6.0, PANEL);
                rect(g, -12.0, 3.0, 24.0, 1.8, alpha(c, 0.85 * fl));
            }
            _ => {
                // The chair, drifting as if someone just left it.
                let j = (time * 0.4).sin() * 1.5;
                shadow_circle(g, j, 26.0, 11.0);
                g.draw_arc(Vec2::new(j, 26.0), 13.5, 0.35, PI - 0.35, STEEL_DARK); // backrest
                circle(g, j, 26.0, 9.5, Color::new(0.30, 0.26, 0.36, 1.0));
                circle(g, j, 26.0, 4.0, STEEL_DARK);
            }
        }
    }

    /// 8 — holo table: a spinning wireframe projection over a round pedestal.
    /// Layers: pedestal, hologram (self-animated: an elliptical orbit, so it
    /// is not a rigid layer rotation).
    fn holo_table(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_circle(g, 0.0, 14.0, 27.0);
                circle(g, 0.0, 14.0, 27.0, TRIM);
                circle(g, 0.0, 14.0, 23.0, STEEL_DARK);
                circle(
                    g,
                    0.0,
                    14.0,
                    6.0,
                    alpha(GLOW_CYAN, 0.5 + 0.3 * (time * 2.0).sin()),
                );
            }
            _ => {
                // The projected wireframe: a triangle spinning above the emitter.
                let mut pts = [(0.0f32, 0.0f32); 3];
                for (k, p) in pts.iter_mut().enumerate() {
                    let a = time * 0.9 + k as f32 * (TAU / 3.0);
                    *p = (a.cos() * 26.0, a.sin() * 10.0 - 18.0);
                }
                let flick = 0.55 + 0.25 * (time * 17.0).sin();
                for k in 0..3 {
                    let (x1, y1) = pts[k];
                    let (x2, y2) = pts[(k + 1) % 3];
                    line(g, x1, y1, x2, y2, 1.5, alpha(GLOW_CYAN, flick));
                    // Projection beams back down to the emitter.
                    line(g, x1, y1, 0.0, 12.0, 1.0, alpha(GLOW_CYAN, 0.18));
                }
            }
        }
    }

    /// 9 — CRAC cooling unit from above: big housing, top vents, the main
    /// blower set into the roof, a heartbeat status LED on the front edge.
    /// Layers: housing, blower (spinning), led.
    fn crac_cooler(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -40.0, -45.0, 80.0, 90.0);
                rect(g, -40.0, -45.0, 80.0, 90.0, STEEL);
                frame(g, -40.0, -45.0, 80.0, 90.0, 2.0, TRIM);
                for i in 0..3 {
                    rect(g, -32.0, -39.0 + i as f32 * 6.0, 64.0, 2.5, PANEL);
                }
            }
            1 => {
                circle(g, 0.0, 0.0, 26.0, PANEL);
                for k in 0..4 {
                    let a = k as f32 * FRAC_PI_2;
                    g.draw_arc(Vec2::new(0.0, 0.0), 23.0, a, a + 0.62, STEEL_DARK);
                    g.draw_arc(Vec2::new(0.0, 0.0), 23.0, a + 0.1, a + 0.5, TRIM);
                }
                circle(g, 0.0, 0.0, 5.0, TRIM);
            }
            _ => {
                rect(g, -40.0, 39.0, 80.0, 6.0, STEEL_DARK);
                let ok = blink(time, 1.2, 0.0, 0.9);
                rect(
                    g,
                    28.0,
                    40.0,
                    6.0,
                    4.0,
                    if ok { LED_GREEN } else { LED_RED },
                );
            }
        }
    }

    /// 10 — raised-floor vent tile with a faint airflow shimmer.
    /// Layers: grille, airflow.
    fn floor_vent(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(
                    g,
                    -42.0,
                    -42.0,
                    84.0,
                    84.0,
                    Color::new(0.13, 0.12, 0.17, 1.0),
                );
                frame(g, -42.0, -42.0, 84.0, 84.0, 2.0, TRIM);
                for i in 0..7 {
                    let o = -36.0 + i as f32 * 12.0;
                    rect(g, o, -36.0, 3.0, 72.0, PANEL);
                    rect(g, -36.0, o, 72.0, 3.0, PANEL);
                }
            }
            _ => {
                // Cold air rippling out of the grille.
                for k in 0..2u32 {
                    let ph = (time * 0.5 + k as f32 * 0.5).fract();
                    let r = 8.0 + ph * 30.0;
                    circle(g, 0.0, 0.0, r, Color::new(0.5, 0.8, 1.0, 0.10 * (1.0 - ph)));
                }
            }
        }
    }

    /// 11 — floor exhaust duct: five blades spinning under a safety cross.
    /// Layers: duct, blades (spinning), guard.
    fn exhaust_fan(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                rect(g, -42.0, -42.0, 84.0, 84.0, STEEL_DARK);
                frame(g, -42.0, -42.0, 84.0, 84.0, 2.0, TRIM);
                circle(g, 0.0, 0.0, 36.0, PANEL);
            }
            1 => {
                for k in 0..5 {
                    let a = k as f32 * (TAU / 5.0);
                    g.draw_arc(Vec2::new(0.0, 0.0), 32.0, a, a + 0.7, STEEL);
                }
                circle(g, 0.0, 0.0, 8.0, TRIM);
            }
            _ => {
                line(g, -36.0, 0.0, 36.0, 0.0, 3.0, alpha(TRIM, 0.85));
                line(g, 0.0, -36.0, 0.0, 36.0, 3.0, alpha(TRIM, 0.85));
            }
        }
    }

    /// 12 — coolant tank seen from above: liquid, rising bubbles, bolted hatch.
    /// Layers: tank, bubbles, hatch.
    fn coolant_tank(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_circle(g, 0.0, 0.0, 38.0);
                circle(g, 0.0, 0.0, 38.0, TRIM);
                circle(g, 0.0, 0.0, 34.0, Color::new(0.08, 0.20, 0.26, 1.0));
                circle(g, 0.0, 0.0, 30.0, Color::new(0.10, 0.42, 0.52, 0.85));
            }
            1 => {
                for k in 0..5u32 {
                    let ph = (time * 0.28 + k as f32 * 0.37).fract();
                    let x = (k as f32 * 2.4).sin() * 17.0 * (1.0 - ph * 0.4);
                    let y = 20.0 - ph * 40.0;
                    let r = 1.5 + rnd(k, 2) * 2.5;
                    circle(g, x, y, r, Color::new(0.6, 0.9, 1.0, 0.5 * (1.0 - ph)));
                }
            }
            _ => {
                circle(g, 0.0, 0.0, 9.0, STEEL);
                for k in 0..4 {
                    let a = k as f32 * FRAC_PI_2 + 0.4;
                    circle(g, a.cos() * 6.0, a.sin() * 6.0, 1.5, PANEL);
                }
            }
        }
    }

    /// 13 — overhead pipe run seen from below the camera: two runs casting
    /// floor shadows, flanges, a red valve wheel creeping.
    /// Layers: pipes, valve (swaying).
    fn pipe_run(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                // The pipes hang overhead, so their shadows fall well below them.
                rect(g, -50.0, -13.0, 100.0, 13.0, alpha(SHADOW, 0.20));
                rect(g, -50.0, 14.0, 100.0, 13.0, alpha(SHADOW, 0.20));
                rect(
                    g,
                    -50.0,
                    -20.0,
                    100.0,
                    13.0,
                    Color::new(0.30, 0.26, 0.34, 1.0),
                );
                rect(g, -50.0, -19.0, 100.0, 3.0, alpha(CREAM, 0.25)); // glint
                rect(g, -50.0, 7.0, 100.0, 13.0, COPPER);
                rect(g, -50.0, 8.0, 100.0, 3.0, alpha(CREAM, 0.2));
                for &x in &[-30.0, 10.0] {
                    rect(g, x, -22.0, 6.0, 17.0, TRIM);
                }
                for &x in &[-12.0, 34.0] {
                    rect(g, x, 5.0, 6.0, 17.0, TRIM);
                }
            }
            _ => {
                // The valve wheel creeps as pressure is trimmed (the sway).
                circle(g, 0.0, 0.0, 9.5, Color::new(0.72, 0.16, 0.20, 1.0));
                circle(g, 0.0, 0.0, 3.0, STEEL_DARK);
                for k in 0..3 {
                    let a = k as f32 * (PI / 3.0);
                    line(
                        g,
                        -a.cos() * 9.0,
                        -a.sin() * 9.0,
                        a.cos() * 9.0,
                        a.sin() * 9.0,
                        2.0,
                        Color::new(0.5, 0.10, 0.14, 1.0),
                    );
                }
            }
        }
    }

    /// 14 — UPS cabinet from above: hazard strip and cable glands at the
    /// back, vented lid with the bolt painted on, charge LEDs up front.
    /// Layers: cabinet, bolt, charge.
    fn ups_cabinet(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -30.0, -45.0, 60.0, 90.0);
                rect(g, -30.0, -45.0, 60.0, 90.0, STEEL);
                frame(g, -30.0, -45.0, 60.0, 90.0, 2.0, TRIM);
                // Rear cable glands, feeds ducking out the back.
                for k in 0..2u32 {
                    let x = -12.0 + k as f32 * 24.0;
                    line(
                        g,
                        x,
                        -40.0,
                        x + (k as f32 - 0.5) * 10.0,
                        -49.0,
                        3.0,
                        if k == 0 { COPPER } else { STEEL_DARK },
                    );
                    circle(g, x, -38.0, 4.5, STEEL_DARK);
                    circle(g, x, -38.0, 2.0, PANEL);
                }
                // Hazard strip across the lid.
                for i in 0..7 {
                    let c = if i % 2 == 0 {
                        HAZARD_YELLOW
                    } else {
                        Color::new(0.05, 0.05, 0.06, 1.0)
                    };
                    rect(g, -28.0 + i as f32 * 8.0, -28.0, 8.0, 6.0, c);
                }
                // Vent grille.
                for i in 0..4 {
                    rect(g, -22.0, -16.0 + i as f32 * 7.0, 44.0, 2.5, PANEL);
                }
            }
            1 => {
                // The bolt, painted on the lid.
                let on = blink(time, 1.0, 0.25, 0.7);
                let c = if on {
                    HAZARD_YELLOW
                } else {
                    alpha(HAZARD_YELLOW, 0.35)
                };
                line(g, 4.0, 16.0, -2.0, 24.0, 2.5, c);
                line(g, -2.0, 24.0, 3.0, 24.0, 2.5, c);
                line(g, 3.0, 24.0, -4.0, 32.0, 2.5, c);
            }
            _ => {
                // Front edge: charge readout breathing between 4 and 5 bars.
                rect(g, -30.0, 37.0, 60.0, 8.0, STEEL_DARK);
                let fill = 4 + if blink(time, 0.5, 0.0, 0.5) { 1 } else { 0 };
                for i in 0..5 {
                    let c = if i < fill { LED_GREEN } else { PANEL };
                    rect(g, -24.0 + i as f32 * 10.0, 39.0, 7.0, 4.0, c);
                }
            }
        }
    }

    /// 15 — backup generator from above: engine block with cooling fins, the
    /// round alternator, exhaust rings drifting off the stack.
    /// Layers: block, smoke, stack, gauge (needle = the layer rotation).
    fn generator(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -45.0, -25.0, 90.0, 64.0);
                rect(g, -45.0, 25.0, 90.0, 14.0, STEEL_DARK); // skid
                frame(g, -45.0, 25.0, 90.0, 14.0, 1.5, TRIM);
                rect(
                    g,
                    -40.0,
                    -25.0,
                    55.0,
                    50.0,
                    Color::new(0.30, 0.28, 0.24, 1.0),
                );
                frame(
                    g,
                    -40.0,
                    -25.0,
                    55.0,
                    50.0,
                    2.0,
                    Color::new(0.42, 0.39, 0.33, 1.0),
                );
                for i in 0..5 {
                    rect(
                        g,
                        -34.0 + i as f32 * 10.0,
                        -20.0,
                        4.0,
                        40.0,
                        Color::new(0.20, 0.19, 0.16, 1.0),
                    );
                }
                circle(g, 28.0, 0.0, 16.0, STEEL);
                circle(g, 28.0, 0.0, 5.0, TRIM);
            }
            1 => {
                // The exhaust stack pokes up past the block; from above its
                // smoke spreads as widening rings.
                for k in 0..3u32 {
                    let ph = (time * 0.45 + k as f32 / 3.0).fract();
                    circle(
                        g,
                        0.0,
                        0.0,
                        7.0 + ph * 13.0,
                        Color::new(0.5, 0.5, 0.55, 0.22 * (1.0 - ph)),
                    );
                }
            }
            2 => {
                rect(g, -33.0, -31.0, 4.0, 8.0, STEEL_DARK); // stack feed
                circle(g, -31.0, -36.0, 6.5, STEEL_DARK);
                circle(g, -31.0, -36.0, 3.0, PANEL);
            }
            _ => {
                // Fuel gauge on the block; the needle (along +x here) trembles
                // through the layer rotation while it runs.
                circle(g, 0.0, 0.0, 7.0, CREAM);
                line(g, 0.0, 0.0, 5.5, 0.0, 1.5, LED_RED);
            }
        }
    }

    /// 16 — open cable tray: a river of colour-coded runs.
    /// Layers: tray, cables.
    fn cable_tray(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(
                    g,
                    -48.0,
                    -20.0,
                    96.0,
                    40.0,
                    Color::new(0.14, 0.13, 0.18, 1.0),
                );
                rect(g, -48.0, -24.0, 96.0, 4.0, TRIM);
                rect(g, -48.0, 20.0, 96.0, 4.0, TRIM);
            }
            _ => {
                let colors = [COPPER, GLOW_CYAN, LED_GREEN, GLOW_MAGENTA, LED_AMBER];
                for (c_idx, &col) in colors.iter().enumerate() {
                    let y0 = -13.0 + c_idx as f32 * 6.5;
                    let mut lx = -46.0;
                    let mut ly = y0;
                    for seg in 1..=8 {
                        let nx = -46.0 + seg as f32 * 11.5;
                        // The live cable (cyan) hums; the rest lie still.
                        let wob = if c_idx == 1 {
                            (time * 2.0 + seg as f32).sin()
                        } else {
                            0.0
                        };
                        let ny = y0 + ((nx * 0.15) + c_idx as f32 * 2.1).sin() * 2.5 + wob;
                        line(g, lx, ly, nx, ny, 2.5, col);
                        lx = nx;
                        ly = ny;
                    }
                }
            }
        }
    }

    /// 17 — spare cable coil with its loose end and connector.
    /// Layers: coil, glints (spinning), lead.
    fn cable_coil(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                shadow_circle(g, 0.0, 0.0, 33.0);
                circle(g, 0.0, 0.0, 33.0, Color::new(0.48, 0.28, 0.20, 1.0));
                circle(g, 0.0, 0.0, 26.0, COPPER);
                circle(g, 0.0, 0.0, 19.0, Color::new(0.48, 0.28, 0.20, 1.0));
                circle(g, 0.0, 0.0, 12.0, Color::new(0.10, 0.09, 0.13, 1.0));
            }
            1 => {
                // Wind glints slowly circling the coil.
                for k in 0..3 {
                    let a = k as f32 * (TAU / 3.0);
                    g.draw_arc(Vec2::new(0.0, 0.0), 29.5, a, a + 0.9, alpha(CREAM, 0.18));
                }
            }
            _ => {
                line(g, 28.0, 14.0, 42.0, 26.0, 4.0, COPPER);
                rect(g, 40.0, 24.0, 8.0, 8.0, STEEL);
            }
        }
    }

    /// 18 — tape library from above, lid off: cartridge racks down both long
    /// walls, the picker robot riding the centre rail and reaching into the
    /// shelves. Layers: chassis, picker (self-animated along the rail).
    fn tape_library(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -42.0, -45.0, 84.0, 90.0);
                rect(g, -42.0, -45.0, 84.0, 90.0, STEEL_DARK);
                frame(g, -42.0, -45.0, 84.0, 90.0, 2.0, TRIM);
                rect(g, -37.0, -40.0, 74.0, 80.0, PANEL); // the open bay
                                                          // Cartridge racks along the two long walls.
                for side in 0..2u32 {
                    let x = if side == 0 { -35.0 } else { 21.0 };
                    for row in 0..7u32 {
                        let y = -38.0 + row as f32 * 11.0;
                        if rnd(row * 7 + side, 11) > 0.82 {
                            rect(g, x, y, 14.0, 9.0, Color::new(0.05, 0.05, 0.09, 1.0));
                        // empty
                        } else {
                            let v = 0.2 + rnd(side * 3 + row, row) * 0.15;
                            rect(g, x, y, 14.0, 9.0, Color::new(v, v * 0.95, v * 1.2, 1.0));
                            rect(g, x + 2.0, y + 3.0, 10.0, 2.0, alpha(CREAM, 0.35));
                        }
                    }
                }
                // Centre rail.
                rect(g, -2.0, -38.0, 4.0, 76.0, TRIM);
            }
            _ => {
                // The carriage rides the rail, arm reaching into a shelf.
                let ay = (time * 0.7).sin() * 28.0;
                let reach = ((time * 1.1).sin() * 0.5 + 0.5) * 14.0;
                let side = if (time * 0.23).sin() > 0.0 { 1.0 } else { -1.0 };
                line(g, 0.0, ay, side * (6.0 + reach), ay, 4.0, alpha(CREAM, 0.9));
                rect(g, side * (6.0 + reach) - 3.0, ay - 4.0, 6.0, 8.0, CREAM); // gripper
                rect(g, -7.0, ay - 8.0, 14.0, 16.0, CREAM); // carriage
                let seek = blink(time, 5.0, 0.0, 0.5);
                rect(
                    g,
                    -2.0,
                    ay - 2.0,
                    4.0,
                    4.0,
                    if seek { LED_RED } else { PANEL },
                );
            }
        }
    }

    /// 19 — strapped supply crate, stencilled for the datacenter.
    /// Layers: crate, stencil.
    fn supply_crate(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -38.0, -32.0, 76.0, 64.0);
                rect(
                    g,
                    -38.0,
                    -32.0,
                    76.0,
                    64.0,
                    Color::new(0.35, 0.27, 0.20, 1.0),
                );
                frame(
                    g,
                    -38.0,
                    -32.0,
                    76.0,
                    64.0,
                    2.5,
                    Color::new(0.24, 0.18, 0.13, 1.0),
                );
                rect(
                    g,
                    -38.0,
                    -6.0,
                    76.0,
                    12.0,
                    Color::new(0.22, 0.17, 0.13, 1.0),
                );
                rect(
                    g,
                    -8.0,
                    -32.0,
                    16.0,
                    64.0,
                    Color::new(0.26, 0.20, 0.15, 1.0),
                );
                for &(x, y) in &[(-33.0, -27.0), (28.0, -27.0), (-33.0, 22.0), (28.0, 22.0)] {
                    circle(g, x + 2.5, y + 2.5, 2.0, Color::new(0.15, 0.11, 0.08, 1.0));
                }
            }
            _ => g.draw_text(
                "CLD-01",
                Vec2::new(-22.0, 24.0),
                13.0,
                Color::new(0.9, 0.85, 0.7, 0.55),
            ),
        }
    }

    /// 20 — security camera from above: a pivot on its wall stub, panning a
    /// long watch cone across the floor (the same read as the rogues' vision
    /// cones). Layers: mount, cone (swaying), head (swaying with it).
    fn security_cam(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -16.0, -48.0, 32.0, 10.0, STEEL_DARK); // the wall stub
                frame(g, -16.0, -48.0, 32.0, 10.0, 1.5, TRIM);
            }
            1 => {
                // The watch cone sweeps the floor first, under the body.
                g.draw_arc(
                    Vec2::new(0.0, 0.0),
                    64.0,
                    FRAC_PI_2 - 0.30,
                    FRAC_PI_2 + 0.30,
                    Color::new(1.0, 0.2, 0.2, 0.07),
                );
                g.draw_arc(
                    Vec2::new(0.0, 0.0),
                    30.0,
                    FRAC_PI_2 - 0.30,
                    FRAC_PI_2 + 0.30,
                    Color::new(1.0, 0.2, 0.2, 0.06),
                );
            }
            _ => {
                // Body seen from above: housing, head, lens looking down the cone.
                rect(g, -5.0, -2.0, 10.0, 14.0, STEEL);
                rect(g, -7.0, 12.0, 14.0, 8.0, STEEL_DARK);
                circle(g, 0.0, 20.0, 3.5, PANEL);
                circle(g, 0.0, 20.0, 1.5, GLOW_CYAN);
                circle(g, 0.0, 0.0, 6.0, TRIM); // the pivot
                let recording = blink(time, 1.0, 0.0, 0.12);
                circle(g, 4.5, 4.0, 1.8, if recording { LED_RED } else { PANEL });
            }
        }
    }

    /// 21 — fire suppression pair from above: two agent tanks, handwheels up,
    /// plumbed into the discharge manifold along the back wall.
    /// Layers: manifold, tanks, wheel l, wheel r (creeping), tag.
    fn fire_suppressor(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                rect(g, -30.0, -44.0, 60.0, 7.0, STEEL); // the manifold
                for k in 0..4 {
                    circle(g, -21.0 + k as f32 * 14.0, -36.0, 2.0, PANEL); // nozzles
                }
                for &x in &[-18.0f32, 18.0] {
                    line(g, x, -37.0, x, -12.0, 4.0, TRIM); // feed pipe
                }
            }
            1 => {
                for &x in &[-18.0f32, 18.0] {
                    shadow_circle(g, x, 8.0, 17.0);
                    circle(g, x, 8.0, 17.0, TRIM);
                    circle(g, x, 8.0, 14.5, Color::new(0.72, 0.14, 0.18, 1.0));
                    circle(g, x, 8.0, 6.5, Color::new(0.55, 0.10, 0.14, 1.0)); // shoulder
                }
            }
            2 | 3 => {
                // The handwheel on top, creeping as pressure is trimmed (the
                // layer rotation).
                for s in 0..3 {
                    let a = s as f32 * (PI / 3.0);
                    line(
                        g,
                        -a.cos() * 8.5,
                        -a.sin() * 8.5,
                        a.cos() * 8.5,
                        a.sin() * 8.5,
                        1.8,
                        STEEL,
                    );
                }
                circle(g, 0.0, 0.0, 2.2, TRIM);
            }
            // Inspection tag on the floor between them.
            _ => rect(g, -4.0, 32.0, 8.0, 10.0, alpha(HAZARD_YELLOW, 0.8)),
        }
    }

    /// 22 — hazard floor pad: striped border around a KEEP CLEAR zone.
    /// Layers: pad, sign.
    fn hazard_pad(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                rect(
                    g,
                    -44.0,
                    -44.0,
                    88.0,
                    88.0,
                    Color::new(0.12, 0.11, 0.15, 1.0),
                );
                let black = Color::new(0.05, 0.05, 0.06, 1.0);
                for i in 0..11 {
                    let o = -44.0 + i as f32 * 8.0;
                    let c = if i % 2 == 0 { HAZARD_YELLOW } else { black };
                    let c2 = if i % 2 == 0 { black } else { HAZARD_YELLOW };
                    rect(g, o, -44.0, 8.0, 8.0, c);
                    rect(g, o, 36.0, 8.0, 8.0, c2);
                    rect(g, -44.0, o, 8.0, 8.0, c2);
                    rect(g, 36.0, o, 8.0, 8.0, c);
                }
            }
            _ => {
                // Warning triangle painted in the middle.
                line(g, 0.0, -18.0, 16.0, 12.0, 3.0, HAZARD_YELLOW);
                line(g, 16.0, 12.0, -16.0, 12.0, 3.0, HAZARD_YELLOW);
                line(g, -16.0, 12.0, 0.0, -18.0, 3.0, HAZARD_YELLOW);
                rect(g, -1.5, -8.0, 3.0, 10.0, HAZARD_YELLOW);
                rect(g, -1.5, 5.0, 3.0, 3.0, HAZARD_YELLOW);
            }
        }
    }

    /// 23 — the uplink obelisk from above: a diamond monolith top with seams
    /// bleeding light toward its points, an escort of motes orbiting it.
    /// Layers: aura, shadow, monolith (both static 45°), seams, escort.
    fn uplink_obelisk(g: &Graphics, layer: usize, time: f32) {
        let breath = 0.5 + 0.5 * (time * 2.0).sin();
        match layer {
            0 => {
                circle(g, 0.0, 0.0, 44.0, alpha(GLOW_MAGENTA, 0.07 + 0.05 * breath));
                circle(g, 0.0, 0.0, 30.0, alpha(GLOW_MAGENTA, 0.06 + 0.05 * breath));
            }
            // The monolith reads as a diamond from above (its shadow first).
            1 => rect(g, -16.0, -16.0, 32.0, 32.0, SHADOW),
            2 => {
                rect(
                    g,
                    -16.0,
                    -16.0,
                    32.0,
                    32.0,
                    Color::new(0.07, 0.05, 0.10, 1.0),
                );
                frame(
                    g,
                    -16.0,
                    -16.0,
                    32.0,
                    32.0,
                    1.5,
                    Color::new(0.35, 0.15, 0.35, 1.0),
                );
            }
            3 => {
                // Seams bleeding light toward the four points, the core white-hot.
                let seam = alpha(GLOW_MAGENTA, 0.35 + 0.4 * breath);
                line(g, 0.0, 0.0, 21.0, 0.0, 2.0, seam);
                line(g, 0.0, 0.0, -21.0, 0.0, 2.0, seam);
                line(g, 0.0, 0.0, 0.0, 21.0, 2.0, seam);
                line(g, 0.0, 0.0, 0.0, -21.0, 2.0, seam);
                circle(g, 0.0, 0.0, 6.0, alpha(GLOW_MAGENTA, 0.5 + 0.4 * breath));
                circle(g, 0.0, 0.0, 2.5, alpha(CREAM, 0.8));
            }
            _ => {
                // The escort.
                for k in 0..3 {
                    let a = time * 1.3 + k as f32 * 2.1;
                    let c = if k % 2 == 0 { GLOW_MAGENTA } else { GLOW_CYAN };
                    circle(g, a.cos() * 32.0, a.sin() * 36.0, 2.5, alpha(c, 0.85));
                }
            }
        }
    }
    // ======================= OUTDOOR: gate / parking lot =======================

    /// Linear blend of two colours.
    fn mix(a: Color, b: Color, t: f32) -> Color {
        Color::new(
            a.r + (b.r - a.r) * t,
            a.g + (b.g - a.g) * t,
            a.b + (b.b - a.b) * t,
            a.a + (b.a - a.a) * t,
        )
    }

    /// 24 — compact autonomous pod, parked nose-down (+y): pearl shell, glass
    /// ends, the roof lidar puck sweeping, daytime-running strips and a
    /// breathing status dot at the tail. Layers: body, glass, lidar (spin),
    /// lights.
    fn car_pod(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => car_shell(
                g,
                -19.0,
                -30.0,
                38.0,
                60.0,
                PEARL,
                Color::new(0.90, 0.91, 0.94, 1.0),
            ),
            1 => {
                rect(g, -15.0, 12.0, 30.0, 10.0, GLASS); // windscreen
                rect(g, -13.0, 13.0, 26.0, 1.5, alpha(GLASS_HI, 0.5));
                rect(g, -15.0, -22.0, 30.0, 7.0, GLASS); // rear glass
                rect(g, -13.0, -21.0, 26.0, 1.5, alpha(GLASS_HI, 0.35));
            }
            2 => lidar(g, 5.0),
            _ => {
                rect(g, -16.0, 28.0, 9.0, 2.0, alpha(CREAM, 0.85)); // DRL strips
                rect(g, 7.0, 28.0, 9.0, 2.0, alpha(CREAM, 0.85));
                rect(g, -15.0, -30.0, 7.0, 2.0, alpha(LED_RED, 0.7)); // tails
                rect(g, 8.0, -30.0, 7.0, 2.0, alpha(LED_RED, 0.7));
                let breath = 0.4 + 0.5 * (time * 1.5).sin().max(0.0);
                circle(g, 0.0, -27.0, 1.5, alpha(GLOW_CYAN, breath));
            }
        }
    }

    /// 25 — autonomous sedan rolling in, headlights on: teal shell, glass,
    /// the roof lidar spinning, a warm headlight wash spilling forward onto
    /// the asphalt. Layers: body, glass, lidar (spin), lights.
    fn car_sedan(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => car_shell(
                g,
                -21.0,
                -44.0,
                42.0,
                80.0,
                TEAL,
                Color::new(0.18, 0.42, 0.50, 1.0),
            ),
            1 => {
                rect(g, -17.0, 14.0, 34.0, 10.0, GLASS); // windscreen
                rect(g, -15.0, 15.0, 30.0, 1.5, alpha(GLASS_HI, 0.5));
                rect(g, -17.0, -30.0, 34.0, 8.0, GLASS); // rear glass
                rect(g, -15.0, -29.0, 30.0, 1.5, alpha(GLASS_HI, 0.35));
                rect(g, -19.0, -14.0, 2.0, 26.0, GLASS); // side glass
                rect(g, 17.0, -14.0, 2.0, 26.0, GLASS);
            }
            2 => lidar(g, 5.0),
            _ => {
                let fl = 0.92 + 0.08 * (time * 11.0).sin();
                wash_v(g, 0.0, 39.0, 38.0, 16.0, WARM_LIGHT, 0.24 * fl);
                rect(g, -17.0, 36.0, 10.0, 3.0, CREAM); // headlights
                rect(g, 7.0, 36.0, 10.0, 3.0, CREAM);
                rect(g, -17.0, -44.0, 9.0, 2.0, LED_RED); // tails
                rect(g, 8.0, -44.0, 9.0, 2.0, LED_RED);
                rect(g, -19.0, -45.0, 38.0, 1.5, alpha(LED_RED, 0.18));
            }
        }
    }

    /// 26 — sedan with both doors open and the cabin lit: warm light spilling
    /// out onto the ground either side, the doors as tilted layers off their
    /// hinges, seats through the glass roof, hazards blinking.
    /// Layers: spill, body, door l / r (static swing), cabin, lidar (idle),
    /// lights.
    fn car_open(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                let fl = 0.9 + 0.1 * (time * 5.0).sin();
                for &x in &[-33.0f32, 33.0] {
                    circle(g, x, -2.0, 15.0, alpha(WARM_LIGHT, 0.14 * fl));
                    circle(g, x, -2.0, 9.0, alpha(WARM_LIGHT, 0.14 * fl));
                }
            }
            1 => car_shell(
                g,
                -21.0,
                -44.0,
                42.0,
                80.0,
                PEARL,
                Color::new(0.90, 0.91, 0.94, 1.0),
            ),
            2 | 3 => {
                // A door, hinge at the origin, panel along +y (the layer's
                // static rotation swings it out).
                rect(g, -2.0, 0.0, 4.0, 24.0, PEARL);
                rect(g, -1.0, 4.0, 2.0, 12.0, GLASS);
                rect(g, -2.0, 0.0, 4.0, 1.5, TRIM);
            }
            4 => {
                rect(g, -17.0, -30.0, 34.0, 8.0, GLASS); // rear glass
                rect(g, -17.0, -20.0, 34.0, 32.0, alpha(WARM_LIGHT, 0.55)); // lit cabin
                rect(g, -12.0, -8.0, 10.0, 11.0, STEEL_DARK); // seats
                rect(g, 2.0, -8.0, 10.0, 11.0, STEEL_DARK);
                rect(g, -17.0, -20.0, 34.0, 32.0, alpha(GLASS, 0.40)); // the glass roof
                rect(g, -17.0, 14.0, 34.0, 10.0, GLASS); // windscreen
                rect(g, -15.0, 15.0, 30.0, 1.5, alpha(GLASS_HI, 0.5));
            }
            5 => lidar(g, 5.0),
            _ => {
                hazards(g, -21.0, -44.0, 42.0, 80.0, time);
                rect(g, -17.0, -44.0, 9.0, 2.0, alpha(LED_RED, 0.7));
                rect(g, 8.0, -44.0, 9.0, 2.0, alpha(LED_RED, 0.7));
            }
        }
    }

    /// 27 — boxy delivery van, hazards on: orange cargo box with roof ribs and
    /// a livery band, the cab and windscreen up front, mirrors, the roof
    /// puck. Layers: body, glass, puck (spin), lights.
    fn delivery_van(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -23.0, -46.0, 46.0, 88.0);
                for &(ax, ay) in &[(-25.0, -30.0), (22.0, -30.0), (-25.0, 22.0), (22.0, 22.0)] {
                    rect(g, ax, ay, 3.0, 12.0, TYRE);
                }
                rect(g, -23.0, -42.0, 46.0, 84.0, ORANGE);
                rect(g, -19.0, -46.0, 38.0, 4.0, ORANGE);
                // Cargo roof: ribbed, a rear-door seam at the back.
                rect(
                    g,
                    -20.0,
                    -42.0,
                    40.0,
                    60.0,
                    Color::new(0.82, 0.40, 0.14, 1.0),
                );
                for i in 0..7 {
                    rect(
                        g,
                        -20.0,
                        -38.0 + i as f32 * 8.0,
                        40.0,
                        1.5,
                        alpha(PANEL, 0.30),
                    );
                }
                line(g, 0.0, -46.0, 0.0, -38.0, 1.5, alpha(PANEL, 0.6));
                rect(g, -23.0, -8.0, 46.0, 4.0, alpha(PAINT, 0.85)); // livery band
                circle(g, 0.0, -20.0, 3.5, Color::new(0.70, 0.34, 0.12, 1.0)); // roof vent
                rect(
                    g,
                    -20.0,
                    20.0,
                    40.0,
                    10.0,
                    Color::new(0.94, 0.52, 0.22, 1.0),
                ); // cab roof
            }
            1 => {
                rect(g, -19.0, 31.0, 38.0, 9.0, GLASS); // windscreen
                rect(g, -17.0, 32.0, 34.0, 1.5, alpha(GLASS_HI, 0.5));
                rect(g, -25.0, 29.0, 4.0, 3.0, CHROME); // mirrors
                rect(g, 21.0, 29.0, 4.0, 3.0, CHROME);
            }
            2 => lidar(g, 4.0),
            _ => {
                hazards(g, -23.0, -46.0, 46.0, 88.0, time);
                rect(g, -18.0, -46.0, 10.0, 2.0, alpha(LED_RED, 0.75)); // tails
                rect(g, 8.0, -46.0, 10.0, 2.0, alpha(LED_RED, 0.75));
                rect(g, -6.0, -46.0, 4.0, 2.0, alpha(CREAM, 0.7)); // reversing
                rect(g, 2.0, -46.0, 4.0, 2.0, alpha(CREAM, 0.7));
                rect(g, -18.0, 40.0, 10.0, 2.0, alpha(CREAM, 0.6)); // dipped heads
                rect(g, 8.0, 40.0, 10.0, 2.0, alpha(CREAM, 0.6));
            }
        }
    }

    /// 28 — inductive charging pad flush with the asphalt: rubber pad, the
    /// coil disc under it, a feed conduit to the back, green status LEDs on
    /// the front edge, and the charge field pulsing outward.
    /// Layers: pad, rings (pulsing).
    fn charge_pad(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                line(g, 0.0, -30.0, 0.0, -44.0, 3.0, COPPER); // conduit
                rect(g, -5.0, -46.0, 10.0, 6.0, STEEL_DARK);
                rect(g, -30.0, -30.0, 60.0, 60.0, RUBBER);
                frame(g, -30.0, -30.0, 60.0, 60.0, 2.0, CONCRETE_DARK);
                circle(g, 0.0, 0.0, 22.0, Color::new(0.13, 0.13, 0.17, 1.0));
                for &r in &[18.0f32, 12.0, 6.0] {
                    ring(g, 0.0, 0.0, r, 1.5, alpha(COPPER, 0.5));
                }
                rect(g, -30.0, 26.0, 60.0, 4.0, STEEL_DARK);
                for i in 0..3u32 {
                    let on = blink(time, 1.0, i as f32 * 0.33, 0.5);
                    rect(
                        g,
                        -7.0 + i as f32 * 5.0,
                        27.0,
                        3.0,
                        2.0,
                        if on { LED_GREEN } else { PANEL },
                    );
                }
            }
            _ => {
                for k in 0..3u32 {
                    let ph = (time * 0.5 + k as f32 / 3.0).fract();
                    ring(
                        g,
                        0.0,
                        0.0,
                        4.0 + ph * 22.0,
                        2.0,
                        alpha(LED_GREEN, 0.45 * (1.0 - ph)),
                    );
                }
            }
        }
    }

    /// 29 — a pod parked on a charging pad, topping up: the pad's field
    /// pulsing out from under the shell, the lidar barely turning, a charge
    /// gauge filling on the flank and the amber charge LED at the tail.
    /// Layers: pad, rings, body, glass, lidar (slow), charge.
    fn car_charging(g: &Graphics, layer: usize, time: f32) {
        // The pod sits 6 units up the pad so the pad's front LEDs show.
        match layer {
            0 => charge_pad(g, 0, time),
            1 => {
                for k in 0..3u32 {
                    let ph = (time * 0.5 + k as f32 / 3.0).fract();
                    ring(
                        g,
                        0.0,
                        -6.0,
                        14.0 + ph * 16.0,
                        2.5,
                        alpha(LED_GREEN, 0.6 * (1.0 - ph)),
                    );
                }
            }
            2 | 3 => {
                g.save();
                g.translate(0.0, -6.0);
                car_pod(g, layer - 2, time);
                g.restore();
            }
            4 => lidar(g, 5.0),
            _ => {
                rect(g, -21.0, -18.0, 3.0, 24.0, PANEL);
                let fill = ((time * 0.15).fract() * 5.0) as u32;
                for i in 0..5u32 {
                    let c = if i < fill { LED_GREEN } else { STEEL_DARK };
                    rect(g, -20.5, 3.5 - i as f32 * 4.6, 2.0, 3.6, c);
                }
                if blink(time, 0.7, 0.0, 0.5) {
                    circle(g, 0.0, -33.0, 1.8, LED_AMBER);
                    circle(g, 0.0, -33.0, 4.0, alpha(LED_AMBER, 0.2));
                }
            }
        }
    }

    /// 30 — the main gate across the entry lane: kerbed asphalt with the
    /// stop line and induction loop, two scanner posts, the scan beam
    /// between them, and the swing arm — closed across the lane, then a slow
    /// swing open along it and back (the layer's `Anim`, its shadow a second
    /// layer on the same curve). Layers: lane, posts, scan, arm shadow, arm.
    fn main_gate(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -30.0, -50.0, 60.0, 100.0, ASPHALT);
                rect(g, -34.0, -50.0, 4.0, 100.0, CONCRETE_DARK); // kerbs
                rect(g, 30.0, -50.0, 4.0, 100.0, CONCRETE_DARK);
                rect(g, -26.0, 26.0, 52.0, 3.0, PAINT); // stop line
                frame(
                    g,
                    -20.0,
                    32.0,
                    40.0,
                    16.0,
                    1.0,
                    Color::new(0.05, 0.05, 0.07, 1.0),
                ); // induction loop
                for i in 0..3 {
                    rect(
                        g,
                        -1.0,
                        -46.0 + i as f32 * 14.0,
                        2.0,
                        8.0,
                        alpha(PAINT, 0.5),
                    );
                }
            }
            1 => {
                for &x in &[-48.0f32, 30.0] {
                    shadow_rect(g, x, 5.0, 12.0, 14.0);
                    rect(g, x, 5.0, 12.0, 14.0, STEEL);
                    frame(g, x, 5.0, 12.0, 14.0, 1.5, TRIM);
                    let slot = if x < 0.0 { x + 10.0 } else { x };
                    rect(g, slot, 8.0, 2.0, 8.0, PANEL); // scanner slot facing the lane
                }
            }
            2 => {
                let y = 12.0 + (time * 1.5).sin() * 4.0;
                line(g, -36.0, y, 30.0, y, 1.0, alpha(GLOW_CYAN, 0.45));
                let open = gate_angle(time).abs() > 1.0;
                for &x in &[-42.0f32, 36.0] {
                    let on = blink(time, 0.5, if x < 0.0 { 0.0 } else { 0.5 }, 0.5);
                    circle(g, x, 9.0, 1.8, if on { GLOW_CYAN } else { PANEL });
                    circle(g, x, 15.0, 1.8, if open { LED_GREEN } else { LED_RED });
                }
            }
            3 => rect(g, 0.0, -2.5, 62.0, 5.0, SHADOW),
            _ => {
                circle(g, 0.0, 0.0, 5.0, STEEL_DARK);
                rect(g, 0.0, -2.5, 62.0, 5.0, PAINT);
                for i in 0..5 {
                    rect(
                        g,
                        8.0 + i as f32 * 12.0,
                        -2.5,
                        6.0,
                        5.0,
                        Color::new(0.85, 0.15, 0.20, 1.0),
                    );
                }
                circle(g, 0.0, 0.0, 2.0, TRIM);
                let tip = blink(time, 1.0, 0.0, 0.5);
                circle(g, 60.0, 0.0, 1.8, if tip { LED_RED } else { PANEL });
            }
        }
    }

    /// 31 — the guard booth beside the lane: a tall concrete box (long
    /// shadow) with its lit window strip washing monitor light onto the
    /// asphalt at its right, the roof AC unit's fan turning, a rotating amber
    /// beacon on the roof corner. Layers: wash, booth, ac fan (spin), beacon
    /// (spin).
    fn guard_booth(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                let fl = 0.9 + 0.1 * (time * 9.0).sin();
                wash_h(
                    g,
                    20.0,
                    0.0,
                    36.0,
                    26.0,
                    Color::new(0.75, 0.95, 0.70, 1.0),
                    0.20 * fl,
                );
            }
            1 => {
                rect(g, -24.0, -22.0, 50.0, 56.0, SHADOW);
                rect(g, -30.0, -28.0, 50.0, 56.0, CONCRETE_DARK);
                frame(g, -30.0, -28.0, 50.0, 56.0, 2.0, CONCRETE);
                rect(
                    g,
                    -26.0,
                    -24.0,
                    42.0,
                    48.0,
                    Color::new(0.30, 0.30, 0.32, 1.0),
                );
                rect(g, 16.0, -20.0, 4.0, 40.0, alpha(WARM_LIGHT, 0.75)); // the lit window
                rect(g, -14.0, -30.0, 14.0, 4.0, STEEL); // door on the back wall
                rect(g, -22.0, -20.0, 16.0, 16.0, STEEL); // AC unit
                frame(g, -22.0, -20.0, 16.0, 16.0, 1.5, TRIM);
            }
            2 => fan(g, 6.0),
            _ => {
                g.draw_arc(Vec2::new(0.0, 0.0), 14.0, -0.3, 0.3, alpha(LED_AMBER, 0.28));
                circle(g, 0.0, 0.0, 3.0, LED_AMBER);
                circle(g, 0.0, 0.0, 1.5, CREAM);
            }
        }
    }

    /// 32 — a row of three bollards chained together, each capped with a
    /// breathing blue marker light. Layers: posts, leds.
    fn bollards(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                for k in 0..2 {
                    let x0 = -32.0 + k as f32 * 32.0;
                    line(g, x0, 0.0, x0 + 16.0, 3.0, 1.5, alpha(CHROME, 0.8));
                    line(g, x0 + 16.0, 3.0, x0 + 32.0, 0.0, 1.5, alpha(CHROME, 0.8));
                }
                for &x in &[-32.0f32, 0.0, 32.0] {
                    post(g, x, 0.0, 6.0, CONCRETE, STEEL);
                }
            }
            _ => {
                for (k, &x) in [-32.0f32, 0.0, 32.0].iter().enumerate() {
                    let a = 0.35 + 0.35 * (time * 2.0 + k as f32 * 2.1).sin();
                    circle(g, x, 0.0, 3.0, alpha(GLOW_CYAN, a));
                    circle(g, x, 0.0, 1.5, alpha(CREAM, a));
                }
            }
        }
    }

    /// 33 — concrete planter with a shrub rustling in it and a small solar
    /// stake lamp. Layers: box, shrub (sway), lamp.
    fn planter(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -35.0, -20.0, 70.0, 40.0);
                rect(g, -35.0, -20.0, 70.0, 40.0, CONCRETE);
                frame(g, -35.0, -20.0, 70.0, 40.0, 2.0, CONCRETE_DARK);
                rect(
                    g,
                    -31.0,
                    -16.0,
                    62.0,
                    32.0,
                    Color::new(0.16, 0.11, 0.08, 1.0),
                );
            }
            1 => {
                for k in 0..7u32 {
                    let x = -22.0 + rnd(k, 1) * 44.0;
                    let y = -9.0 + rnd(k, 2) * 18.0;
                    let r = 6.0 + rnd(k, 3) * 4.0;
                    circle(g, x + 1.0, y + 1.0, r + 1.0, LEAF_DARK);
                    circle(g, x, y, r, LEAF);
                    circle(g, x - r * 0.3, y - r * 0.3, r * 0.4, LEAF_LIGHT);
                }
            }
            _ => {
                circle(g, 32.0, -12.0, 3.5, STEEL_DARK);
                let on = blink(time, 0.4, 0.0, 0.7);
                if on {
                    circle(g, 32.0, -12.0, 5.5, alpha(WARM_LIGHT, 0.18));
                }
                circle(g, 32.0, -12.0, 2.0, if on { WARM_LIGHT } else { PANEL });
            }
        }
    }

    /// 34 — a lamp post: the pool of light it throws on the ground, the long
    /// shadow of the mast, the plinth and arm, the lamp head, two moths.
    /// Layers: pool, shadow, mast, head, moths.
    fn lamp_post(g: &Graphics, layer: usize, time: f32) {
        let (hx, hy) = (10.0f32, -6.0f32); // the lamp head
        let (bx, by) = (-14.0f32, 14.0f32); // the plinth
        match layer {
            0 => {
                let fl = if rnd(3, (time * 8.0) as u32) > 0.94 {
                    0.7
                } else {
                    1.0
                };
                circle(g, hx, hy, 44.0, alpha(WARM_LIGHT, 0.07 * fl));
                circle(g, hx, hy, 30.0, alpha(WARM_LIGHT, 0.07 * fl));
                circle(g, hx, hy, 16.0, alpha(WARM_LIGHT, 0.09 * fl));
            }
            1 => {
                line(g, bx, by, 22.0, 44.0, 3.0, alpha(SHADOW, 0.22));
                circle(g, 28.0, 40.0, 5.0, alpha(SHADOW, 0.22));
            }
            2 => {
                post(g, bx, by, 5.0, CONCRETE, STEEL_DARK);
                line(g, bx, by, hx, hy, 4.0, STEEL_DARK);
                line(g, bx, by, hx, hy, 1.5, TRIM);
            }
            3 => {
                rect(g, hx - 7.0, hy - 6.0, 16.0, 10.0, STEEL);
                frame(g, hx - 7.0, hy - 6.0, 16.0, 10.0, 1.5, TRIM);
                circle(g, hx, hy, 6.0, alpha(WARM_LIGHT, 0.5));
                circle(g, hx, hy, 4.0, WARM_LIGHT);
                circle(g, hx, hy, 2.0, CREAM);
            }
            _ => {
                for k in 0..2 {
                    let a = time * (4.0 + k as f32) + k as f32 * 3.0;
                    let r = 6.0 + 4.0 * (time * 1.7 + k as f32).sin();
                    circle(g, a.cos() * r, a.sin() * r * 0.8, 1.0, alpha(CREAM, 0.7));
                }
            }
        }
    }

    /// 35 — an EV parking bay: painted bay lines, the EV glyph and a bolt
    /// roundel on the asphalt. A flat decal. Layers: asphalt, lines, glyph.
    fn ev_bay(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                rect(g, -45.0, -45.0, 90.0, 90.0, alpha(ASPHALT, 0.95));
                circle(g, 8.0, -6.0, 9.0, alpha(Color::BLACK, 0.25));
                circle(g, 4.0, -9.0, 5.0, alpha(Color::BLACK, 0.2));
            }
            1 => {
                rect(g, -40.0, -44.0, 4.0, 84.0, PAINT);
                rect(g, 36.0, -44.0, 4.0, 84.0, PAINT);
                rect(g, -40.0, -44.0, 80.0, 4.0, PAINT);
                for k in 0..4u32 {
                    let x = -40.0 + rnd(k, 1) * 80.0;
                    let y = -44.0 + rnd(k, 2) * 84.0;
                    rect(g, x, y, 3.0, 2.0, alpha(ASPHALT, 0.8)); // wear
                }
            }
            _ => {
                let c = alpha(LED_GREEN, 0.85);
                rect(g, -18.0, 0.0, 4.0, 20.0, c); // E
                rect(g, -18.0, 0.0, 14.0, 4.0, c);
                rect(g, -18.0, 8.0, 11.0, 4.0, c);
                rect(g, -18.0, 16.0, 14.0, 4.0, c);
                line(g, 2.0, 0.0, 10.0, 20.0, 4.0, c); // V
                line(g, 18.0, 0.0, 10.0, 20.0, 4.0, c);
                ring(g, 0.0, -20.0, 8.0, 1.5, PAINT); // the bolt roundel
                line(g, 2.0, -27.0, -2.0, -20.0, 2.0, HAZARD_YELLOW);
                line(g, -2.0, -20.0, 2.0, -20.0, 2.0, HAZARD_YELLOW);
                line(g, 2.0, -20.0, -2.0, -13.0, 2.0, HAZARD_YELLOW);
            }
        }
    }

    /// 36 — a zebra crossing over the lane, stop line behind it. Flat decal.
    /// Layers: asphalt, stripes.
    fn crosswalk(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                rect(g, -45.0, -45.0, 90.0, 90.0, alpha(ASPHALT, 0.95));
                rect(g, -44.0, 34.0, 88.0, 4.0, PAINT);
            }
            _ => {
                for i in 0..6u32 {
                    let x = -44.0 + i as f32 * 15.6;
                    rect(g, x, -30.0, 9.0, 60.0, PAINT);
                    for k in 0..2u32 {
                        rect(
                            g,
                            x + rnd(i, k) * 7.0,
                            -30.0 + rnd(k, i) * 56.0,
                            2.0,
                            3.0,
                            alpha(ASPHALT, 0.8),
                        );
                    }
                }
            }
        }
    }

    /// 37 — delivery drone pad with a quadcopter landed on the H: the pad's
    /// corner beacons chasing, the drone (arms, motor pods, gimbal, status
    /// LED), four rotors that shiver now and then (each an `Anim` layer).
    /// Layers: pad, beacons, drone, rotor a..d.
    fn drone_pad(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                let pad = Color::new(0.12, 0.12, 0.15, 1.0);
                circle(g, 0.0, 0.0, 44.0, pad);
                circle(g, 0.0, 0.0, 40.0, PAINT);
                circle(g, 0.0, 0.0, 37.0, pad);
                rect(g, -16.0, -14.0, 6.0, 28.0, HAZARD_YELLOW);
                rect(g, 10.0, -14.0, 6.0, 28.0, HAZARD_YELLOW);
                rect(g, -10.0, -3.0, 20.0, 6.0, HAZARD_YELLOW);
            }
            1 => {
                let lit = (time * 2.0) as u32 % 4;
                for (k, &(x, y)) in [
                    (-31.0f32, -31.0f32),
                    (31.0, -31.0),
                    (31.0, 31.0),
                    (-31.0, 31.0),
                ]
                .iter()
                .enumerate()
                {
                    let on = lit == k as u32;
                    if on {
                        circle(g, x, y, 5.0, alpha(LED_RED, 0.2));
                    }
                    circle(g, x, y, 2.5, if on { LED_RED } else { PANEL });
                }
            }
            2 => {
                circle(g, 3.0, 3.0, 8.0, SHADOW);
                for &(x, y) in &[
                    (-16.0f32, -16.0f32),
                    (16.0, -16.0),
                    (-16.0, 16.0),
                    (16.0, 16.0),
                ] {
                    line(g, 3.0, 3.0, x + 3.0, y + 3.0, 3.0, alpha(SHADOW, 0.2));
                    line(g, 0.0, 0.0, x, y, 3.0, STEEL_DARK);
                    circle(g, x, y, 4.0, STEEL_DARK);
                    circle(g, x, y, 2.0, TRIM);
                }
                circle(g, 0.0, 0.0, 7.0, STEEL);
                circle(g, 0.0, 0.0, 4.0, PANEL);
                circle(g, 0.0, 9.0, 2.5, PANEL); // the gimbal
                circle(g, 0.0, 9.0, 1.0, GLOW_CYAN);
                let on = blink(time, 1.0, 0.0, 0.15);
                circle(g, -4.0, -4.0, 1.5, if on { LED_RED } else { STEEL_DARK });
            }
            _ => {
                rect(g, -11.0, -1.2, 22.0, 2.4, alpha(CHROME, 0.85));
                circle(g, 0.0, 0.0, 1.5, TRIM);
            }
        }
    }

    /// One e-scooter docked front-up, in its layer's frame (deck stripe /
    /// LED per `variant`).
    fn scooter(g: &Graphics, variant: u32, time: f32) {
        rect(g, -3.0, -28.0, 10.0, 60.0, SHADOW);
        rect(g, -3.0, -30.0, 6.0, 8.0, TYRE);
        rect(g, -3.0, 22.0, 6.0, 8.0, TYRE);
        rect(
            g,
            -5.0,
            -22.0,
            10.0,
            44.0,
            Color::new(0.16, 0.16, 0.20, 1.0),
        );
        let stripe = match variant {
            0 => Color::new(0.60, 0.90, 0.20, 1.0),
            1 => GLOW_CYAN,
            _ => NEON_PINK,
        };
        rect(g, -1.0, -18.0, 2.0, 36.0, stripe);
        line(g, 0.0, -24.0, 0.0, -28.0, 3.0, STEEL);
        rect(g, -9.0, -30.0, 18.0, 3.0, STEEL_DARK);
        rect(g, -9.0, -30.0, 3.0, 3.0, RUBBER);
        rect(g, 6.0, -30.0, 3.0, 3.0, RUBBER);
        let led = match variant {
            0 => {
                if blink(time, 1.0, 0.0, 0.5) {
                    LED_GREEN
                } else {
                    PANEL
                }
            }
            1 => LED_AMBER,
            _ => PANEL,
        };
        circle(g, 0.0, 14.0, 1.5, led);
    }

    /// 38 — a scooter rack: the docking rail, two scooters docked and a
    /// third leaning in its slot (a static tilt). Layers: rail, scooter a /
    /// b / c.
    fn scooter_rack(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -44.0, -34.0, 88.0, 6.0);
                rect(g, -44.0, -34.0, 88.0, 6.0, CHROME);
                rect(g, -44.0, -36.0, 6.0, 10.0, STEEL_DARK);
                rect(g, 38.0, -36.0, 6.0, 10.0, STEEL_DARK);
                for &x in &[-28.0f32, 0.0, 28.0] {
                    rect(g, x - 6.0, -32.0, 12.0, 4.0, STEEL_DARK);
                }
            }
            n => scooter(g, (n - 1) as u32, time),
        }
    }

    /// 39 — a storm drain in a puddle: the wet patch, the grate, a sheen
    /// drifting over the water and a drip's ripple. Flat. Layers: puddle,
    /// grate, sheen.
    fn drain_grate(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                circle(g, 2.0, 6.0, 30.0, Color::new(0.05, 0.07, 0.12, 0.55));
                circle(g, 4.0, 8.0, 24.0, Color::new(0.08, 0.10, 0.16, 0.35));
            }
            1 => {
                rect(g, -24.0, -14.0, 48.0, 28.0, STEEL_DARK);
                frame(g, -24.0, -14.0, 48.0, 28.0, 2.0, CONCRETE_DARK);
                for i in 0..6 {
                    rect(g, -20.0 + i as f32 * 6.5, -10.0, 3.0, 20.0, PANEL);
                }
                for &(x, y) in &[
                    (-21.0f32, -11.0f32),
                    (21.0, -11.0),
                    (-21.0, 11.0),
                    (21.0, 11.0),
                ] {
                    circle(g, x, y, 1.5, CONCRETE);
                }
            }
            _ => {
                let x = -28.0 + (time * 0.12).fract() * 56.0;
                rect(g, x, -20.0, 6.0, 50.0, Color::new(0.6, 0.7, 0.9, 0.08));
                let ph = (time * 0.7).fract();
                ring(
                    g,
                    10.0,
                    14.0,
                    2.0 + ph * 10.0,
                    1.5,
                    alpha(GLASS_HI, 0.3 * (1.0 - ph)),
                );
            }
        }
    }

    /// 40 — a tall holo billboard: the sign slab edge-on with its neon front
    /// edge, raised on two columns (long shadow), a big colour-cycling wash
    /// down the ground in front of it and a glyph strip crawling along the
    /// face. Layers: wash, shadow, slab, glyphs.
    fn holo_billboard(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                let c = mix(NEON_PINK, GLOW_CYAN, 0.5 + 0.5 * (time * 0.5).sin());
                for i in 0..5 {
                    let t = i as f32;
                    rect(
                        g,
                        -42.0 - t * 2.0,
                        -24.0 + t * 14.0,
                        84.0 + t * 4.0,
                        14.0,
                        alpha(c, 0.30 * (1.0 - t / 5.0)),
                    );
                }
                let y = -24.0 + (time * 0.35).fract() * 70.0;
                rect(g, -50.0, y, 100.0, 3.0, alpha(CREAM, 0.10));
            }
            1 => {
                rect(g, -32.0, -24.0, 84.0, 10.0, Color::new(0.0, 0.0, 0.0, 0.22));
                circle(g, -20.0, -30.0, 5.0, Color::new(0.0, 0.0, 0.0, 0.22));
                circle(g, 40.0, -30.0, 5.0, Color::new(0.0, 0.0, 0.0, 0.22));
            }
            2 => {
                for &x in &[-30.0f32, 30.0] {
                    circle(g, x, -40.0, 5.0, STEEL_DARK);
                    circle(g, x, -40.0, 3.0, TRIM);
                }
                rect(g, -42.0, -34.0, 84.0, 10.0, PANEL);
                frame(g, -42.0, -34.0, 84.0, 10.0, 1.5, TRIM);
                let pulse = 0.7 + 0.3 * (time * 3.0).sin();
                rect(g, -42.0, -25.0, 84.0, 2.0, alpha(NEON_PINK, pulse));
                rect(g, -40.0, -33.0, 80.0, 1.0, alpha(CREAM, 0.15));
            }
            _ => glyph_strip(g, -40.0, 40.0, -32.0, time, GLOW_CYAN),
        }
    }

    /// 41 — a dumpster, one lid down and one flipped open over the back
    /// edge, bags and a box in the open half, flies. Layers: body, lid l,
    /// lid r (static tilt), flies.
    fn dumpster(g: &Graphics, layer: usize, time: f32) {
        let green = Color::new(0.16, 0.32, 0.22, 1.0);
        let lid = Color::new(0.20, 0.38, 0.27, 1.0);
        let lid_edge = Color::new(0.26, 0.46, 0.34, 1.0);
        match layer {
            0 => {
                rect(g, -25.0, -17.0, 60.0, 44.0, SHADOW);
                rect(g, -30.0, -22.0, 60.0, 44.0, green);
                frame(g, -30.0, -22.0, 60.0, 44.0, 2.0, lid_edge);
                rect(g, 0.0, -20.0, 28.0, 40.0, Color::new(0.05, 0.05, 0.06, 1.0));
                circle(g, 10.0, -8.0, 7.0, Color::new(0.12, 0.12, 0.14, 1.0));
                circle(g, 19.0, 6.0, 6.0, Color::new(0.14, 0.13, 0.15, 1.0));
                rect(g, 4.0, 4.0, 10.0, 8.0, Color::new(0.45, 0.35, 0.22, 1.0));
                for i in 0..10 {
                    let c = if i % 2 == 0 {
                        HAZARD_YELLOW
                    } else {
                        Color::new(0.05, 0.05, 0.06, 1.0)
                    };
                    rect(g, -30.0 + i as f32 * 6.0, 18.0, 6.0, 4.0, c);
                }
            }
            1 => {
                rect(g, -30.0, -22.0, 30.0, 44.0, lid);
                frame(g, -30.0, -22.0, 30.0, 44.0, 1.5, lid_edge);
                for &y in &[-12.0f32, 0.0, 12.0] {
                    line(g, -28.0, y, -2.0, y, 1.0, alpha(PANEL, 0.3));
                }
                rect(g, -19.0, 16.0, 8.0, 3.0, STEEL_DARK); // handle
            }
            2 => {
                rect(g, 0.0, -9.0, 30.0, 8.0, lid);
                frame(g, 0.0, -9.0, 30.0, 8.0, 1.5, lid_edge);
                rect(g, 0.0, -1.0, 30.0, 1.0, STEEL_DARK); // the hinge
            }
            _ => {
                for k in 0..2 {
                    let f = k as f32;
                    let x = 14.0 + (time * 7.0 + f).cos() * 9.0;
                    let y = -4.0 + (time * 9.3 + f * 2.0).sin() * 7.0;
                    circle(g, x, y, 1.2, Color::new(0.05, 0.05, 0.05, 0.9));
                }
            }
        }
    }

    // ========================= LOBBY: the welcome hall =========================

    /// 42 — the reception desk: a long walnut counter with angled return
    /// wings (static layers), a breathing light strip along the visitor
    /// side, the desk lamp's pool, a bell, paperwork, an edge-on terminal
    /// washing green light back over the receptionist's side, and the chair
    /// pushed back behind. Layers: shadow, wing l / r, desk, terminal, chair.
    fn reception_desk(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => shadow_rect(g, -40.0, -10.0, 80.0, 20.0),
            1 | 2 => {
                let x = if layer == 1 { -26.0 } else { 0.0 };
                rect(g, x + 4.0, -6.0, 26.0, 20.0, SHADOW);
                rect(g, x, -10.0, 26.0, 20.0, WALNUT);
                rect(g, x, 4.0, 26.0, 6.0, WALNUT_LIGHT);
                rect(g, x, 8.0, 26.0, 2.0, alpha(GLOW_CYAN, 0.55));
            }
            3 => {
                rect(g, -40.0, -10.0, 80.0, 20.0, WALNUT);
                rect(g, -40.0, 4.0, 80.0, 6.0, WALNUT_LIGHT); // the raised transaction top
                let breath = 0.45 + 0.3 * (time * 1.2).sin();
                rect(g, -40.0, 8.0, 80.0, 2.0, alpha(GLOW_CYAN, breath));
                circle(g, -24.0, -3.0, 11.0, alpha(WARM_LIGHT, 0.22)); // lamp pool
                circle(g, -24.0, -3.0, 6.0, alpha(WARM_LIGHT, 0.22));
                circle(g, -24.0, -4.0, 4.0, STEEL); // lamp head
                circle(g, -24.0, -4.0, 2.5, WARM_LIGHT);
                circle(g, 24.0, -2.0, 3.5, BRASS); // the bell
                circle(g, 24.0, -2.0, 1.5, Color::new(0.95, 0.85, 0.55, 1.0));
                rect(g, 8.0, -6.0, 10.0, 8.0, alpha(CREAM, 0.85)); // paperwork
                line(g, 10.0, -3.0, 16.0, -3.0, 1.0, alpha(PANEL, 0.4));
                line(g, 10.0, -1.0, 15.0, -1.0, 1.0, alpha(PANEL, 0.4));
            }
            4 => {
                let fl = 0.9 + 0.1 * (time * 13.0).sin();
                wash_v(g, 0.0, -3.0, 24.0, -22.0, LED_GREEN, 0.18 * fl);
                rect(g, -12.0, -2.0, 24.0, 5.0, PANEL);
                rect(g, -12.0, -3.0, 24.0, 1.5, alpha(LED_GREEN, 0.8 * fl));
            }
            _ => chair(g, (time * 0.4).sin() * 1.5, -26.0),
        }
    }

    /// 43 — a pair of turnstile lanes: three housings with their card
    /// readers, lane chevrons on the floor, the left arm locked (red), the
    /// right arm swinging through with each walker (the layer's `Anim`,
    /// green when open). Layers: floor, housings, arm l, arm r, leds.
    fn turnstiles(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -46.0, -2.0, 92.0, 4.0, RUBBER);
                for &x in &[-17.0f32, 17.0] {
                    chevron(g, x, 18.0, 6.0, 2.5, alpha(PAINT, 0.5));
                    chevron(g, x, 27.0, 6.0, 2.5, alpha(PAINT, 0.3));
                }
            }
            1 => {
                for &x in &[-40.0f32, 0.0, 40.0] {
                    shadow_rect(g, x - 6.0, -20.0, 12.0, 40.0);
                    rect(g, x - 6.0, -20.0, 12.0, 40.0, STEEL_DARK);
                    frame(g, x - 6.0, -20.0, 12.0, 40.0, 1.5, TRIM);
                    rect(g, x - 4.0, -16.0, 8.0, 20.0, GLASS);
                    rect(g, x - 3.0, -6.0, 6.0, 4.0, alpha(GLOW_CYAN, 0.6)); // the reader
                }
            }
            2 => {
                rect(g, 2.0, 2.0, 28.0, 3.0, SHADOW);
                circle(g, 0.0, 0.0, 3.5, STEEL);
                rect(g, 0.0, -2.0, 28.0, 4.0, CHROME);
            }
            3 => {
                rect(g, -26.0, 2.0, 28.0, 3.0, SHADOW);
                circle(g, 0.0, 0.0, 3.5, STEEL);
                rect(g, -28.0, -2.0, 28.0, 4.0, CHROME);
            }
            _ => {
                let open = turnstile_angle(time) > 0.2;
                rect(g, -42.0, 14.0, 4.0, 3.0, LED_RED); // left lane: locked
                rect(g, -2.0, 14.0, 4.0, 3.0, LED_RED);
                let go = if open || blink(time, 1.0, 0.0, 0.5) {
                    LED_GREEN
                } else {
                    PANEL
                };
                rect(g, 3.0, 14.0, 4.0, 3.0, go);
                rect(g, 38.0, 14.0, 4.0, 3.0, go);
            }
        }
    }

    /// 44 — the security scanner arch over a rubber mat: two pillars joined
    /// by the beam overhead (long shadows), a scan line sweeping the gap,
    /// pass / deny LEDs on the pillar tops. Layers: mat, arch, sweep, leds.
    fn scanner_arch(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -40.0, -26.0, 80.0, 52.0, RUBBER);
                frame(g, -40.0, -26.0, 80.0, 52.0, 1.5, CONCRETE_DARK);
                circle(g, -4.0, 12.0, 3.0, alpha(CREAM, 0.12));
                circle(g, 4.0, 12.0, 3.0, alpha(CREAM, 0.12));
            }
            1 => {
                rect(g, -30.0, -14.0, 12.0, 40.0, SHADOW);
                rect(g, 30.0, -14.0, 12.0, 40.0, SHADOW);
                rect(g, -18.0, 1.0, 48.0, 10.0, SHADOW);
                rect(g, -36.0, -20.0, 12.0, 40.0, STEEL);
                frame(g, -36.0, -20.0, 12.0, 40.0, 1.5, TRIM);
                rect(g, 24.0, -20.0, 12.0, 40.0, STEEL);
                frame(g, 24.0, -20.0, 12.0, 40.0, 1.5, TRIM);
                rect(g, -24.0, -5.0, 48.0, 10.0, TRIM);
                rect(g, -24.0, -3.0, 48.0, 6.0, STEEL);
            }
            2 => {
                let y = (time * 1.5).sin() * 16.0;
                let y2 = (time * 1.5 - 0.15).sin() * 16.0;
                line(g, -24.0, y2, 24.0, y2, 1.2, alpha(GLOW_CYAN, 0.2));
                line(g, -24.0, y, 24.0, y, 1.2, alpha(GLOW_CYAN, 0.55));
            }
            _ => {
                let deny = (time * 0.5) as u32 % 3 == 2;
                let c = if deny {
                    if blink(time, 4.0, 0.0, 0.5) {
                        LED_RED
                    } else {
                        PANEL
                    }
                } else {
                    LED_GREEN
                };
                for &x in &[-33.0f32, 31.0] {
                    rect(g, x, -17.0, 4.0, 3.0, c);
                }
            }
        }
    }

    /// A slatted bench `w` wide, in its layer's frame: end frames, backrest
    /// rail along the back (-y), the slats over a dark gap.
    fn bench(g: &Graphics, w: f32) {
        let hw = w / 2.0;
        shadow_rect(g, -hw, -16.0, w, 30.0);
        rect(
            g,
            -hw + 5.0,
            -14.0,
            w - 10.0,
            28.0,
            Color::new(0.05, 0.05, 0.06, 1.0),
        );
        rect(g, -hw, -16.0, 5.0, 30.0, STEEL_DARK);
        rect(g, hw - 5.0, -16.0, 5.0, 30.0, STEEL_DARK);
        rect(g, -hw + 5.0, -18.0, w - 10.0, 4.0, WALNUT);
        for i in 0..5 {
            let c = if i % 2 == 0 { WALNUT_LIGHT } else { WALNUT };
            rect(g, -hw + 5.0, -13.0 + i as f32 * 5.4, w - 10.0, 4.0, c);
        }
    }

    /// 45 — the long waiting bench, a coffee cup and a folded paper left on
    /// it. Layers: bench, items.
    fn bench_long(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => bench(g, 84.0),
            _ => {
                circle(g, 20.0, -2.0, 3.5, CREAM);
                circle(g, 20.0, -2.0, 2.0, Color::new(0.35, 0.20, 0.10, 1.0));
                rect(g, -26.0, -5.0, 12.0, 8.0, alpha(CREAM, 0.85));
                line(g, -24.0, -2.0, -16.0, -2.0, 1.0, alpha(PANEL, 0.4));
                line(g, -24.0, 0.5, -18.0, 0.5, 1.0, alpha(PANEL, 0.4));
            }
        }
    }

    /// 46 — the short bench, someone's backpack on it. Layers: bench, items.
    fn bench_short(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => bench(g, 56.0),
            _ => {
                rect(g, 8.0, -7.0, 10.0, 13.0, Color::new(0.15, 0.22, 0.40, 1.0));
                rect(g, 8.0, -7.0, 10.0, 3.0, Color::new(0.20, 0.30, 0.50, 1.0));
                line(g, 9.0, 6.0, 4.0, 10.0, 1.5, STEEL_DARK); // strap
            }
        }
    }

    /// 47 — a potted plant: terracotta pot, fronds fanning out and swaying,
    /// the nursery tag. Layers: pot, leaves (sway), tag.
    fn potted_plant(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                shadow_circle(g, 0.0, 0.0, 20.0);
                circle(g, 0.0, 0.0, 20.0, Color::new(0.62, 0.35, 0.25, 1.0));
                circle(g, 0.0, 0.0, 17.0, Color::new(0.16, 0.11, 0.08, 1.0));
                circle(g, -13.0, -13.0, 3.0, alpha(CREAM, 0.25));
            }
            1 => {
                for k in 0..8 {
                    let a = k as f32 * (TAU / 8.0) + 0.3;
                    let (tx, ty) = (a.cos() * 26.0, a.sin() * 26.0);
                    line(g, 0.0, 0.0, tx, ty, 5.0, LEAF);
                    circle(g, tx * 0.85, ty * 0.85, 4.0, LEAF_LIGHT);
                    let b = a + TAU / 16.0;
                    line(g, 0.0, 0.0, b.cos() * 16.0, b.sin() * 16.0, 4.0, LEAF_DARK);
                }
                circle(g, 0.0, 0.0, 5.0, LEAF_DARK);
            }
            _ => {
                line(g, 12.0, 6.0, 13.0, 11.0, 1.0, CREAM);
                rect(g, 10.0, 10.0, 7.0, 6.0, alpha(CREAM, 0.9));
            }
        }
    }

    /// 48 — the big lobby holo-screen: a wall-mounted edge-on slab, a wide
    /// cool wash down the floor with a scan band rolling through it, glyphs
    /// crawling along the face and a live bar graph. Layers: wash, slab,
    /// glyphs.
    fn lobby_holo(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                let c = mix(
                    GLOW_CYAN,
                    Color::new(0.6, 0.5, 1.0, 1.0),
                    0.5 + 0.5 * (time * 0.3).sin(),
                );
                for i in 0..5 {
                    let t = i as f32;
                    rect(
                        g,
                        -44.0 - t * 1.5,
                        -28.0 + t * 13.6,
                        88.0 + t * 3.0,
                        13.6,
                        alpha(c, 0.28 * (1.0 - t / 5.0)),
                    );
                }
                let y = -28.0 + (time * 0.3).fract() * 68.0;
                rect(g, -50.0, y, 100.0, 3.0, alpha(CREAM, 0.08));
            }
            1 => {
                rect(g, -30.0, -44.0, 60.0, 6.0, STEEL_DARK); // wall bracket
                rect(g, -44.0, -38.0, 88.0, 10.0, PANEL);
                frame(g, -44.0, -38.0, 88.0, 10.0, 1.5, TRIM);
                let fl = 0.85 + 0.15 * (time * 7.0).sin();
                rect(g, -44.0, -29.0, 88.0, 2.0, alpha(GLOW_CYAN, 0.85 * fl));
            }
            _ => {
                glyph_strip(g, -42.0, 26.0, -36.0, time, GLOW_CYAN);
                let tick = (time * 3.0) as u32;
                for i in 0..5u32 {
                    let h = 1.0 + rnd(i, tick) * 5.0;
                    rect(g, 28.0 + i as f32 * 3.0, -30.0 - h, 2.0, h, GLOW_MAGENTA);
                }
            }
        }
    }

    /// 49 — the floor directory totem: a slim kiosk whose top face is a
    /// display scrolling a listing, its front edge lit and washing the floor.
    /// Layers: body, screen (scrolling), wash.
    fn directory_totem(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -5.0, -17.0, 20.0, 44.0, SHADOW);
                rect(g, -10.0, -22.0, 20.0, 44.0, STEEL);
                frame(g, -10.0, -22.0, 20.0, 44.0, 1.5, TRIM);
            }
            1 => {
                rect(g, -8.0, -18.0, 16.0, 30.0, PANEL);
                for k in 0..7u32 {
                    let y = -16.0 + (k as f32 * 4.5 + time * 3.0).rem_euclid(26.0);
                    let w = 4.0 + rnd(k, 1) * 7.0;
                    let c = if k == 0 { CREAM } else { alpha(GLOW_CYAN, 0.8) };
                    rect(g, -6.0, y, w, 2.0, c);
                }
            }
            _ => {
                rect(g, -10.0, 20.0, 20.0, 2.0, alpha(GLOW_CYAN, 0.7));
                wash_v(g, 0.0, 22.0, 20.0, 14.0, GLOW_CYAN, 0.16);
            }
        }
    }

    /// 50 — a vending machine: vented top, the lit front glass with its
    /// product rows glowing along the front edge, the select panel, and the
    /// fluorescent wash on the floor in front (with the odd flicker).
    /// Layers: body, front, wash.
    fn vending_machine(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -19.0, -15.0, 48.0, 40.0, SHADOW);
                rect(g, -24.0, -20.0, 48.0, 40.0, STEEL);
                frame(g, -24.0, -20.0, 48.0, 40.0, 2.0, TRIM);
                for i in 0..4 {
                    rect(g, -16.0, -14.0 + i as f32 * 5.0, 32.0, 2.0, PANEL);
                }
                rect(g, -24.0, 6.0, 48.0, 3.0, alpha(LED_RED, 0.7)); // brand band
            }
            1 => {
                rect(g, -22.0, 13.0, 44.0, 7.0, GLASS);
                let cols = [
                    LED_RED,
                    LED_AMBER,
                    GLOW_CYAN,
                    GLOW_MAGENTA,
                    LED_GREEN,
                    CREAM,
                ];
                for row in 0..2u32 {
                    for i in 0..8u32 {
                        if rnd(i, row + 7) > 0.85 {
                            continue; // sold out
                        }
                        let c = cols[(rnd(i, row) * cols.len() as f32) as usize % cols.len()];
                        rect(
                            g,
                            -20.0 + i as f32 * 4.2,
                            14.5 + row as f32 * 3.0,
                            2.5,
                            2.0,
                            c,
                        );
                    }
                }
                rect(g, 14.0, 13.0, 8.0, 7.0, STEEL_DARK); // select panel
                let on = blink(time, 1.5, 0.0, 0.5);
                rect(g, 17.0, 15.0, 2.0, 2.0, if on { LED_GREEN } else { PANEL });
            }
            _ => {
                let fl = if rnd(1, (time * 6.0) as u32) > 0.9 {
                    0.6
                } else {
                    1.0
                };
                wash_v(
                    g,
                    0.0,
                    20.0,
                    44.0,
                    18.0,
                    Color::new(0.85, 0.95, 1.0, 1.0),
                    0.16 * fl,
                );
            }
        }
    }

    /// 51 — the coffee corner: a small counter, the espresso machine with its
    /// heating LEDs, cups lined up, a sugar jar, steam wisping off the
    /// machine. Layers: counter, machine, cups, steam.
    fn coffee_corner(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -35.0, -14.0, 70.0, 28.0);
                rect(g, -35.0, -14.0, 70.0, 28.0, WALNUT);
                rect(g, -35.0, 11.0, 70.0, 3.0, alpha(CHROME, 0.6));
                rect(g, 24.0, 4.0, 8.0, 6.0, CREAM); // napkins
            }
            1 => {
                rect(g, -28.0, -10.0, 22.0, 20.0, STEEL_DARK);
                frame(g, -28.0, -10.0, 22.0, 20.0, 1.5, TRIM);
                rect(g, -26.0, -8.0, 18.0, 6.0, CHROME);
                circle(g, -17.0, 2.0, 3.0, STEEL);
                line(g, -17.0, 2.0, -8.0, 6.0, 2.0, STEEL_DARK); // portafilter handle
                rect(g, -24.0, 6.0, 14.0, 3.0, PANEL); // drip tray
                let heat = blink(time, 0.7, 0.0, 0.7);
                circle(g, -26.0, -6.0, 1.2, if heat { LED_RED } else { PANEL });
                circle(g, -23.0, -6.0, 1.2, GLOW_CYAN);
            }
            2 => {
                for &x in &[4.0f32, 11.0, 18.0, 25.0] {
                    circle(g, x, -6.0, 3.0, CREAM);
                    circle(g, x, -6.0, 1.8, Color::new(0.90, 0.85, 0.75, 1.0));
                }
                circle(g, 28.0, 4.0, 4.0, alpha(GLASS_HI, 0.7)); // sugar jar
                circle(g, 28.0, 4.0, 2.5, CREAM);
                line(g, 4.0, 4.0, 14.0, 6.0, 1.5, CHROME); // a spoon
            }
            _ => {
                for k in 0..3u32 {
                    let ph = (time * 0.4 + k as f32 / 3.0).fract();
                    let x = (ph * 6.0 + k as f32 * 2.0).sin() * 3.0;
                    circle(
                        g,
                        x,
                        -ph * 14.0,
                        1.5 + ph * 1.5,
                        alpha(CREAM, 0.35 * (1.0 - ph)),
                    );
                }
            }
        }
    }

    /// 52 — a bank of charging lockers: the cabinet with its door grid, the
    /// front LED matrix (free / charging / done / dark per locker), a cable
    /// left dangling on the floor. Layers: cabinet, leds, cable.
    fn charge_lockers(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                shadow_rect(g, -40.0, -16.0, 80.0, 32.0);
                rect(g, -40.0, -16.0, 80.0, 32.0, STEEL);
                frame(g, -40.0, -16.0, 80.0, 32.0, 2.0, TRIM);
                for c in 0..8 {
                    for r in 0..2 {
                        frame(
                            g,
                            -38.0 + c as f32 * 9.5,
                            -14.0 + r as f32 * 12.0,
                            9.0,
                            11.0,
                            1.0,
                            alpha(PANEL, 0.5),
                        );
                    }
                }
            }
            1 => {
                rect(g, -40.0, 10.0, 80.0, 6.0, STEEL_DARK);
                for c in 0..8u32 {
                    for r in 0..2u32 {
                        let s = rnd(c, r + 1);
                        let col = if s < 0.35 {
                            LED_GREEN
                        } else if s < 0.7 {
                            alpha(
                                LED_AMBER,
                                0.4 + 0.6 * (0.5 + 0.5 * (time * 3.0 + c as f32).sin()),
                            )
                        } else if s < 0.85 {
                            GLOW_CYAN
                        } else {
                            PANEL
                        };
                        rect(
                            g,
                            -37.0 + c as f32 * 9.5,
                            11.0 + r as f32 * 2.8,
                            3.0,
                            2.0,
                            col,
                        );
                    }
                }
            }
            _ => {
                line(g, 14.0, 16.0, 18.0, 26.0, 2.0, COPPER);
                line(g, 18.0, 26.0, 30.0, 32.0, 2.0, COPPER);
                rect(g, 29.0, 30.0, 6.0, 5.0, STEEL_DARK);
            }
        }
    }

    /// 53 — the tower's mark inlaid in the lobby floor: brass rings in the
    /// marble, the obelisk's diamond at the centre (a static 45° layer) and
    /// its four seams. A flat decal. Layers: inlay, mark, seams.
    fn floor_logo(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                circle(g, 0.0, 0.0, 40.0, alpha(BRASS, 0.9));
                circle(g, 0.0, 0.0, 37.0, MARBLE);
                circle(g, 0.0, 0.0, 26.0, BRASS);
                circle(g, 0.0, 0.0, 24.0, MARBLE_DARK);
            }
            1 => {
                rect(
                    g,
                    -12.0,
                    -12.0,
                    24.0,
                    24.0,
                    Color::new(0.10, 0.08, 0.13, 1.0),
                );
                frame(g, -12.0, -12.0, 24.0, 24.0, 2.0, BRASS);
                rect(g, -5.0, -5.0, 10.0, 10.0, BRASS);
            }
            _ => {
                for k in 0..4 {
                    let a = k as f32 * FRAC_PI_2;
                    line(
                        g,
                        a.cos() * 17.0,
                        a.sin() * 17.0,
                        a.cos() * 24.0,
                        a.sin() * 24.0,
                        2.5,
                        BRASS,
                    );
                }
            }
        }
    }

    /// 54 — the elevator lobby: the lift doors in their wall stub and the
    /// call panel beside them, up / down arrows lighting in turn and pooling
    /// amber on the floor. Layers: wall, panel, arrows.
    fn call_panel(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -30.0, -46.0, 60.0, 12.0, STEEL_DARK);
                frame(g, -30.0, -46.0, 60.0, 12.0, 1.5, TRIM);
                rect(g, -24.0, -44.0, 32.0, 8.0, CHROME); // the doors
                line(g, -8.0, -44.0, -8.0, -36.0, 1.5, PANEL);
                rect(g, -24.0, -34.0, 32.0, 2.0, alpha(CHROME, 0.5)); // threshold
            }
            1 => {
                rect(g, 12.0, -34.0, 14.0, 10.0, PANEL);
                frame(g, 12.0, -34.0, 14.0, 10.0, 1.5, TRIM);
            }
            _ => {
                let c = (time * 0.4).fract();
                let up = c < 0.42;
                let down = (0.5..0.92).contains(&c);
                let (lit, dim) = (LED_AMBER, alpha(LED_AMBER, 0.25));
                chevron(g, 16.0, -29.0, 2.5, 1.5, if up { lit } else { dim });
                line(
                    g,
                    20.5,
                    -31.0,
                    23.0,
                    -27.5,
                    1.5,
                    if down { lit } else { dim },
                );
                line(
                    g,
                    23.0,
                    -27.5,
                    25.5,
                    -31.0,
                    1.5,
                    if down { lit } else { dim },
                );
                if up || down {
                    circle(g, 19.0, -22.0, 6.0, alpha(LED_AMBER, 0.12));
                }
            }
        }
    }

    /// 55 — a velvet rope between two brass posts, the rope's bow breathing
    /// as the queue brushes it. Layers: shadow, rope (swaying), posts.
    fn velvet_rope(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                for i in 0..10 {
                    let x0 = -36.0 + i as f32 * 7.2;
                    let x1 = x0 + 7.2;
                    let f = |x: f32| 5.0 * (1.0 - (x / 36.0) * (x / 36.0));
                    line(
                        g,
                        x0 + 3.0,
                        f(x0) + 4.0,
                        x1 + 3.0,
                        f(x1) + 4.0,
                        3.0,
                        alpha(SHADOW, 0.2),
                    );
                }
            }
            1 => {
                let b = 5.0 + 1.5 * (time * 0.7).sin();
                for i in 0..10 {
                    let x0 = -36.0 + i as f32 * 7.2;
                    let x1 = x0 + 7.2;
                    let f = |x: f32| b * (1.0 - (x / 36.0) * (x / 36.0));
                    line(g, x0, f(x0), x1, f(x1), 3.5, VELVET);
                }
                circle(g, -36.0, 0.0, 2.0, BRASS);
                circle(g, 36.0, 0.0, 2.0, BRASS);
            }
            _ => {
                for &x in &[-36.0f32, 36.0] {
                    post(g, x, 0.0, 5.5, Color::new(0.55, 0.42, 0.18, 1.0), BRASS);
                }
            }
        }
    }

    /// 56 — a wall-mounted fire extinguisher: the wall stub and bracket, the
    /// red cylinder from above with its valve and hose, the fire-point mark
    /// on the floor. Layers: mount, tank, sign.
    fn extinguisher(g: &Graphics, layer: usize, _time: f32) {
        let red = Color::new(0.72, 0.14, 0.18, 1.0);
        match layer {
            0 => {
                rect(g, -16.0, -46.0, 32.0, 10.0, STEEL_DARK);
                frame(g, -16.0, -46.0, 32.0, 10.0, 1.5, TRIM);
                rect(g, -3.0, -36.0, 6.0, 4.0, STEEL);
            }
            1 => {
                shadow_circle(g, 0.0, -28.0, 9.0);
                circle(g, 0.0, -28.0, 9.0, red);
                circle(g, 0.0, -28.0, 6.5, Color::new(0.85, 0.25, 0.28, 1.0));
                circle(g, 0.0, -28.0, 3.5, CHROME); // the valve
                circle(g, 0.0, -28.0, 1.5, STEEL_DARK);
                line(
                    g,
                    3.0,
                    -30.0,
                    9.0,
                    -24.0,
                    2.0,
                    Color::new(0.08, 0.08, 0.09, 1.0),
                );
                line(
                    g,
                    9.0,
                    -24.0,
                    8.0,
                    -16.0,
                    2.0,
                    Color::new(0.08, 0.08, 0.09, 1.0),
                );
                rect(g, 6.0, -16.0, 4.0, 4.0, STEEL_DARK); // nozzle
            }
            _ => {
                rect(g, -8.0, -6.0, 16.0, 16.0, alpha(red, 0.8));
                circle(g, 0.0, 3.0, 4.0, CREAM);
                rect(g, -1.5, -4.0, 3.0, 6.0, CREAM);
            }
        }
    }

    /// 57 — a credit terminal kiosk: the pedestal with card slot and keypad,
    /// its top-face screen (header, a progress bar filling, a blinking
    /// cursor) and the screen's glow spilling toward the user. Layers: body,
    /// screen, wash.
    fn credit_kiosk(g: &Graphics, layer: usize, time: f32) {
        match layer {
            0 => {
                rect(g, -10.0, -15.0, 30.0, 40.0, SHADOW);
                rect(g, -15.0, -20.0, 30.0, 40.0, STEEL);
                frame(g, -15.0, -20.0, 30.0, 40.0, 2.0, TRIM);
                rect(g, -8.0, 2.0, 16.0, 2.0, PANEL); // card slot
                let on = blink(time, 1.0, 0.0, 0.5);
                circle(g, 10.0, 3.0, 1.5, if on { LED_GREEN } else { PANEL });
                for c in 0..3 {
                    for r in 0..4 {
                        rect(
                            g,
                            -7.0 + c as f32 * 5.0,
                            6.0 + r as f32 * 3.0,
                            3.0,
                            2.0,
                            TRIM,
                        );
                    }
                }
                rect(g, -15.0, 18.0, 30.0, 2.0, alpha(GLOW_CYAN, 0.6));
            }
            1 => {
                rect(g, -11.0, -16.0, 22.0, 14.0, PANEL);
                rect(g, -9.0, -14.0, 18.0, 2.0, alpha(GLOW_CYAN, 0.7));
                rect(g, -9.0, -9.0, 18.0, 3.0, Color::new(0.10, 0.20, 0.25, 1.0));
                rect(g, -9.0, -9.0, 18.0 * (time * 0.25).fract(), 3.0, LED_GREEN);
                rect(g, -9.0, -4.0, 12.0, 1.5, alpha(CREAM, 0.5));
                rect(g, -9.0, -1.5, 8.0, 1.5, alpha(CREAM, 0.5));
                if blink(time, 2.0, 0.0, 0.5) {
                    rect(g, 4.0, -3.0, 2.0, 2.0, CREAM);
                }
            }
            _ => wash_v(g, 0.0, 20.0, 30.0, 14.0, GLOW_CYAN, 0.14),
        }
    }

    /// 58 — a holo clock projected on the floor: the ring and ticks, the
    /// hour hand (static), the minute and second hands (spinning layers).
    /// Layers: face, hour, minute, second.
    fn wall_clock(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                ring(g, 0.0, 0.0, 34.0, 2.5, alpha(GLOW_CYAN, 0.7));
                circle(g, 0.0, 0.0, 31.0, Color::new(0.05, 0.15, 0.20, 0.35));
                for k in 0..12 {
                    let a = k as f32 * (TAU / 12.0);
                    let len = if k % 3 == 0 { 5.0 } else { 2.5 };
                    line(
                        g,
                        a.cos() * 29.0,
                        a.sin() * 29.0,
                        a.cos() * (29.0 - len),
                        a.sin() * (29.0 - len),
                        1.5,
                        alpha(GLOW_CYAN, 0.8),
                    );
                }
                circle(g, 0.0, 0.0, 2.0, CREAM);
            }
            1 => rect(g, -2.5, -18.0, 5.0, 20.0, alpha(CREAM, 0.9)),
            2 => rect(g, -1.5, -26.0, 3.0, 30.0, alpha(CREAM, 0.85)),
            _ => {
                rect(g, -0.75, -29.0, 1.5, 34.0, alpha(NEON_PINK, 0.9));
                circle(g, 0.0, 3.0, 1.5, NEON_PINK);
            }
        }
    }

    /// 59 — the welcome mat inside the doors: coir texture, a border,
    /// chevrons pointing in, some wear. A flat decal. Layers: mat, pattern.
    fn welcome_mat(g: &Graphics, layer: usize, _time: f32) {
        match layer {
            0 => {
                rect(
                    g,
                    -35.0,
                    -20.0,
                    70.0,
                    40.0,
                    Color::new(0.16, 0.14, 0.14, 1.0),
                );
                for i in 0..9 {
                    rect(
                        g,
                        -33.0,
                        -17.0 + i as f32 * 4.0,
                        66.0,
                        1.0,
                        alpha(PANEL, 0.25),
                    );
                }
                frame(
                    g,
                    -35.0,
                    -20.0,
                    70.0,
                    40.0,
                    2.5,
                    Color::new(0.34, 0.30, 0.28, 1.0),
                );
            }
            _ => {
                let accent = Color::new(1.0, 0.44, 0.38, 0.8);
                for &x in &[-18.0f32, 0.0, 18.0] {
                    chevron(g, x, 0.0, 7.0, 2.5, accent);
                }
                for k in 0..5u32 {
                    rect(
                        g,
                        -30.0 + rnd(k, 3) * 58.0,
                        -15.0 + rnd(k, 4) * 28.0,
                        3.0,
                        2.0,
                        alpha(CREAM, 0.12),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_names_are_present_and_unique() {
        assert_eq!(PROP_COUNT, PROP_NAMES.len());
        for (i, a) in PROP_NAMES.iter().enumerate() {
            assert!(!a.is_empty());
            for b in PROP_NAMES.iter().skip(i + 1) {
                assert_ne!(a, b, "duplicate prop name");
            }
        }
    }

    /// The families partition the library into contiguous id ranges, in
    /// order, and every prop maps back to the family whose range holds it.
    #[test]
    fn families_partition_the_library() {
        assert_eq!(PROP_FAMILIES[0], ("DATACENTER", 0));
        assert_eq!(
            family_range(0),
            0..24,
            "the datacenter set keeps its 24 ids"
        );
        let mut next = 0;
        for f in 0..PROP_FAMILIES.len() {
            let r = family_range(f);
            assert_eq!(
                r.start, next,
                "family {f} does not start where the last ended"
            );
            assert!(!r.is_empty(), "family {f} is empty");
            for k in r.clone() {
                assert_eq!(prop_family(k), f, "prop {k}");
            }
            next = r.end;
        }
        assert_eq!(next, PROP_COUNT);
        assert!(largest_family() >= 24 && largest_family() <= 4 * 8);
        assert_eq!(prop_kind_id(PROP_FAMILIES[1].1), "car_pod");
        assert_eq!(prop_kind_id(PROP_FAMILIES[2].1), "reception_desk");
    }

    #[test]
    fn kind_ids_are_snake_case_and_unique() {
        let ids: Vec<String> = (0..PROP_COUNT).map(prop_kind_id).collect();
        assert_eq!(ids[0], "rack_closed");
        assert_eq!(ids[9], "crac_cooler");
        for (i, a) in ids.iter().enumerate() {
            assert!(a
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
            assert!(!a.starts_with('_') && !a.ends_with('_'));
            for b in ids.iter().skip(i + 1) {
                assert_ne!(a, b, "duplicate prop id");
            }
        }
    }

    #[test]
    fn every_prop_has_layers_with_unique_names_and_sane_bounds() {
        for (kind, layers) in PROP_LAYERS.iter().enumerate() {
            assert!(!layers.is_empty(), "prop {kind} has no layers");
            assert!(
                layers.len() <= MAX_LAYERS,
                "prop {kind} has too many layers"
            );
            for (i, l) in layers.iter().enumerate() {
                assert!(!l.name.is_empty());
                assert!(
                    l.bounds.2 > 0.0 && l.bounds.3 > 0.0,
                    "prop {kind} layer {i}"
                );
                for m in layers.iter().skip(i + 1) {
                    assert_ne!(l.name, m.name, "prop {kind}: duplicate layer name");
                }
            }
        }
    }

    /// The generated settings (props/props.json -> props_data.rs) must refer
    /// to real props and layers, in library order.
    #[test]
    fn generated_settings_match_the_library() {
        for (kind, s) in PROP_SETTINGS.iter().enumerate() {
            assert_eq!(
                s.kind,
                prop_kind_id(kind),
                "props_data.rs is out of order/date"
            );
            assert!((1..=MAX_PX).contains(&s.px), "{}: px out of range", s.kind);
            for l in s.layers {
                assert!(
                    PROP_LAYERS[kind].iter().any(|d| d.name == l.name),
                    "{}: unknown layer '{}' in props.json (run `make gen-props` after renaming)",
                    s.kind,
                    l.name
                );
            }
        }
    }

    #[test]
    fn settings_json_round_trips_the_defaults() {
        let entries: Vec<(u32, [PixelMode; MAX_LAYERS])> = (0..PROP_COUNT)
            .map(|k| (prop_px(k), prop_modes(k)))
            .collect();
        let json = settings_json(&entries);
        assert!(json.starts_with("{\n  \"props\": [\n"));
        assert!(json.contains("\"kind\": \"security_cam\""));
        assert_eq!(json.matches("\"kind\"").count(), PROP_COUNT);
        assert_eq!(
            json.matches("\"name\"").count(),
            PROP_LAYERS.iter().map(|l| l.len()).sum::<usize>()
        );
    }

    #[test]
    fn rotations_and_boxes() {
        assert_eq!(LayerRot::None.angle(3.0), 0.0);
        assert!((LayerRot::Static(90.0).angle(0.0) - PI / 2.0).abs() < 1e-6);
        assert!((LayerRot::Spin { hz: 1.0 }.angle(0.25) - PI / 2.0).abs() < 1e-6);
        assert_eq!(
            snap_box((-35.0, 37.0, 70.0, 8.0), 4.0),
            (-36.0, 36.0, 72.0, 12.0)
        );
        let (x, y, w, h) = rot_box((-14.0, -14.0, 28.0, 28.0), 4.0);
        assert_eq!((x, y), (-20.0, -20.0));
        assert_eq!((w, h), (40.0, 40.0));
        assert_eq!(snap_size(75.0, 4), 75.0); // 25 texels x 3
        assert_eq!(snap_size(368.0, 4), 350.0); // 25 texels x 14
        assert_eq!(snap_size(20.0, 4), 20.0); // < 1 px per texel: unchanged
        assert_eq!(snap_size(368.0, 1), 368.0);
        assert_eq!(PixelMode::from_id("after"), Some(After));
        assert_eq!(PixelMode::from_id("x"), None);
        assert_eq!(Before.toggled(), After);
    }
}
