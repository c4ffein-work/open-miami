use crate::math::{Color, Vec2};
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::HtmlCanvasElement;

// The renderer lives entirely in JS/WebGL (see renderer.js). Rust describes
// each frame as a flat f32 command stream plus a text arena, and hands both to
// `window.frameRender` once per frame — a single wasm->JS boundary crossing;
// the &[f32] slice is passed as a zero-copy Float32Array view into wasm memory.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = frameRender)]
    fn frame_render(cmds: &[f32], texts: &str);
}

// Command opcodes. Each command is the opcode followed by its fixed number of
// f32 arguments. renderer.js holds the mirror of this table — keep in sync.
#[cfg(target_arch = "wasm32")]
mod op {
    pub const CLEAR: f32 = 0.0; // r g b a
    pub const RECT: f32 = 1.0; // x y w h  r g b a
    pub const RECT_LINES: f32 = 2.0; // x y w h thickness  r g b a
    pub const CIRCLE: f32 = 3.0; // x y radius  r g b a
    pub const LINE: f32 = 4.0; // x1 y1 x2 y2 thickness  r g b a
    pub const ARC: f32 = 5.0; // x y radius a0 a1  r g b a  (filled pie)
    pub const TEXT: f32 = 6.0; // textIdx x y size  r g b a  (left/baseline)
    pub const SAVE: f32 = 7.0; //
    pub const RESTORE: f32 = 8.0; //
    pub const TRANSLATE: f32 = 9.0; // x y
    pub const ROTATE: f32 = 10.0; // angle
    pub const ROBOT: f32 = 11.0; // colorIdx poseIdx weaponIdx x y angle sizePx time
    pub const SCALE: f32 = 12.0; // sx sy
    pub const SHOGGOTH: f32 = 13.0; // x y sizePx heading reveal time
    pub const POSTFX: f32 = 14.0; // kind t r g b  (full-screen post pass over the whole frame)
    pub const PIX_BEGIN: f32 = 15.0; // px w h  (open a pixel-art group: rasterize at art resolution)
    pub const PIX_END: f32 = 16.0; // x y  (close it: nearest-upscale the group at (x, y))
}

/// Separator between entries in the per-frame text arena. renderer.js splits
/// on the same character; it can never appear in game text.
#[cfg(target_arch = "wasm32")]
const TEXT_SEP: char = '\u{1f}';

#[cfg(target_arch = "wasm32")]
pub struct Graphics {
    canvas: HtmlCanvasElement,
    // Interior mutability keeps the draw API `&self`, matching the previous
    // canvas-context backend so no call site changes.
    cmds: RefCell<Vec<f32>>,
    texts: RefCell<String>,
    text_count: RefCell<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Graphics;

#[cfg(target_arch = "wasm32")]
impl Graphics {
    pub fn new() -> Result<Self, String> {
        let window = web_sys::window().ok_or("No window found")?;
        let document = window.document().ok_or("No document found")?;

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

        // The WebGL context is created and owned by renderer.js; Rust never
        // touches the canvas beyond sizing it.
        Ok(Graphics {
            canvas,
            cmds: RefCell::new(Vec::with_capacity(4096)),
            texts: RefCell::new(String::new()),
            text_count: RefCell::new(0),
        })
    }

    pub fn width(&self) -> f32 {
        self.canvas.width() as f32
    }

    pub fn height(&self) -> f32 {
        self.canvas.height() as f32
    }

    /// Hand the accumulated frame to the JS renderer and reset for the next
    /// one. Called once at the end of every game-loop tick.
    pub fn flush(&self) {
        let mut cmds = self.cmds.borrow_mut();
        let mut texts = self.texts.borrow_mut();
        frame_render(&cmds, &texts);
        cmds.clear();
        texts.clear();
        *self.text_count.borrow_mut() = 0;
    }

    fn push(&self, vals: &[f32]) {
        self.cmds.borrow_mut().extend_from_slice(vals);
    }

    pub fn clear(&self, color: Color) {
        self.push(&[op::CLEAR, color.r, color.g, color.b, color.a]);
    }

    pub fn draw_rectangle(&self, pos: Vec2, width: f32, height: f32, color: Color) {
        self.push(&[
            op::RECT,
            pos.x,
            pos.y,
            width,
            height,
            color.r,
            color.g,
            color.b,
            color.a,
        ]);
    }

    pub fn draw_rectangle_lines(
        &self,
        pos: Vec2,
        width: f32,
        height: f32,
        thickness: f32,
        color: Color,
    ) {
        self.push(&[
            op::RECT_LINES,
            pos.x,
            pos.y,
            width,
            height,
            thickness,
            color.r,
            color.g,
            color.b,
            color.a,
        ]);
    }

