use crate::graphics::Graphics;
use crate::math::Vec2;

// The pure culling predicate + rect live in src/math.rs so their tests run on
// the host (this module is wasm-only); re-exported here for the renderers.
pub use crate::math::{rect_visible, ViewCull, CULL_MARGIN};

/// Default zoom: how many screen pixels one world unit covers. >1 pulls the
/// camera in closer to the characters.
pub const DEFAULT_ZOOM: f32 = 1.6;

/// The viewport the game was tuned at: `set_viewport` scales the zoom from
/// it by the geometric mean of the two axes (~constant visible AREA whatever
/// the window size or aspect — a 2:1 window trades height of view for width
/// instead of simply seeing more), clamped so the graphics never shrink
/// below readability on a small window nor balloon on a huge one. Inside the
/// clamp band, what you can see stays fair; at the band's edges, legibility
/// wins and the window sees less (or more) of the world instead.
pub const REF_VIEW_W: f32 = 960.0;
pub const REF_VIEW_H: f32 = 720.0;
pub const ZOOM_SCALE_MIN: f32 = 0.75;
pub const ZOOM_SCALE_MAX: f32 = 1.5;

/// How far (in screen px, before zoom) the view may be pushed toward the mouse
/// while Shift is held — a look-ahead so you can peek down a corridor.
pub const LOOK_MAX_PX: f32 = 260.0;
/// Fraction of the mouse's offset from screen centre that becomes look-ahead.
pub const LOOK_FACTOR: f32 = 0.55;
/// Look-ahead easing rate (per second); higher snaps faster.
pub const LOOK_EASE: f32 = 9.0;

/// Camera sway: a barely-there roll of the whole view, like a slow breath /
/// slightly drunk hand-held feel. Amplitude in radians (0.35°) and rate in Hz.
pub const SWAY_ROLL_RAD: f32 = 0.0061;
pub const SWAY_ROLL_HZ: f32 = 0.11;
/// A matching sub-pixel drift so the roll doesn't read as pure rotation.
pub const SWAY_DRIFT_PX: f32 = 2.5;
pub const SWAY_DRIFT_HZ: f32 = 0.07;

pub struct Camera {
    /// World point the camera follows (the player).
    pub target: Vec2,
    /// Current smoothed look-ahead offset, in WORLD units, added to `target`.
    pub look: Vec2,
    /// Scenario `look_at` offset (world units): pulls the focus toward a
    /// point of interest by its weight (see `set_cinematic`).
    pub cine: Vec2,
    /// World-units -> screen-px scale.
    pub zoom: f32,
    /// Current sway (set by `update_sway`): screen-space drift + roll.
    sway_dx: f32,
    sway_dy: f32,
    sway_roll: f32,
    canvas_width: f32,
    canvas_height: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec2::zero(),
            look: Vec2::zero(),
            cine: Vec2::zero(),
            zoom: DEFAULT_ZOOM,
            sway_dx: 0.0,
            sway_dy: 0.0,
            sway_roll: 0.0,
            canvas_width: 960.0,
            canvas_height: 720.0,
        }
    }
}

