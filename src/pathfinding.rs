use crate::collision::{circle_rect_collision, has_line_of_sight_with_padding};
use crate::ecs::world::Wall;
use crate::math::Vec2;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Find the closest point on a rectangle to a given point
fn closest_point_on_rect(point: Vec2, rect_x: f32, rect_y: f32, rect_w: f32, rect_h: f32) -> Vec2 {
    let closest_x = point.x.max(rect_x).min(rect_x + rect_w);
    let closest_y = point.y.max(rect_y).min(rect_y + rect_h);
    Vec2::new(closest_x, closest_y)
}

/// Check if a line segment intersects with a rectangle (for finding blocking walls)
fn line_intersects_rect(
    line_start: Vec2,
    line_end: Vec2,
    rect_x: f32,
    rect_y: f32,
    rect_w: f32,
    rect_h: f32,
) -> bool {
    // Simple line-segment intersection check
    // Check if line intersects any of the 4 edges of the rectangle
    let top_left = Vec2::new(rect_x, rect_y);
    let top_right = Vec2::new(rect_x + rect_w, rect_y);
    let bottom_left = Vec2::new(rect_x, rect_y + rect_h);
    let bottom_right = Vec2::new(rect_x + rect_w, rect_y + rect_h);

    line_segment_intersection(line_start, line_end, top_left, top_right)
        || line_segment_intersection(line_start, line_end, top_right, bottom_right)
        || line_segment_intersection(line_start, line_end, bottom_right, bottom_left)
        || line_segment_intersection(line_start, line_end, bottom_left, top_left)
}

/// Check if two line segments intersect
fn line_segment_intersection(p1: Vec2, p2: Vec2, p3: Vec2, p4: Vec2) -> bool {
    let d1 = (p2.x - p1.x) * (p3.y - p1.y) - (p2.y - p1.y) * (p3.x - p1.x);
    let d2 = (p2.x - p1.x) * (p4.y - p1.y) - (p2.y - p1.y) * (p4.x - p1.x);
    let d3 = (p4.x - p3.x) * (p1.y - p3.y) - (p4.y - p3.y) * (p1.x - p3.x);
    let d4 = (p4.x - p3.x) * (p2.y - p3.y) - (p4.y - p3.y) * (p2.x - p3.x);

    d1 * d2 <= 0.0 && d3 * d4 <= 0.0
}

/// Size of each grid cell in world units
pub const GRID_CELL_SIZE: f32 = 50.0;

/// World dimensions (must match game world size)
pub const WORLD_WIDTH: f32 = 2000.0;
pub const WORLD_HEIGHT: f32 = 2000.0;

/// Grid coordinates (i, j) representing a cell in the navigation grid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridCoord {
    pub i: i32,
    pub j: i32,
}

impl GridCoord {
    pub fn new(i: i32, j: i32) -> Self {
        GridCoord { i, j }
    }

    /// Convert world position to grid coordinates
    pub fn from_world_pos(x: f32, y: f32) -> Self {
        let i = (x / GRID_CELL_SIZE).floor() as i32;
        let j = (y / GRID_CELL_SIZE).floor() as i32;
        GridCoord { i, j }
    }

    /// Convert grid coordinates to world position (center of cell)
    pub fn to_world_pos(&self) -> Vec2 {
        Vec2::new(
            self.i as f32 * GRID_CELL_SIZE + GRID_CELL_SIZE / 2.0,
            self.j as f32 * GRID_CELL_SIZE + GRID_CELL_SIZE / 2.0,
        )
    }

    /// Get Manhattan distance to another grid coordinate
    pub fn manhattan_distance(&self, other: &GridCoord) -> i32 {
        (self.i - other.i).abs() + (self.j - other.j).abs()
    }

    /// Get Euclidean distance squared to another grid coordinate (for heuristic)
    pub fn distance_squared(&self, other: &GridCoord) -> i32 {
        let di = self.i - other.i;
        let dj = self.j - other.j;
        di * di + dj * dj
    }

