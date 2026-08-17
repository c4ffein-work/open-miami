use crate::graphics::Graphics;
use crate::math::{visible_tile_range, Color, Vec2};
use crate::scenario::Surface;

pub struct Level {
    width: f32,
    height: f32,
    tile_size: f32,
    /// The floor's ground rendering (`FloorDef::surface`).
    surface: Surface,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            width: 2000.0,
            height: 2000.0,
            tile_size: 50.0,
            surface: Surface::Checker,
        }
    }
}

/// Cheap deterministic per-tile hash in `0..1` (tone variation).
fn tile_noise(x: i32, y: i32, salt: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ salt;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

impl Level {
    pub fn new() -> Self {
        Self::default()
    }

    /// Select the ground rendering (called when a floor loads).
    pub fn set_surface(&mut self, surface: Surface) {
        self.surface = surface;
    }

    pub fn surface(&self) -> Surface {
        self.surface
    }

    /// Render the floor. Only tiles overlapping `view_min..view_max`
    /// (world-space camera bounds) are drawn, so cost scales with the screen
    /// size rather than the whole 2000x2000 level.
    ///
    /// `tint` is an optional colour blended over the floor (used for the kill
    /// flash: the background strobes while walls and actors stay untouched);
    /// its alpha is the blend amount.
    ///
    /// Every surface is plain rectangles (one base rect per tile plus a few
    /// thin seam / grid rects), so it batches like the checker floor did and
    /// stays compatible with the `?pixel=N` world pixel group.
    pub fn render(&self, graphics: &Graphics, view_min: Vec2, view_max: Vec2, tint: Option<Color>) {
        let tiles_x = (self.width / self.tile_size) as i32;
        let tiles_y = (self.height / self.tile_size) as i32;
        let ts = self.tile_size;

        let range = visible_tile_range(view_min, view_max, ts, tiles_x, tiles_y);

        let mix = |c: Color| -> Color {
            match tint {
                Some(t) => Color::new(
                    c.r + (t.r - c.r) * t.a,
                    c.g + (t.g - c.g) * t.a,
                    c.b + (t.b - c.b) * t.a,
                    1.0,
                ),
                None => c,
            }
        };
        let grey = |v: f32, a: f32| Color::new(v, v, v, a);

        for x in range.min_x..range.max_x {
            for y in range.min_y..range.max_y {
                let origin = Vec2::new(x as f32 * ts, y as f32 * ts);
                match self.surface {
                    Surface::Checker => {
                        let color = mix(if (x + y) % 2 == 0 {
                            Color::new(40.0 / 255.0, 35.0 / 255.0, 45.0 / 255.0, 1.0)
                        } else {
                            Color::new(35.0 / 255.0, 30.0 / 255.0, 40.0 / 255.0, 1.0)
                        });
                        graphics.draw_rectangle(origin, ts, ts, color);
                    }
                    Surface::Asphalt => {
                        // Dark blue-black with a subtle per-tile tone wobble,
                        // plus an occasional lighter patch (worn / wet spots).
                        let n = tile_noise(x, y, 0xA5F);
                        let v = 0.075 + n * 0.022;
                        let base = mix(Color::new(v, v + 0.004, v + 0.014, 1.0));
                        graphics.draw_rectangle(origin, ts, ts, base);
                        if n > 0.86 {
                            let m = tile_noise(x, y, 0x77);
                            let px = ts * (0.15 + 0.5 * m);
                            let py = ts * (0.15 + 0.5 * (1.0 - m));
                            graphics.draw_rectangle(
                                Vec2::new(origin.x + px, origin.y + py),
                                ts * 0.3,
                                ts * 0.22,
                                mix(Color::new(v + 0.02, v + 0.024, v + 0.036, 1.0)),
                            );
                        }
                    }
                    Surface::Marble => {
                        // Pale two-tone slabs — pale for THIS palette (the
                        // checker floor sits at ~0.15), muted enough that the
                        // white HUD and the bots still read — with thin darker
                        // seams on the right / bottom edges.
                        let n = tile_noise(x, y, 0x3AB);
                        let (r, g, b) = if (x + y) % 2 == 0 {
                            (0.385, 0.37, 0.40)
                        } else {
                            (0.34, 0.325, 0.355)
                        };
                        let d = (n - 0.5) * 0.025;
                        graphics.draw_rectangle(
                            origin,
                            ts,
                            ts,
                            mix(Color::new(r + d, g + d, b + d, 1.0)),
                        );
                        let seam = mix(Color::new(0.235, 0.22, 0.25, 1.0));
                        graphics.draw_rectangle(
                            Vec2::new(origin.x + ts - 1.0, origin.y),
                            1.0,
                            ts,
                            seam,
                        );
                        graphics.draw_rectangle(
                            Vec2::new(origin.x, origin.y + ts - 1.0),
                            ts,
                            1.0,
                            seam,
                        );
                        // A faint vein now and then.
                        if n > 0.8 {
                            let m = tile_noise(x, y, 0x51);
                            graphics.draw_rectangle(
                                Vec2::new(origin.x + ts * 0.2 * m, origin.y + ts * (0.2 + 0.6 * m)),
                                ts * (0.4 + 0.4 * m),
                                1.0,
                                mix(Color::new(0.30, 0.285, 0.31, 1.0)),
                            );
                        }
                    }
                    Surface::Concrete => {
                        // Mid grey with tone variation and expansion joints
                        // every 4 tiles (200 world units).
                        let n = tile_noise(x, y, 0xC0C);
                        let v = 0.27 + n * 0.035;
                        graphics.draw_rectangle(origin, ts, ts, mix(grey(v, 1.0)));
                        let joint = mix(grey(0.17, 1.0));
                        if x % 4 == 3 {
                            graphics.draw_rectangle(
                                Vec2::new(origin.x + ts - 2.0, origin.y),
                                2.0,
                                ts,
                                joint,
                            );
                        }
                        if y % 4 == 3 {
                            graphics.draw_rectangle(
                                Vec2::new(origin.x, origin.y + ts - 2.0),
                                ts,
                                2.0,
                                joint,
                            );
                        }
                    }
                    Surface::Grating => {
                        // Dark deck with a fine grid (12.5 px cells): the void
                        // shows through the mesh.
                        graphics.draw_rectangle(
                            origin,
                            ts,
                            ts,
                            mix(Color::new(0.055, 0.05, 0.075, 1.0)),
                        );
                        let bar = mix(Color::new(0.16, 0.15, 0.20, 1.0));
                        let step = ts / 4.0;
                        for i in 0..4 {
                            let o = i as f32 * step;
                            graphics.draw_rectangle(
                                Vec2::new(origin.x + o, origin.y),
                                1.0,
                                ts,
                                bar,
                            );
                            graphics.draw_rectangle(
                                Vec2::new(origin.x, origin.y + o),
                                ts,
                                1.0,
                                bar,
                            );
                        }
                    }
                }
            }
        }

        // Walls are now rendered from the World via render_walls()
    }
}
