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
        let canvas = document
            .get_element_by_id("canvas")
            .ok_or("No canvas element found")?
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| "Element is not a canvas")?;

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
        self.context.set_fill_style(&color.to_css_string().into());
        self.context
            .fill_rect(0.0, 0.0, self.width() as f64, self.height() as f64);
    }

    pub fn draw_rectangle(&self, pos: Vec2, width: f32, height: f32, color: Color) {
        self.context.set_fill_style(&color.to_css_string().into());
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
        self.context.set_stroke_style(&color.to_css_string().into());
        self.context.set_line_width(thickness as f64);
        self.context
            .stroke_rect(pos.x as f64, pos.y as f64, width as f64, height as f64);
    }

    pub fn draw_circle(&self, center: Vec2, radius: f32, color: Color) {
        self.context.set_fill_style(&color.to_css_string().into());
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
        self.context.set_stroke_style(&color.to_css_string().into());
        self.context.set_line_width(thickness as f64);
        self.context.begin_path();
        self.context.move_to(start.x as f64, start.y as f64);
        self.context.line_to(end.x as f64, end.y as f64);
        self.context.stroke();
    }

    pub fn draw_text(&self, text: &str, pos: Vec2, font_size: f32, color: Color) {
        self.context.set_fill_style(&color.to_css_string().into());
        self.context
            .set_font(&format!("{}px sans-serif", font_size));
        let _ = self.context.fill_text(text, pos.x as f64, pos.y as f64);
    }
}
