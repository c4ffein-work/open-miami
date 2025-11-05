use crate::math::Vec2;

pub struct Camera {
    pub target: Vec2,
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec2::zero(),
            offset: Vec2::zero(),
            zoom: 1.0,
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

    pub fn apply(&self) {
        // In a pure canvas implementation, we would apply transformations here
        // For now, this is a placeholder for potential canvas transform operations
    }

    pub fn reset(&self) {
        // Reset camera transformations
        // Placeholder for canvas transform reset
    }

    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        // For now, we'll implement a simple screen-to-world conversion
        // This assumes the camera is centered on the target
        // In a full implementation, you'd apply the inverse of the camera transform

        // Simplified version: just return screen_pos for now
        // This can be enhanced later with proper transformation math
        screen_pos
    }
}
