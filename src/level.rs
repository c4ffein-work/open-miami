use crate::graphics::Graphics;
use crate::math::{visible_tile_range, Color, Vec2};

pub struct Level {
    width: f32,
    height: f32,
    tile_size: f32,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            width: 2000.0,
            height: 2000.0,
            tile_size: 50.0,
        }
    }
}

impl Level {
    pub fn new() -> Self {
        Self::default()
    }

    /// Render the floor grid. Only tiles overlapping `view_min..view_max`
    /// (world-space camera bounds) are drawn, so cost scales with the screen
    /// size rather than the whole 2000x2000 level.
    pub fn render(&self, graphics: &Graphics, view_min: Vec2, view_max: Vec2) {
        // Draw floor tiles with a grid pattern
        let tiles_x = (self.width / self.tile_size) as i32;
        let tiles_y = (self.height / self.tile_size) as i32;

        let range = visible_tile_range(view_min, view_max, self.tile_size, tiles_x, tiles_y);

        for x in range.min_x..range.max_x {
            for y in range.min_y..range.max_y {
                let color = if (x + y) % 2 == 0 {
                    Color::new(40.0 / 255.0, 35.0 / 255.0, 45.0 / 255.0, 1.0)
                } else {
                    Color::new(35.0 / 255.0, 30.0 / 255.0, 40.0 / 255.0, 1.0)
                };

                graphics.draw_rectangle(
                    Vec2::new(x as f32 * self.tile_size, y as f32 * self.tile_size),
                    self.tile_size,
                    self.tile_size,
                    color,
                );
            }
        }

        // Walls are now rendered from the World via render_walls()
    }
}