    pub fn draw_circle(&self, center: Vec2, radius: f32, color: Color) {
        self.push(&[
            op::CIRCLE,
            center.x,
            center.y,
            radius,
            color.r,
            color.g,
            color.b,
            color.a,
        ]);
    }

    pub fn draw_line(&self, start: Vec2, end: Vec2, thickness: f32, color: Color) {
        self.push(&[
            op::LINE,
            start.x,
            start.y,
            end.x,
            end.y,
            thickness,
            color.r,
            color.g,
            color.b,
            color.a,
        ]);
    }

    pub fn draw_text(&self, text: &str, pos: Vec2, font_size: f32, color: Color) {
        let idx = {
            let mut texts = self.texts.borrow_mut();
            let mut count = self.text_count.borrow_mut();
            if *count > 0 {
                texts.push(TEXT_SEP);
            }
            texts.push_str(text);
            let idx = *count;
            *count += 1;
            idx
        };
        self.push(&[
            op::TEXT,
            idx as f32,
            pos.x,
            pos.y,
            font_size,
            color.r,
            color.g,
            color.b,
            color.a,
        ]);
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
        self.push(&[
            op::ARC,
            center.x,
            center.y,
            radius,
            start_angle,
            end_angle,
            color.r,
            color.g,
            color.b,
            color.a,
        ]);
    }

    /// Save the current transformation state
    pub fn save(&self) {
        self.push(&[op::SAVE]);
    }

    /// Restore the previous transformation state
    pub fn restore(&self) {
        self.push(&[op::RESTORE]);
    }

    /// Translate the canvas
    pub fn translate(&self, x: f32, y: f32) {
        self.push(&[op::TRANSLATE, x, y]);
    }

    /// Rotate the canvas around the current origin
    pub fn rotate(&self, angle: f32) {
        self.push(&[op::ROTATE, angle]);
    }

    /// Scale the canvas around the current origin (camera zoom).
    pub fn scale(&self, sx: f32, sy: f32) {
        self.push(&[op::SCALE, sx, sy]);
    }

    /// Draw a live-rendered 3D robot sprite. The JS renderer runs the
    /// robot-core 3D->2D pipeline for the requested (color, pose, weapon) at
    /// the continuous animation time `time` — every frame, no caching — and
    /// draws it as a rotated quad of `size_px` px.
    /// Indices follow renderer.js tables:
    ///   color:  0 coral, 1 red, 2 violet, 3 magenta
    ///   pose:   0 idle, 1 walk, 2 shoot, 3 hit
    ///   weapon: 0 fist, 1 pistol, 2 machinegun, 3 shotgun
    #[allow(clippy::too_many_arguments)]
    pub fn draw_robot(
        &self,
        color_idx: u32,
        pose_idx: u32,
        weapon_idx: u32,
        center: Vec2,
        angle: f32,
        size_px: f32,
        time: f32,
    ) {
        self.push(&[
            op::ROBOT,
            color_idx as f32,
            pose_idx as f32,
            weapon_idx as f32,
            center.x,
            center.y,
            angle,
            size_px,
            time,
        ]);
    }

    /// Draw the LIVE 3D shoggoth boss. The JS renderer runs the shoggoth-core
    /// 3D->2D pipeline (mass, smiley mask, tentacles + dot eyes) at continuous
    /// time `time` — every frame, no caching — into a scratch tile and draws it
    /// as an axis-aligned quad of `size_px` px centered on `center`.
    ///   heading: radians in screen convention (0 = +x, PI/2 = +y/down); the
    ///            mask leans toward it (the direction the boss is moving)
    ///   reveal:  0..1 mask-off progress — 0 = mask intact, (0,1) = the mask
    ///            cracking and being consumed inward while the tentacles grow,
    ///            1 = raw enraged form
    pub fn draw_shoggoth_live(
        &self,
        center: Vec2,
        size_px: f32,
        heading: f32,
        reveal: f32,
        time: f32,
    ) {
        self.push(&[
            op::SHOGGOTH,
            center.x,
            center.y,
            size_px,
            heading,
            reveal,
            time,
        ]);
    }

