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
        self.context
            .set_font(&format!("{}px sans-serif", font_size));
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

    /// Draw a pixelated sprite (top-down humanoid)
    /// Draws a simple pixel-art character facing upward (rotation should be applied externally)
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

        // Create a simple top-down humanoid shape
        // The sprite faces "up" (negative Y) in local coordinates

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

        let dark_color = Color::new(
            body_color.r * 0.7,
            body_color.g * 0.7,
            body_color.b * 0.7,
            body_color.a,
        );

        // Head (front of character)
        self.draw_rectangle(
            Vec2::new(-pixel_size, -pixel_size * 3.0),
            pixel_size * 2.0,
            pixel_size * 2.0,
            body_color,
        );

        // Body/torso
        self.draw_rectangle(
            Vec2::new(-pixel_size * 1.5, -pixel_size),
            pixel_size * 3.0,
            pixel_size * 3.0,
            body_color,
        );

        // Arms (shoulders)
        self.draw_rectangle(
            Vec2::new(-pixel_size * 2.5, -pixel_size * 0.5),
            pixel_size,
            pixel_size * 2.0,
            dark_color,
        ); // Left arm
        self.draw_rectangle(
            Vec2::new(pixel_size * 1.5, -pixel_size * 0.5),
            pixel_size,
            pixel_size * 2.0,
            dark_color,
        ); // Right arm

        // Legs
        self.draw_rectangle(
            Vec2::new(-pixel_size * 1.0, pixel_size * 2.0),
            pixel_size,
            pixel_size * 2.0,
            dark_color,
        ); // Left leg
        self.draw_rectangle(
            Vec2::new(pixel_size * 0.0, pixel_size * 2.0),
            pixel_size,
            pixel_size * 2.0,
            dark_color,
        ); // Right leg

        // Direction indicator (small dot at head)
        if !dead {
            self.draw_rectangle(
                Vec2::new(-pixel_size * 0.5, -pixel_size * 4.5),
                pixel_size,
                pixel_size,
                Color::WHITE,
            );
        }

        self.restore();
    }
}
