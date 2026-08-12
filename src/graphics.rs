use crate::math::{Color, Vec2};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

#[cfg(target_arch = "wasm32")]
pub struct Graphics {
    context: CanvasRenderingContext2d,
    canvas: HtmlCanvasElement,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Graphics;

#[cfg(target_arch = "wasm32")]
impl Graphics {
    pub fn new() -> Result<Self, String> {
        let window = web_sys::window().ok_or("No window found")?;
        let document = window.document().ok_or("No document found")?;

        // Debug: log what we're looking for
        web_sys::console::log_1(&"Looking for canvas with id: glcanvas".into());

        let canvas = document
            .get_element_by_id("glcanvas")
            .ok_or("No canvas element found with id 'glcanvas'")?
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| "Element with id 'glcanvas' is not a canvas")?;

        // Set canvas size to window size
        let width = window
            .inner_width()
            .map_err(|_| "Failed to get window width")?
            .as_f64()
            .unwrap_or(960.0) as u32;
        let height = window
            .inner_height()
            .map_err(|_| "Failed to get window height")?
            .as_f64()
            .unwrap_or(720.0) as u32;
        canvas.set_width(width);
        canvas.set_height(height);

        let context = canvas
            .get_context("2d")
            .map_err(|_| "Failed to get 2d context")?
            .ok_or("No 2d context")?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| "Failed to cast to 2d context")?;

