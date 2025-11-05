use crate::graphics::Graphics;
use crate::math::{Color, Vec2};

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

    pub fn render(&self, graphics: &Graphics) {
        // Draw floor tiles with a grid pattern
        let tiles_x = (self.width / self.tile_size) as i32;
        let tiles_y = (self.height / self.tile_size) as i32;

        for x in 0..tiles_x {
            for y in 0..tiles_y {
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

        // Draw some walls/obstacles
        self.draw_wall(graphics, 200.0, 200.0, 400.0, 20.0);
        self.draw_wall(graphics, 200.0, 200.0, 20.0, 200.0);
        self.draw_wall(graphics, 800.0, 300.0, 20.0, 300.0);
        self.draw_wall(graphics, 400.0, 600.0, 300.0, 20.0);
    }

    fn draw_wall(&self, graphics: &Graphics, x: f32, y: f32, width: f32, height: f32) {
        graphics.draw_rectangle(
            Vec2::new(x, y),
            width,
            height,
            Color::new(80.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0),
        );
        // Border for visual depth
        graphics.draw_rectangle_lines(
            Vec2::new(x, y),
            width,
            height,
            2.0,
            Color::new(100.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0),
        );
    }
}
