use crate::math::Vec2;

pub fn circle_circle_collision(pos1: Vec2, radius1: f32, pos2: Vec2, radius2: f32) -> bool {
    let distance = pos1.distance(pos2);
    distance < radius1 + radius2
}

pub fn circle_rect_collision(
    circle_pos: Vec2,
    radius: f32,
    rect_x: f32,
    rect_y: f32,
    rect_w: f32,
    rect_h: f32,
) -> bool {
    // Find the closest point on the rectangle to the circle
    let closest_x = circle_pos.x.max(rect_x).min(rect_x + rect_w);
    let closest_y = circle_pos.y.max(rect_y).min(rect_y + rect_h);

    let distance = ((circle_pos.x - closest_x).powi(2) + (circle_pos.y - closest_y).powi(2)).sqrt();
    distance < radius
}

pub fn point_in_rect(point: Vec2, rect_x: f32, rect_y: f32, rect_w: f32, rect_h: f32) -> bool {
    point.x >= rect_x
        && point.x <= rect_x + rect_w
        && point.y >= rect_y
        && point.y <= rect_y + rect_h
}