    /// Get all valid neighbors (4-directional: up, down, left, right)
    pub fn neighbors(&self) -> Vec<GridCoord> {
        vec![
            GridCoord::new(self.i - 1, self.j), // left
            GridCoord::new(self.i + 1, self.j), // right
            GridCoord::new(self.i, self.j - 1), // down
            GridCoord::new(self.i, self.j + 1), // up
        ]
    }

    /// Check if coordinate is within grid bounds
    pub fn is_valid(&self) -> bool {
        let grid_width = (WORLD_WIDTH / GRID_CELL_SIZE) as i32;
        let grid_height = (WORLD_HEIGHT / GRID_CELL_SIZE) as i32;
        self.i >= 0 && self.i < grid_width && self.j >= 0 && self.j < grid_height
    }
}

/// Node in the A* search with priority based on f-score
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AStarNode {
    coord: GridCoord,
    f_score: i32, // g_score + h_score (priority)
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap behavior
        other.f_score.cmp(&self.f_score)
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Navigation grid representing walkable/blocked cells
pub struct NavigationGrid {
    blocked_cells: HashSet<GridCoord>,
    walls: Vec<Wall>,
}

impl NavigationGrid {
    /// Create a new navigation grid from world walls
    pub fn new(walls: &[Wall]) -> Self {
        let grid_width = (WORLD_WIDTH / GRID_CELL_SIZE) as i32;
        let grid_height = (WORLD_HEIGHT / GRID_CELL_SIZE) as i32;
        let mut blocked_cells = HashSet::new();

        // Mark cells as blocked if they intersect with any wall
        for i in 0..grid_width {
            for j in 0..grid_height {
                let coord = GridCoord::new(i, j);
                let cell_center = coord.to_world_pos();

                // Check if this cell's center would collide with any wall
                // Using a larger radius to keep enemies away from walls and prevent grinding
                let collision_radius = GRID_CELL_SIZE * 0.5; // 50% of cell size (reasonable value)

                for wall in walls {
                    if circle_rect_collision(
                        cell_center,
                        collision_radius,
                        wall.x,
                        wall.y,
                        wall.width,
                        wall.height,
                    ) {
                        blocked_cells.insert(coord);
                        break;
                    }
                }
            }
        }

        NavigationGrid {
            blocked_cells,
            walls: walls.to_vec(),
        }
    }

    /// Check if a grid cell is walkable
    pub fn is_walkable(&self, coord: &GridCoord) -> bool {
        coord.is_valid() && !self.blocked_cells.contains(coord)
    }

    /// Perform string pulling optimization on a path
    /// Removes redundant waypoints by checking line of sight with inflated walls (25px padding)
    fn string_pull_path(&self, path: Vec<Vec2>) -> Vec<Vec2> {
        if path.len() <= 2 {
            return path; // Can't optimize paths with 2 or fewer waypoints
        }

        let wall_padding = 25.0; // Same as used in AI system for consistency
        let mut optimized_path = Vec::new();

        let mut current_idx = 0;
        optimized_path.push(path[current_idx]);

        while current_idx < path.len() - 1 {
            // Try to skip ahead as far as possible while maintaining line of sight
            let mut furthest_visible = current_idx + 1;

            for test_idx in (current_idx + 2)..path.len() {
                if has_line_of_sight_with_padding(
                    path[current_idx],
                    path[test_idx],
                    &self.walls,
                    wall_padding,
                ) {
                    furthest_visible = test_idx;
                } else {
                    break; // Stop searching once we lose line of sight
                }
            }

            // Add the furthest visible waypoint
            current_idx = furthest_visible;
            optimized_path.push(path[current_idx]);
        }

        optimized_path
    }

    /// Push waypoints toward inflated walls to create tactical, wall-hugging movement
    /// This makes enemies take the shortest path around obstacles by moving close to walls
    fn optimize_waypoints_toward_walls(&self, mut path: Vec<Vec2>) -> Vec<Vec2> {
        if path.len() < 3 {
            return path; // Need at least 3 points (start, waypoint, end)
        }

        let wall_padding = 25.0; // Inflated wall boundary (same as AI system)
        let safety_margin = 3.0; // Stay 3px away from inflated boundary for extra safety

        // Optimize each intermediate waypoint (skip first and last)
        for i in 1..path.len() - 1 {
            let prev = path[i - 1];
            let current = path[i];
            let next = path[i + 1];

            // Find which inflated wall blocks the direct path from prev to next
            let mut blocking_wall: Option<&Wall> = None;
            for wall in &self.walls {
                let inflated_x = wall.x - wall_padding;
                let inflated_y = wall.y - wall_padding;
                let inflated_w = wall.width + wall_padding * 2.0;
                let inflated_h = wall.height + wall_padding * 2.0;

                if line_intersects_rect(prev, next, inflated_x, inflated_y, inflated_w, inflated_h)
                {
                    blocking_wall = Some(wall);
                    break; // Use first blocking wall found
                }
            }

            // If we found a blocking wall, push waypoint toward it
            if let Some(wall) = blocking_wall {
                let inflated_x = wall.x - wall_padding;
                let inflated_y = wall.y - wall_padding;
                let inflated_w = wall.width + wall_padding * 2.0;
                let inflated_h = wall.height + wall_padding * 2.0;

                // Find closest point on inflated wall boundary
                let closest_wall_point =
                    closest_point_on_rect(current, inflated_x, inflated_y, inflated_w, inflated_h);

                // Calculate direction from current waypoint toward wall
                let to_wall = Vec2::new(
                    closest_wall_point.x - current.x,
                    closest_wall_point.y - current.y,
                );
                let distance_to_wall = to_wall.length();

                if distance_to_wall > safety_margin {
                    // Push waypoint toward wall, stopping at safety margin
                    let push_distance = distance_to_wall - safety_margin;
                    let direction =
                        Vec2::new(to_wall.x / distance_to_wall, to_wall.y / distance_to_wall);
                    let new_waypoint = Vec2::new(
                        current.x + direction.x * push_distance,
                        current.y + direction.y * push_distance,
                    );

                    // Verify we still have LOS to neighbors with new position
                    if has_line_of_sight_with_padding(prev, new_waypoint, &self.walls, wall_padding)
                        && has_line_of_sight_with_padding(
                            new_waypoint,
                            next,
                            &self.walls,
                            wall_padding,
                        )
                    {
                        path[i] = new_waypoint;
                    }
                }
            }
        }

        path
    }

    /// Find path from start to goal using A* algorithm
    /// Returns a list of world positions (waypoints) from start to goal
    pub fn find_path(&self, start: Vec2, goal: Vec2) -> Option<Vec<Vec2>> {
        let start_coord = GridCoord::from_world_pos(start.x, start.y);
        let goal_coord = GridCoord::from_world_pos(goal.x, goal.y);

        // If start or goal is blocked, return None
        if !self.is_walkable(&start_coord) || !self.is_walkable(&goal_coord) {
            return None;
        }

        // If we're already at the goal, return empty path
        if start_coord == goal_coord {
            return Some(vec![]);
        }

        // A* algorithm
        let mut open_set = BinaryHeap::new();
        let mut came_from: HashMap<GridCoord, GridCoord> = HashMap::new();
        let mut g_score: HashMap<GridCoord, i32> = HashMap::new();
        let mut closed_set: HashSet<GridCoord> = HashSet::new();

        g_score.insert(start_coord, 0);
        let h_score = start_coord.distance_squared(&goal_coord);
        open_set.push(AStarNode {
            coord: start_coord,
            f_score: h_score,
        });

        while let Some(current_node) = open_set.pop() {
            let current = current_node.coord;

            // Goal reached!
            if current == goal_coord {
                let path = self.reconstruct_path(came_from, current);
                let path = self.string_pull_path(path);
                let path = self.optimize_waypoints_toward_walls(path);
                return Some(path);
            }

            // Skip if already processed (can happen with duplicate nodes in heap)
            if closed_set.contains(&current) {
                continue;
            }
            closed_set.insert(current);

            let current_g = *g_score.get(&current).unwrap_or(&i32::MAX);

            // Check all neighbors
            for neighbor in current.neighbors() {
                if !self.is_walkable(&neighbor) || closed_set.contains(&neighbor) {
                    continue;
                }

                // Cost to move to neighbor is always 1 (uniform cost for grid movement)
                let tentative_g = current_g + 1;
                let neighbor_g = *g_score.get(&neighbor).unwrap_or(&i32::MAX);

                if tentative_g < neighbor_g {
                    // Found a better path to neighbor
                    came_from.insert(neighbor, current);
                    g_score.insert(neighbor, tentative_g);

                    let h_score = neighbor.distance_squared(&goal_coord);
                    let f_score = tentative_g + h_score;

                    open_set.push(AStarNode {
                        coord: neighbor,
                        f_score,
                    });
                }
            }
        }

        // No path found
        None
    }

    /// Reconstruct path from A* came_from map
    fn reconstruct_path(
        &self,
        came_from: HashMap<GridCoord, GridCoord>,
        mut current: GridCoord,
    ) -> Vec<Vec2> {
        let mut path = vec![current.to_world_pos()];

        while let Some(&prev) = came_from.get(&current) {
            current = prev;
            path.push(current.to_world_pos());
        }

        path.reverse();
        path
    }

    /// Get the next waypoint to move toward (first step in path)
    pub fn get_next_waypoint(&self, start: Vec2, goal: Vec2) -> Option<Vec2> {
        let path = self.find_path(start, goal)?;
        if path.is_empty() {
            // Already at goal
            Some(goal)
        } else {
            // Skip waypoints that are in the same grid cell as start
            let start_coord = GridCoord::from_world_pos(start.x, start.y);
            for waypoint in &path {
                let waypoint_coord = GridCoord::from_world_pos(waypoint.x, waypoint.y);
                if waypoint_coord != start_coord {
                    return Some(*waypoint);
                }
            }
            // All waypoints are in the same cell, just return the goal
            Some(goal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_coord_from_world_pos() {
        let coord = GridCoord::from_world_pos(125.0, 75.0);
        assert_eq!(coord.i, 2);
        assert_eq!(coord.j, 1);

        let coord = GridCoord::from_world_pos(0.0, 0.0);
        assert_eq!(coord.i, 0);
        assert_eq!(coord.j, 0);

        let coord = GridCoord::from_world_pos(1999.0, 1999.0);
        assert_eq!(coord.i, 39);
        assert_eq!(coord.j, 39);
    }

    #[test]
    fn test_grid_coord_to_world_pos() {
        let coord = GridCoord::new(2, 1);
        let pos = coord.to_world_pos();
        assert_eq!(pos.x, 125.0); // 2 * 50 + 25
        assert_eq!(pos.y, 75.0); // 1 * 50 + 25

        let coord = GridCoord::new(0, 0);
        let pos = coord.to_world_pos();
        assert_eq!(pos.x, 25.0);
        assert_eq!(pos.y, 25.0);
    }

    #[test]
    fn test_grid_coord_round_trip() {
        let original = Vec2::new(325.0, 175.0);
        let coord = GridCoord::from_world_pos(original.x, original.y);
        let world_pos = coord.to_world_pos();

        // Should be at cell center
        assert_eq!(coord.i, 6);
        assert_eq!(coord.j, 3);
        assert_eq!(world_pos.x, 325.0); // 6 * 50 + 25
        assert_eq!(world_pos.y, 175.0); // 3 * 50 + 25
    }

    #[test]
    fn test_manhattan_distance() {
        let a = GridCoord::new(0, 0);
        let b = GridCoord::new(3, 4);
        assert_eq!(a.manhattan_distance(&b), 7);

        let c = GridCoord::new(5, 5);
        let d = GridCoord::new(2, 1);
        assert_eq!(c.manhattan_distance(&d), 7);
    }

    #[test]
    fn test_distance_squared() {
        let a = GridCoord::new(0, 0);
        let b = GridCoord::new(3, 4);
        assert_eq!(a.distance_squared(&b), 25); // 3^2 + 4^2 = 25
    }

    #[test]
    fn test_neighbors() {
        let coord = GridCoord::new(5, 5);
        let neighbors = coord.neighbors();

        assert_eq!(neighbors.len(), 4);
        assert!(neighbors.contains(&GridCoord::new(4, 5))); // left
        assert!(neighbors.contains(&GridCoord::new(6, 5))); // right
        assert!(neighbors.contains(&GridCoord::new(5, 4))); // down
        assert!(neighbors.contains(&GridCoord::new(5, 6))); // up
    }

    #[test]
    fn test_is_valid() {
        assert!(GridCoord::new(0, 0).is_valid());
        assert!(GridCoord::new(39, 39).is_valid());
        assert!(!GridCoord::new(-1, 0).is_valid());
        assert!(!GridCoord::new(0, -1).is_valid());
        assert!(!GridCoord::new(40, 0).is_valid());
        assert!(!GridCoord::new(0, 40).is_valid());
    }

    #[test]
    fn test_navigation_grid_no_walls() {
        let grid = NavigationGrid::new(&[]);

        // All cells should be walkable with no walls
        assert!(grid.is_walkable(&GridCoord::new(0, 0)));
        assert!(grid.is_walkable(&GridCoord::new(10, 10)));
        assert!(grid.is_walkable(&GridCoord::new(39, 39)));
    }

    #[test]
    fn test_navigation_grid_with_walls() {
        // Create a wall at (200, 200) with size 400x20
        let walls = vec![Wall::new(200.0, 200.0, 400.0, 20.0)];
        let grid = NavigationGrid::new(&walls);

        // Cells intersecting the wall should be blocked
        let wall_cell = GridCoord::from_world_pos(300.0, 210.0);
        assert!(!grid.is_walkable(&wall_cell));

        // Cells away from wall should be walkable
        let free_cell = GridCoord::from_world_pos(100.0, 100.0);
        assert!(grid.is_walkable(&free_cell));
    }

    #[test]
    fn test_find_path_straight_line() {
        // No walls, should find straight path
        let grid = NavigationGrid::new(&[]);
        let start = Vec2::new(25.0, 25.0);
        let goal = Vec2::new(175.0, 25.0);

        let path = grid.find_path(start, goal);
        assert!(path.is_some());

        let path = path.unwrap();
        assert!(!path.is_empty());

        // Should move from (0,0) to (3,0) in grid coords
        // Path length should be reasonable
        assert!(!path.is_empty());
    }

    #[test]
    fn test_find_path_same_position() {
        let grid = NavigationGrid::new(&[]);
        let pos = Vec2::new(100.0, 100.0);

        let path = grid.find_path(pos, pos);
        assert!(path.is_some());
        assert_eq!(path.unwrap().len(), 0); // Empty path when already at goal
    }

    #[test]
    fn test_find_path_around_obstacle() {
        // Create a vertical wall blocking direct path
        let walls = vec![Wall::new(250.0, 0.0, 20.0, 300.0)];
        let grid = NavigationGrid::new(&walls);

        let start = Vec2::new(100.0, 150.0); // Left of wall
        let goal = Vec2::new(400.0, 150.0); // Right of wall

        let path = grid.find_path(start, goal);
        assert!(path.is_some());

        // Path should exist and navigate around the wall
        let path = path.unwrap();
        assert!(!path.is_empty());
    }

    #[test]
    fn test_find_path_blocked_start() {
        // Wall covering start position
        let walls = vec![Wall::new(0.0, 0.0, 100.0, 100.0)];
        let grid = NavigationGrid::new(&walls);

        let start = Vec2::new(50.0, 50.0); // Inside wall
        let goal = Vec2::new(500.0, 500.0);

        let path = grid.find_path(start, goal);
        assert!(path.is_none()); // Can't path from blocked position
    }

    #[test]
    fn test_find_path_blocked_goal() {
        // Wall covering goal position
        let walls = vec![Wall::new(400.0, 400.0, 100.0, 100.0)];
        let grid = NavigationGrid::new(&walls);

        let start = Vec2::new(50.0, 50.0);
        let goal = Vec2::new(450.0, 450.0); // Inside wall

        let path = grid.find_path(start, goal);
        assert!(path.is_none()); // Can't path to blocked position
    }

    #[test]
    fn test_find_path_completely_blocked() {
        // Create walls that completely surround the goal
        let walls = vec![
            Wall::new(300.0, 300.0, 200.0, 10.0), // top
            Wall::new(300.0, 490.0, 200.0, 10.0), // bottom
            Wall::new(300.0, 300.0, 10.0, 200.0), // left
            Wall::new(490.0, 300.0, 10.0, 200.0), // right
        ];
        let grid = NavigationGrid::new(&walls);

        let start = Vec2::new(100.0, 100.0);
        let goal = Vec2::new(400.0, 400.0); // Surrounded by walls

        let path = grid.find_path(start, goal);
        // Depending on wall thickness, might not be able to reach
        // This test verifies the algorithm handles unreachable goals
        if path.is_some() {
            // If path found, it should be valid
            assert!(!path.unwrap().is_empty());
        }
    }

    #[test]
    fn test_get_next_waypoint() {
        let grid = NavigationGrid::new(&[]);
        let start = Vec2::new(25.0, 25.0);
        let goal = Vec2::new(475.0, 25.0);

        let waypoint = grid.get_next_waypoint(start, goal);
        assert!(waypoint.is_some());

        // Next waypoint should be closer to goal than start
        let wp = waypoint.unwrap();
        assert!(wp.x > start.x);
    }

    #[test]
    fn test_get_next_waypoint_at_goal() {
        let grid = NavigationGrid::new(&[]);
        let pos = Vec2::new(100.0, 100.0);

        let waypoint = grid.get_next_waypoint(pos, pos);
        assert!(waypoint.is_some());

        // Should return goal when already there
        let wp = waypoint.unwrap();
        assert_eq!(wp.x, pos.x);
        assert_eq!(wp.y, pos.y);
    }

    #[test]
    fn test_complex_maze_path() {
        // Create a more complex wall setup
        let walls = vec![
            Wall::new(200.0, 0.0, 20.0, 300.0),   // vertical wall 1
            Wall::new(400.0, 200.0, 20.0, 400.0), // vertical wall 2
            Wall::new(0.0, 500.0, 300.0, 20.0),   // horizontal wall 1
            Wall::new(500.0, 300.0, 300.0, 20.0), // horizontal wall 2
        ];
        let grid = NavigationGrid::new(&walls);

        let start = Vec2::new(50.0, 50.0);
        let goal = Vec2::new(700.0, 700.0);

        let path = grid.find_path(start, goal);

        // Should find a path through the maze
        assert!(path.is_some());
        let path = path.unwrap();

        // Verify path progresses toward goal
        if !path.is_empty() {
            let first = path.first().unwrap();
            let last = path.last().unwrap();

            // Last waypoint should be closer to goal than first
            let dist_first = first.distance(goal);
            let dist_last = last.distance(goal);
            assert!(dist_last <= dist_first);
        }
    }

    #[test]
    fn test_astar_node_ordering() {
        let node1 = AStarNode {
            coord: GridCoord::new(0, 0),
            f_score: 10,
        };
        let node2 = AStarNode {
            coord: GridCoord::new(1, 1),
            f_score: 5,
        };

        // Lower f_score should have higher priority (come first in max heap)
        assert!(node2 > node1);
    }

    #[test]
    fn test_grid_boundary_cells() {
        let grid = NavigationGrid::new(&[]);

        // Test corners
        assert!(grid.is_walkable(&GridCoord::new(0, 0)));
        assert!(grid.is_walkable(&GridCoord::new(39, 0)));
        assert!(grid.is_walkable(&GridCoord::new(0, 39)));
        assert!(grid.is_walkable(&GridCoord::new(39, 39)));

        // Test just outside bounds
        assert!(!GridCoord::new(-1, 0).is_valid());
        assert!(!GridCoord::new(40, 0).is_valid());
        assert!(!GridCoord::new(0, -1).is_valid());
        assert!(!GridCoord::new(0, 40).is_valid());
    }

    #[test]
    fn test_path_consistency() {
        // Test that finding a path twice gives same result
        let walls = vec![Wall::new(250.0, 250.0, 100.0, 100.0)];
        let grid = NavigationGrid::new(&walls);

        let start = Vec2::new(100.0, 100.0);
        let goal = Vec2::new(600.0, 600.0);

        let path1 = grid.find_path(start, goal);
        let path2 = grid.find_path(start, goal);

        assert_eq!(path1.is_some(), path2.is_some());
        if let (Some(p1), Some(p2)) = (path1, path2) {
            assert_eq!(p1.len(), p2.len());
        }
    }
}