    /// Request a full-screen post-processing pass over THIS frame. When the
    /// command is present anywhere in a frame's stream, renderer.js renders
    /// the whole frame into an offscreen framebuffer and draws it through the
    /// post shader `kind` with strength `t` (0..1) toward colour `(r, g, b)`.
    /// The kinds are a menu of Hotline-Miami-flavoured looks (the mirror of
    /// this table lives in renderer.js — keep in sync):
    ///   kind 0 = BLUR-OUT: a growing multi-tap blur + dissolve toward the
    ///            colour, with synthwave scanlines and noise grain (t = 0
    ///            untouched, 1 = fully dissolved into the colour; the ending)
    ///   kind 1 = SYNTHWAVE CRT: scanlines, slight chromatic split, vignette
    ///            and grain; the colour tints the vignette (the credits)
    ///   kind 2 = VHS TAPE: rolling tracking band, per-scanline jitter,
    ///            chroma bleed, washed colour, dropout streaks
    ///   kind 3 = DRUNK SWAY: slow rotation/zoom breathing, wavy warp,
    ///            orbiting double-vision ghost, hue drift
    ///   kind 4 = CRT TUBE: barrel distortion over a black bezel, RGB
    ///            aperture grille, hard scanlines, mains flicker
    ///   kind 5 = ACID TRIP: radial hue cycling, oversaturation, mild
    ///            posterize, liquid warp
    ///   kind 6 = DATAMOSH: horizontal slice/block displacement glitch,
    ///            channel swaps, digital noise blocks
    ///   kind 7 = NEON BLOOM: bright-pass glow, shadows lifted toward the
    ///            colour
    ///   kind 8 = PIXEL MOSAIC: chunky pixelation + dithered posterize
    ///   kind 9 = TUNNEL RUSH: radial zoom blur toward the centre, hot core,
    ///            edge vignette
    /// Only the last POSTFX of a frame applies. Any other kind is a no-op.
    pub fn postfx(&self, kind: u32, t: f32, color: Color) {
        self.push(&[op::POSTFX, kind as f32, t, color.r, color.g, color.b]);
    }

    /// Open a PIXEL-ART GROUP. Everything drawn until the matching
    /// [`pixel_end`](Self::pixel_end) is rasterized at ART RESOLUTION —
    /// renderer.js redirects the batch into a scratch texture of
    /// `ceil(w / px) x ceil(h / px)` texels and installs a transform so that
    /// the group's local coordinates `0..w, 0..h` land on those texels (one
    /// art pixel = `px` local units) — with no anti-aliasing (the scratch
    /// target has no MSAA, and the batch shader draws hard coverage). Thin
    /// details survive: inside a group, line / rect-outline thickness is
    /// clamped to >= 1 texel and circle radius to >= 0.5 texel.
    ///
    /// The principle: do NOT draw hi-res and average / point-sample down —
    /// draw AT the art resolution and upscale NEAREST. Every art pixel is then
    /// a full pixel, edges are consistent (a shape's edge is either in a texel
    /// or not, never a soft blend), and the pixel grid is anchored to the
    /// object rather than to the screen, so an animated / moving prop keeps
    /// the same crisp look wherever it lands.
    ///
    /// The transform stack is saved on entry and restored by `pixel_end`
    /// (save/restore issued inside the group are balanced away). Groups NEST
    /// (up to 4 deep): an inner group is rasterized into its own scratch
    /// target and its `pixel_end` composites the result into the enclosing
    /// group's texels (then that group's own `pixel_end` upscales the lot).
    /// Robots / the shoggoth may be drawn inside a group (their tiles are
    /// composited into the group texels). Groups larger than the renderer's
    /// scratch cap (1024 texels per side, e.g. a full canvas at `px = 1`) or
    /// deeper than the nesting cap fall back to pass-through drawing — no
    /// pixelation, and the matching `pixel_end` is a no-op (so the content
    /// lands at the current origin, not at the end's `(x, y)`).
    ///
    /// `px` is in the CURRENT local units at the time of the call (an art
    /// pixel is `px` local units wide), which is what anchors the grid to
    /// the object: open the group inside a scaled / rotated transform and the
    /// art pixels scale / rotate with it. See `crate::props::draw_prop_ex`
    /// for the two idioms — pixelate BEFORE a rotation (open the group under
    /// the rotation: the finished pixel image is rotated as a whole) or AFTER
    /// it (open the group in the parent frame, rotate inside it: the content
    /// is re-rasterized on the parent's grid every frame).
    pub fn pixel_begin(&self, px: f32, w: f32, h: f32) {
        self.push(&[op::PIX_BEGIN, px, w, h]);
    }

    /// Close the pixel-art group opened by [`pixel_begin`](Self::pixel_begin):
    /// the group's texture is drawn as a `(w, h)` quad with its origin at
    /// `(x, y)` in the CURRENT (outer) transform, NEAREST-filtered, with the
    /// origin snapped to whole pixels of the target it lands in (device
    /// pixels, or the enclosing group's texels) so the art pixels stay
    /// stable frame to frame.
    pub fn pixel_end(&self, x: f32, y: f32) {
        self.push(&[op::PIX_END, x, y]);
    }

    /// Draw a small 2D-primitive shoggoth icon: a writhing dark mass wearing a
    /// friendly yellow smiley mask (`enraged` false) or showing the horror
    /// beneath (red eyes, tentacles, mask shards). Only used for thumbnails
    /// (the `?viz` gallery / level map); the in-game boss is
    /// `draw_shoggoth_live` (a fallback if the live pipeline were ever missing).
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
