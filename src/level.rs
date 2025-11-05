use macroquad::prelude::*;

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

    pub fn render(&self) {
        // Draw floor tiles with a grid pattern
        let tiles_x = (self.width / self.tile_size) as i32;
        let tiles_y = (self.height / self.tile_size) as i32;

        for x in 0..tiles_x {
            for y in 0..tiles_y {
                let color = if (x + y) % 2 == 0 {
                    Color::from_rgba(40, 35, 45, 255)
                } else {
                    Color::from_rgba(35, 30, 40, 255)
                };

                draw_rectangle(
                    x as f32 * self.tile_size,
                    y as f32 * self.tile_size,
                    self.tile_size,
                    self.tile_size,
                    color,
                );
            }
        }

        // Draw some walls/obstacles
        self.draw_wall(200.0, 200.0, 400.0, 20.0);
        self.draw_wall(200.0, 200.0, 20.0, 200.0);
        self.draw_wall(800.0, 300.0, 20.0, 300.0);
        self.draw_wall(400.0, 600.0, 300.0, 20.0);
    }

    fn draw_wall(&self, x: f32, y: f32, width: f32, height: f32) {
        draw_rectangle(x, y, width, height, Color::from_rgba(80, 60, 70, 255));
        // Border for visual depth
        draw_rectangle_lines(x, y, width, height, 2.0, Color::from_rgba(100, 80, 90, 255));
    }
}
