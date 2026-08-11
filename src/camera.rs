use crate::graphics::Graphics;
use crate::math::Vec2;

pub struct Camera {
    pub target: Vec2,
    pub offset: Vec2,
    pub zoom: f32,
    canvas_width: f32,
    canvas_height: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec2::zero(),
            offset: Vec2::zero(),
            zoom: 1.0,
            canvas_width: 960.0,
            canvas_height: 720.0,
        }
    }
}

impl Camera {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn follow_player(&mut self, player_pos: Vec2) {
        self.target = player_pos;
    }

    pub fn apply(&self, graphics: &Graphics) {
        // Save the current transformation state
        graphics.save();

        // Center the camera on the target (player)
        // Translate so that the target appears in the center of the screen
        let offset_x = self.canvas_width / 2.0 - self.target.x;
        let offset_y = self.canvas_height / 2.0 - self.target.y;

        graphics.translate(offset_x, offset_y);
    }

    pub fn reset(&self, graphics: &Graphics) {
        // Restore the transformation state
        graphics.restore();
    }

    /// World-space bounds currently visible on screen, given the actual canvas
    /// size. Used to cull off-screen floor tiles. Matches the transform applied
    /// in `apply()`, which centers `target` using the camera's canvas size.
    pub fn visible_bounds(&self, screen_width: f32, screen_height: f32) -> (Vec2, Vec2) {
        let min = Vec2::new(
            self.target.x - self.canvas_width / 2.0,
            self.target.y - self.canvas_height / 2.0,
        );
        let max = Vec2::new(min.x + screen_width, min.y + screen_height);
        (min, max)
    }

    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        // Convert screen coordinates to world coordinates
        // Account for the camera offset that centers the target
        let world_x = screen_pos.x - (self.canvas_width / 2.0 - self.target.x);
        let world_y = screen_pos.y - (self.canvas_height / 2.0 - self.target.y);

        Vec2::new(world_x, world_y)
    }
}