impl Camera {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the real canvas size so apply()/screen_to_world()/visible_bounds()
    /// all agree even when the window isn't 960x720, and derive the zoom from
    /// it (see `REF_VIEW_W`): bigger windows draw the world bigger rather than
    /// revealing arbitrarily much of it, until legibility caps out.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.canvas_width = width;
        self.canvas_height = height;
        let area_scale = ((width * height) / (REF_VIEW_W * REF_VIEW_H)).sqrt();
        self.zoom = DEFAULT_ZOOM * area_scale.clamp(ZOOM_SCALE_MIN, ZOOM_SCALE_MAX);
    }

    pub fn follow_player(&mut self, player_pos: Vec2) {
        self.target = player_pos;
    }

    /// Ease the look-ahead toward `mouse_screen` (when `active`, i.e. Shift is
    /// held) or back to centre. Uses the mouse's SCREEN offset from centre so it
    /// doesn't feed back through its own transform.
    pub fn update_look(&mut self, mouse_screen: Vec2, active: bool, dt: f32) {
        let goal = if active {
            let cx = self.canvas_width / 2.0;
            let cy = self.canvas_height / 2.0;
            let mut ox = (mouse_screen.x - cx) * LOOK_FACTOR;
            let mut oy = (mouse_screen.y - cy) * LOOK_FACTOR;
            let len = (ox * ox + oy * oy).sqrt();
            if len > LOOK_MAX_PX {
                ox *= LOOK_MAX_PX / len;
                oy *= LOOK_MAX_PX / len;
            }
            // screen px -> world units
            Vec2::new(ox / self.zoom, oy / self.zoom)
        } else {
            Vec2::zero()
        };
        let k = (dt * LOOK_EASE).clamp(0.0, 1.0);
        self.look.x += (goal.x - self.look.x) * k;
        self.look.y += (goal.y - self.look.y) * k;
    }

    /// Scenario `look_at`: `Some((point, weight))` moves the focus toward the
    /// world `point` by `weight` (0 = on the player, 1 = on the point); the
    /// scenario eases the weight in and out. `None` clears it.
    pub fn set_cinematic(&mut self, focus: Option<(Vec2, f32)>) {
        self.cine = match focus {
            Some((p, w)) => {
                let w = w.clamp(0.0, 1.0);
                Vec2::new((p.x - self.target.x) * w, (p.y - self.target.y) * w)
            }
            None => Vec2::zero(),
        };
    }

    /// The world point that sits at the centre of the screen.
    fn focus(&self) -> Vec2 {
        Vec2::new(
            self.target.x + self.look.x + self.cine.x,
            self.target.y + self.look.y + self.cine.y,
        )
    }

    /// Advance the sway for this frame; `time` in seconds. Called once per
    /// frame before `apply()` so apply/screen_to_world share the same values.
    pub fn update_sway(&mut self, time: f32) {
        let two_pi = std::f32::consts::TAU;
        self.sway_roll = (time * SWAY_ROLL_HZ * two_pi).sin() * SWAY_ROLL_RAD;
        self.sway_dx = (time * SWAY_DRIFT_HZ * two_pi).sin() * SWAY_DRIFT_PX;
        self.sway_dy = (time * SWAY_DRIFT_HZ * two_pi * 1.37 + 1.1).cos() * SWAY_DRIFT_PX;
    }

    pub fn apply(&self, graphics: &Graphics) {
        graphics.save();
        // screen = centre + drift + R(roll) * (world - focus) * zoom
        let f = self.focus();
        graphics.translate(
            self.canvas_width / 2.0 + self.sway_dx,
            self.canvas_height / 2.0 + self.sway_dy,
        );
        graphics.rotate(self.sway_roll);
        graphics.scale(self.zoom, self.zoom);
        graphics.translate(-f.x, -f.y);
    }

    pub fn reset(&self, graphics: &Graphics) {
        graphics.restore();
    }

    /// World-space bounds currently visible on screen. Matches `apply()`.
    pub fn visible_bounds(&self, screen_width: f32, screen_height: f32) -> (Vec2, Vec2) {
        let f = self.focus();
        let half_w = screen_width / (2.0 * self.zoom);
        let half_h = screen_height / (2.0 * self.zoom);
        (
            Vec2::new(f.x - half_w, f.y - half_h),
            Vec2::new(f.x + half_w, f.y + half_h),
        )
    }

    /// The visible bounds inflated by `CULL_MARGIN`, packaged for the world
    /// renderers to skip off-screen sprites (see `ViewCull`).
    pub fn view_cull(&self, screen_width: f32, screen_height: f32) -> ViewCull {
        let (min, max) = self.visible_bounds(screen_width, screen_height);
        ViewCull::new(min, max)
    }

    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        // Exact inverse of apply(): undo drift, roll, zoom, then re-add focus.
        let f = self.focus();
        let px = screen_pos.x - (self.canvas_width / 2.0 + self.sway_dx);
        let py = screen_pos.y - (self.canvas_height / 2.0 + self.sway_dy);
        let (sn, cs) = (-self.sway_roll).sin_cos();
        let rx = px * cs - py * sn;
        let ry = px * sn + py * cs;
        Vec2::new(f.x + rx / self.zoom, f.y + ry / self.zoom)
    }
}
