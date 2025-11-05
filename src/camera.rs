use macroquad::prelude::*;

pub struct Camera {
    pub target: Vec2,
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec2::ZERO,
            offset: Vec2::ZERO,
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
        let offset_x = screen_width() / 2.0 - self.target.x;
        let offset_y = screen_height() / 2.0 - self.target.y;

        set_camera(&Camera2D {
            target: self.target,
            offset: vec2(-offset_x, -offset_y),
            rotation: 0.0,
            zoom: vec2(2.0 / screen_width(), 2.0 / screen_height()) * self.zoom,
            ..Default::default()
        });
    }

    pub fn reset(&self) {
        set_default_camera();
    }

    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        let offset_x = screen_width() / 2.0 - self.target.x;
        let offset_y = screen_height() / 2.0 - self.target.y;

        Vec2::new(screen_pos.x - offset_x, screen_pos.y - offset_y)
    }
}
