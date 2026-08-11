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
}