        Ok(Graphics { context, canvas })
    }

    pub fn width(&self) -> f32 {
        self.canvas.width() as f32
    }

    pub fn height(&self) -> f32 {
        self.canvas.height() as f32
    }

    pub fn clear(&self, color: Color) {
        self.context.set_fill_style_str(&color.to_css_string());
        self.context
            .fill_rect(0.0, 0.0, self.width() as f64, self.height() as f64);
    }

    pub fn draw_rectangle(&self, pos: Vec2, width: f32, height: f32, color: Color) {
        self.context.set_fill_style_str(&color.to_css_string());
        self.context
            .fill_rect(pos.x as f64, pos.y as f64, width as f64, height as f64);
    }

    pub fn draw_rectangle_lines(
        &self,
        pos: Vec2,
        width: f32,
        height: f32,
        thickness: f32,
        color: Color,
    ) {
        self.context.set_stroke_style_str(&color.to_css_string());
        self.context.set_line_width(thickness as f64);
        self.context
            .stroke_rect(pos.x as f64, pos.y as f64, width as f64, height as f64);
    }

    pub fn draw_circle(&self, center: Vec2, radius: f32, color: Color) {
        self.context.set_fill_style_str(&color.to_css_string());
        self.context.begin_path();
        let _ = self.context.arc(
            center.x as f64,
            center.y as f64,
            radius as f64,
            0.0,
            std::f64::consts::PI * 2.0,
        );
        self.context.fill();
    }

    pub fn draw_line(&self, start: Vec2, end: Vec2, thickness: f32, color: Color) {
        self.context.set_stroke_style_str(&color.to_css_string());
        self.context.set_line_width(thickness as f64);
        self.context.begin_path();
        self.context.move_to(start.x as f64, start.y as f64);
        self.context.line_to(end.x as f64, end.y as f64);
        self.context.stroke();
    }

    pub fn draw_text(&self, text: &str, pos: Vec2, font_size: f32, color: Color) {
        self.context.set_fill_style_str(&color.to_css_string());
        // 'GameFont' is the embedded VT323 (see index.html); falls back to
        // monospace if it somehow failed to load.
        self.context
            .set_font(&format!("{}px 'GameFont', monospace", font_size));
        let _ = self.context.fill_text(text, pos.x as f64, pos.y as f64);
    }

    /// Draw a filled arc (pie slice) for vision cones
    pub fn draw_arc(
        &self,
        center: Vec2,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    ) {
        self.context.set_fill_style_str(&color.to_css_string());
        self.context.begin_path();
        self.context.move_to(center.x as f64, center.y as f64);
        let _ = self.context.arc(
            center.x as f64,
            center.y as f64,
            radius as f64,
            start_angle as f64,
            end_angle as f64,
        );
        self.context.close_path();
        self.context.fill();
    }

    /// Save the current transformation state
    pub fn save(&self) {
        self.context.save();
    }

    /// Restore the previous transformation state
    pub fn restore(&self) {
        self.context.restore();
    }

    /// Translate the canvas
    pub fn translate(&self, x: f32, y: f32) {
        let _ = self.context.translate(x as f64, y as f64);
    }

    /// Rotate the canvas around the current origin
    pub fn rotate(&self, angle: f32) {
        let _ = self.context.rotate(angle as f64);
    }

    /// Draw the shoggoth boss: a writhing dark mass. While `enraged` is false it
    /// wears a friendly yellow smiley mask; once the mask cracks off it shows the
    /// horror beneath (red eyes, mask shards flung outward).
    pub fn draw_shoggoth(&self, center: Vec2, radius: f32, enraged: bool) {
        let dark = Color::new(0.11, 0.04, 0.15, 1.0);
        let dark2 = Color::new(0.22, 0.09, 0.28, 1.0);
        // Phase derived from position so the mass subtly writhes as it moves.
        let ph = center.x * 0.03 + center.y * 0.03;

        // Blobby outer lobes, then the core on top.
        for k in 0..6 {
            let a = ph + k as f32 * (std::f32::consts::PI * 2.0 / 6.0);
            let off = radius * (0.62 + 0.12 * (ph + k as f32).sin());
            let lc = Vec2::new(center.x + a.cos() * off, center.y + a.sin() * off);
            self.draw_circle(lc, radius * 0.5, dark2);
        }
        self.draw_circle(center, radius, dark);

        if !enraged {
            // Friendly smiley mask.
            let mr = radius * 0.74;
            self.draw_circle(center, mr, Color::new(1.0, 0.84, 0.12, 1.0));
            let eye_dx = mr * 0.4;
            let eye_y = center.y - mr * 0.12;
            self.draw_circle(Vec2::new(center.x - eye_dx, eye_y), mr * 0.14, Color::BLACK);
            self.draw_circle(Vec2::new(center.x + eye_dx, eye_y), mr * 0.14, Color::BLACK);
            // Upturned smile from line segments.
            let sy = center.y + mr * 0.12;
            let pts = [(-0.5, 0.0), (-0.22, 0.3), (0.22, 0.3), (0.5, 0.0)];
            for w in pts.windows(2) {
                let a = Vec2::new(center.x + w[0].0 * mr, sy + w[0].1 * mr);
                let b = Vec2::new(center.x + w[1].0 * mr, sy + w[1].1 * mr);
                self.draw_line(a, b, 3.0, Color::BLACK);
            }
        } else {
            // Mask cracked off: writhing tentacles lash out from the mass...
            let tentacle = Color::new(0.17, 0.06, 0.22, 1.0);
            for k in 0..7 {
                let base = ph * 1.3 + k as f32 * (std::f32::consts::PI * 2.0 / 7.0);
                let (mut px, mut py, mut ang) = (center.x, center.y, base);
                for seg in 0..4 {
                    let len = radius * (0.55 - seg as f32 * 0.09);
                    ang += ((ph + k as f32) * 1.7 + seg as f32).sin() * 0.7;
                    let nx = px + ang.cos() * len;
                    let ny = py + ang.sin() * len;
                    self.draw_line(
                        Vec2::new(px, py),
                        Vec2::new(nx, ny),
                        (8 - seg * 2).max(1) as f32,
                        tentacle,
                    );
                    px = nx;
                    py = ny;
                }
            }
            // ...and the shoggoth stares back.
            let red = Color::new(1.0, 0.1, 0.15, 1.0);
            for k in 0..7 {
                let a = ph * 1.7 + k as f32 * 0.9;
                let rr = radius * (0.15 + 0.5 * ((ph + k as f32) * 1.3).sin().abs());
                let ec = Vec2::new(center.x + a.cos() * rr, center.y + a.sin() * rr);
                self.draw_circle(ec, radius * 0.1, red);
            }
            // Yellow mask shards flung outward.
            let shard = Color::new(1.0, 0.84, 0.12, 1.0);
            for k in 0..5 {
                let a = ph + k as f32 * 1.35;
                let d = radius * 1.15;
                let sc = Vec2::new(center.x + a.cos() * d, center.y + a.sin() * d);
                self.draw_rectangle(Vec2::new(sc.x - 3.0, sc.y - 3.0), 6.0, 6.0, shard);
            }
        }
    }

    /// Draw a pixelated sprite (top-down robot)
    /// Draws a small pixel-art bot facing upward (rotation should be applied externally):
    /// squared body with treads, a squared head, a short antenna, and a single glowing
    /// visor "eye" pointing forward. Goes dark and prone-looking when `dead` is true.
    pub fn draw_pixelated_sprite(
        &self,
        center: Vec2,
        rotation: f32,
        base_color: Color,
        dead: bool,
    ) {
        self.save();

        // Translate to center and rotate
        self.translate(center.x, center.y);
        self.rotate(rotation);

        let pixel_size = 3.0; // Size of each "pixel"

        // Top-down robot. The bot faces "up" (negative Y) in local coordinates.

        // Downed/stunned bots go dark, like a powered-off chassis.
        let body_color = if dead {
            Color::new(
                base_color.r * 0.4,
                base_color.g * 0.4,
                base_color.b * 0.4,
                base_color.a,
            )
        } else {
            base_color
        };

        // Darker plating for treads / trim.
        let dark_color = Color::new(
            body_color.r * 0.6,
            body_color.g * 0.6,
            body_color.b * 0.6,
            body_color.a,
        );

        // Cream-ish highlight (the makers' accent), dimmed when dead.
        let accent = if dead {
            Color::new(0.55, 0.53, 0.48, base_color.a)
        } else {
            Color::new(0.96, 0.93, 0.86, base_color.a)
        };

        // Treads / drive units running down each side of the chassis.
        self.draw_rectangle(
            Vec2::new(-pixel_size * 2.5, -pixel_size * 1.0),
            pixel_size,
            pixel_size * 4.0,
            dark_color,
        ); // Left tread
        self.draw_rectangle(
            Vec2::new(pixel_size * 1.5, -pixel_size * 1.0),
            pixel_size,
            pixel_size * 4.0,
            dark_color,
        ); // Right tread

        // Main chassis (body).
        self.draw_rectangle(
            Vec2::new(-pixel_size * 1.5, -pixel_size * 1.0),
            pixel_size * 3.0,
            pixel_size * 4.0,
            body_color,
        );

        // Chest core light (small accent panel).
        self.draw_rectangle(
            Vec2::new(-pixel_size * 0.5, pixel_size * 0.5),
            pixel_size,
            pixel_size,
            accent,
        );

        // Squared head, slightly narrower than the chassis, at the front.
        self.draw_rectangle(
            Vec2::new(-pixel_size * 1.0, -pixel_size * 3.0),
            pixel_size * 2.0,
            pixel_size * 2.0,
            body_color,
        );

        // Short antenna poking forward from the head.
        self.draw_rectangle(
            Vec2::new(-pixel_size * 0.5, -pixel_size * 4.0),
            pixel_size,
            pixel_size,
            dark_color,
        );

        // Glowing visor "eye" = forward direction indicator.
        // Bright cream when alive; dead bots keep a dim, dark visor (no glow).
        let visor_color = if dead { dark_color } else { accent };
        self.draw_rectangle(
            Vec2::new(-pixel_size * 0.75, -pixel_size * 2.75),
            pixel_size * 1.5,
            pixel_size * 0.75,
            visor_color,
        );
        // Antenna tip glow, only when powered.
        if !dead {
            self.draw_rectangle(
                Vec2::new(-pixel_size * 0.5, -pixel_size * 4.5),
                pixel_size,
                pixel_size * 0.5,
                accent,
            );
        }

        self.restore();
    }
}
