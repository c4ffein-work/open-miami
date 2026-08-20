use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }

    pub const fn zero() -> Self {
        Vec2 { x: 0.0, y: 0.0 }
    }

    pub fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn normalize(&self) -> Self {
        let len = self.length();
        if len > 0.0 {
            Vec2 {
                x: self.x / len,
                y: self.y / len,
            }
        } else {
            Vec2::zero()
        }
    }

    pub fn dot(&self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    pub fn distance(&self, other: Vec2) -> f32 {
        (*self - other).length()
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, scalar: f32) -> Vec2 {
        Vec2::new(self.x * scalar, self.y * scalar)
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    fn div(self, scalar: f32) -> Vec2 {
        Vec2::new(self.x / scalar, self.y / scalar)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, other: Vec2) {
        self.x += other.x;
        self.y += other.y;
    }
}

/// A half-open range of tile indices `[min, max)` on both axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRange {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// Compute the tile indices overlapping a world-space viewport, clamped to the
/// grid `[0, tiles_x) x [0, tiles_y)`. Used to draw only visible floor tiles
/// instead of the whole level every frame.
pub fn visible_tile_range(
    view_min: Vec2,
    view_max: Vec2,
    tile_size: f32,
    tiles_x: i32,
    tiles_y: i32,
) -> TileRange {
    let min_x = (view_min.x / tile_size).floor() as i32;
    let min_y = (view_min.y / tile_size).floor() as i32;
    let max_x = (view_max.x / tile_size).ceil() as i32;
    let max_y = (view_max.y / tile_size).ceil() as i32;
    TileRange {
        min_x: min_x.clamp(0, tiles_x),
        min_y: min_y.clamp(0, tiles_y),
        max_x: max_x.clamp(0, tiles_x),
        max_y: max_y.clamp(0, tiles_y),
    }
}

/// Safety margin (world units) added around the camera's visible bounds when
/// culling sprites: covers the camera sway (drift + roll), pose overhang past
/// a sprite's nominal footprint, and one frame of camera motion.
pub const CULL_MARGIN: f32 = 64.0;

/// Whether the world-space rect `(cx, cy) ± (half_w, half_h)` intersects the
/// view rect `min..max` (pure, host-testable). Touching an edge counts as
/// visible — only a sprite FULLY outside may skip its draw commands.
pub fn rect_visible(min: Vec2, max: Vec2, cx: f32, cy: f32, half_w: f32, half_h: f32) -> bool {
    cx + half_w >= min.x && cx - half_w <= max.x && cy + half_h >= min.y && cy - half_h <= max.y
}

/// The camera's visible world rect inflated by `CULL_MARGIN`, handed to the
/// world renderers so live sprites (robots, ground guns, the boss) and placed
/// props fully outside the view can skip their commands. Culling happens in
/// WORLD space, so the `?pixel=N` world group changes nothing about it.
/// Built by `Camera::view_cull` (src/camera.rs).
#[derive(Debug, Clone, Copy)]
pub struct ViewCull {
    min: Vec2,
    max: Vec2,
}

impl ViewCull {
    /// Wrap `Camera::visible_bounds` output, inflating it by `CULL_MARGIN`.
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Self {
            min: Vec2::new(min.x - CULL_MARGIN, min.y - CULL_MARGIN),
            max: Vec2::new(max.x + CULL_MARGIN, max.y + CULL_MARGIN),
        }
    }

    /// A cull that never rejects anything (editor / gallery style callers).
    pub fn everything() -> Self {
        Self {
            min: Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
            max: Vec2::new(f32::INFINITY, f32::INFINITY),
        }
    }

    /// Whether a square footprint centred at `(cx, cy)` with half-extent
    /// `half` overlaps the inflated view (draw it) or not (skip it).
    pub fn visible(&self, cx: f32, cy: f32, half: f32) -> bool {
        rect_visible(self.min, self.max, cx, cy, half, half)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b, a: 1.0 }
    }

    pub fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    pub const RED: Color = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const GREEN: Color = Color {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };
    pub const BLUE: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const GRAY: Color = Color {
        r: 0.5,
        g: 0.5,
        b: 0.5,
        a: 1.0,
    };
    pub const YELLOW: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };

    pub fn to_css_string(&self) -> String {
        format!(
            "rgba({}, {}, {}, {})",
            (self.r * 255.0) as u8,
            (self.g * 255.0) as u8,
            (self.b * 255.0) as u8,
            self.a
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_tile_range_center() {
        // Viewport covering world x/y in [100, 260) with 50px tiles => tiles 2..6
        let range = visible_tile_range(
            Vec2::new(100.0, 100.0),
            Vec2::new(260.0, 260.0),
            50.0,
            40,
            40,
        );
        assert_eq!(range.min_x, 2);
        assert_eq!(range.min_y, 2);
        assert_eq!(range.max_x, 6); // 260/50 = 5.2 -> ceil 6
        assert_eq!(range.max_y, 6);
    }

    #[test]
    fn test_visible_tile_range_clamps_to_grid() {
        // Viewport partly off the negative edge and past the far edge
        let range = visible_tile_range(
            Vec2::new(-500.0, -500.0),
            Vec2::new(9000.0, 9000.0),
            50.0,
            40,
            40,
        );
        assert_eq!(range.min_x, 0);
        assert_eq!(range.min_y, 0);
        assert_eq!(range.max_x, 40);
        assert_eq!(range.max_y, 40);
    }

    #[test]
    fn test_visible_tile_range_culls_most_tiles() {
        // A 960x720 viewport over a 2000x2000 / 50px grid (40x40 = 1600 tiles)
        // should touch far fewer than the whole grid.
        let range = visible_tile_range(
            Vec2::new(500.0, 500.0),
            Vec2::new(1460.0, 1220.0),
            50.0,
            40,
            40,
        );
        let visible = (range.max_x - range.min_x) * (range.max_y - range.min_y);
        assert!(visible < 1600, "expected culling, drew {visible} tiles");
        assert!(visible > 0);
    }

    // ---- sprite view culling (rect_visible / ViewCull) ----

    const VMIN: Vec2 = Vec2::new(-100.0, -50.0);
    const VMAX: Vec2 = Vec2::new(100.0, 50.0);

    #[test]
    fn cull_fully_inside_is_visible() {
        assert!(rect_visible(VMIN, VMAX, 0.0, 0.0, 10.0, 10.0));
        assert!(rect_visible(VMIN, VMAX, -80.0, 30.0, 5.0, 5.0));
    }

    #[test]
    fn cull_straddling_each_edge_is_visible() {
        assert!(rect_visible(VMIN, VMAX, -105.0, 0.0, 10.0, 10.0)); // left
        assert!(rect_visible(VMIN, VMAX, 105.0, 0.0, 10.0, 10.0)); // right
        assert!(rect_visible(VMIN, VMAX, 0.0, -55.0, 10.0, 10.0)); // top
        assert!(rect_visible(VMIN, VMAX, 0.0, 55.0, 10.0, 10.0)); // bottom
                                                                  // A rect bigger than the view (containing it) must draw too.
        assert!(rect_visible(VMIN, VMAX, 0.0, 0.0, 1000.0, 1000.0));
    }

    #[test]
    fn cull_touching_an_edge_is_visible() {
        assert!(rect_visible(VMIN, VMAX, -110.0, 0.0, 10.0, 10.0)); // right edge on min.x
        assert!(rect_visible(VMIN, VMAX, 110.0, 0.0, 10.0, 10.0));
        assert!(rect_visible(VMIN, VMAX, 0.0, 60.0, 10.0, 10.0));
    }

    #[test]
    fn cull_fully_outside_each_side_is_skipped() {
        assert!(!rect_visible(VMIN, VMAX, -120.0, 0.0, 10.0, 10.0)); // left of view
        assert!(!rect_visible(VMIN, VMAX, 120.0, 0.0, 10.0, 10.0)); // right of view
        assert!(!rect_visible(VMIN, VMAX, 0.0, -70.0, 10.0, 10.0)); // above
        assert!(!rect_visible(VMIN, VMAX, 0.0, 70.0, 10.0, 10.0)); // below
        assert!(!rect_visible(VMIN, VMAX, 200.0, 200.0, 10.0, 10.0)); // diagonal
    }

    #[test]
    fn cull_view_applies_the_margin() {
        let cull = ViewCull::new(VMIN, VMAX);
        // Outside the raw bounds but inside the margin: still drawn.
        assert!(cull.visible(100.0 + CULL_MARGIN, 0.0, 1.0));
        assert!(cull.visible(0.0, -50.0 - CULL_MARGIN + 1.0, 0.0));
        // Past the margin plus the half-extent: culled.
        assert!(!cull.visible(100.0 + CULL_MARGIN + 10.1, 0.0, 10.0));
        assert!(!cull.visible(0.0, 50.0 + CULL_MARGIN + 10.1, 10.0));
        // Straddling the inflated edge: drawn.
        assert!(cull.visible(100.0 + CULL_MARGIN + 5.0, 0.0, 10.0));
    }

    #[test]
    fn cull_everything_never_skips() {
        let cull = ViewCull::everything();
        assert!(cull.visible(1e9, -1e9, 0.0));
    }
}
