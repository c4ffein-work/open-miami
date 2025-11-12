use crate::ecs::world::Wall;
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

/// Check if a line segment intersects with a line segment (used for line-rect collision)
fn line_segment_intersection(p1: Vec2, p2: Vec2, p3: Vec2, p4: Vec2) -> bool {
    let d1 = (p2.x - p1.x) * (p3.y - p1.y) - (p2.y - p1.y) * (p3.x - p1.x);
    let d2 = (p2.x - p1.x) * (p4.y - p1.y) - (p2.y - p1.y) * (p4.x - p1.x);
    let d3 = (p4.x - p3.x) * (p1.y - p3.y) - (p4.y - p3.y) * (p1.x - p3.x);
    let d4 = (p4.x - p3.x) * (p2.y - p3.y) - (p4.y - p3.y) * (p2.x - p3.x);

    // Use <= to handle edge cases where lines touch at endpoints
    if d1 * d2 <= 0.0 && d3 * d4 <= 0.0 {
        return true;
    }

    false
}

/// Check if a line segment intersects with a rectangle
fn line_rect_intersection(
    line_start: Vec2,
    line_end: Vec2,
    rect_x: f32,
    rect_y: f32,
    rect_w: f32,
    rect_h: f32,
) -> bool {
    // Check if line intersects any of the 4 edges of the rectangle
    let top_left = Vec2::new(rect_x, rect_y);
    let top_right = Vec2::new(rect_x + rect_w, rect_y);
    let bottom_left = Vec2::new(rect_x, rect_y + rect_h);
    let bottom_right = Vec2::new(rect_x + rect_w, rect_y + rect_h);

    // Check intersection with each edge
    line_segment_intersection(line_start, line_end, top_left, top_right)
        || line_segment_intersection(line_start, line_end, top_right, bottom_right)
        || line_segment_intersection(line_start, line_end, bottom_right, bottom_left)
        || line_segment_intersection(line_start, line_end, bottom_left, top_left)
}

/// Check if there's a clear line of sight between two points (no walls blocking)
pub fn has_line_of_sight(from: Vec2, to: Vec2, walls: &[Wall]) -> bool {
    for wall in walls {
        if line_rect_intersection(from, to, wall.x, wall.y, wall.width, wall.height) {
            return false; // Wall blocks line of sight
        }
    }
    true // No walls blocking
}

/// Check if there's a clear line of sight with inflated wall boundaries
/// This is used to decide between direct movement and pathfinding
/// Walls are expanded by padding on all sides to prevent enemies from trying
/// to move directly toward targets that are very close to walls
pub fn has_line_of_sight_with_padding(from: Vec2, to: Vec2, walls: &[Wall], padding: f32) -> bool {
    for wall in walls {
        // Inflate wall boundaries by padding amount
        let inflated_x = wall.x - padding;
        let inflated_y = wall.y - padding;
        let inflated_w = wall.width + padding * 2.0;
        let inflated_h = wall.height + padding * 2.0;

        if line_rect_intersection(from, to, inflated_x, inflated_y, inflated_w, inflated_h) {
            return false; // Inflated wall blocks line of sight
        }
    }
    true // No inflated walls blocking
}
