// Rendering system for drawing entities
use crate::components::*;
use crate::ecs::{Entity, World};
use crate::graphics::Graphics;
use crate::math::{Color, Vec2};

/// Render all entities in the world
pub fn render_entities(world: &World, graphics: &Graphics) {
    // Render enemies first (so player appears on top)
    render_enemies(world, graphics);

    // Render player
    render_player(world, graphics);
}

/// Render all enemies
fn render_enemies(world: &World, graphics: &Graphics) {
    let enemies: Vec<Entity> = world.query::<Enemy>();

    for entity in enemies {
        let (pos, radius, health) = match (
            world.get_component::<Position>(entity),
            world.get_component::<Radius>(entity),
            world.get_component::<Health>(entity),
        ) {
            (Some(p), Some(r), Some(h)) => (p, r, h),
            _ => continue,
        };

        let color = if health.is_alive() {
            Color::RED
        } else {
            Color::new(100.0/255.0, 0.0, 0.0, 1.0) // Dark red for dead
        };

        graphics.draw_circle(Vec2::new(pos.x, pos.y), radius.value, color);
    }
}

/// Render the player
fn render_player(world: &World, graphics: &Graphics) {
    let players: Vec<Entity> = world.query::<Player>();
    let player = match players.first() {
        Some(&e) => e,
        None => return,
    };

    let pos = match world.get_component::<Position>(player) {
        Some(p) => p,
        None => return,
    };

    let rotation = world
        .get_component::<Rotation>(player)
        .map(|r| r.angle)
        .unwrap_or(0.0);

    let health = world
        .get_component::<Health>(player)
        .map(|h| h.current)
        .unwrap_or(0);

    if health > 0 {
        // Draw player body
        graphics.draw_circle(Vec2::new(pos.x, pos.y), 15.0, Color::BLUE);

        // Draw direction indicator
        let dir_len = 20.0;
        let end_x = pos.x + rotation.cos() * dir_len;
        let end_y = pos.y + rotation.sin() * dir_len;
        graphics.draw_line(
            Vec2::new(pos.x, pos.y),
            Vec2::new(end_x, end_y),
            3.0,
            Color::WHITE,
        );
    }
}

/// Render UI (health, ammo, etc.)
pub fn render_ui(graphics: &Graphics, health: i32, ammo: i32, enemies_alive: usize, player_alive: bool) {
    let screen_width = graphics.width();
    let screen_height = graphics.height();

    if player_alive {
        graphics.draw_text("Health:", Vec2::new(10.0, 30.0), 20.0, Color::WHITE);
        graphics.draw_text(&format!("{}", health), Vec2::new(100.0, 30.0), 20.0, Color::WHITE);

        graphics.draw_text("Ammo:", Vec2::new(10.0, 60.0), 20.0, Color::WHITE);
        graphics.draw_text(&format!("{}", ammo), Vec2::new(100.0, 60.0), 20.0, Color::WHITE);

        graphics.draw_text("Enemies:", Vec2::new(10.0, 90.0), 20.0, Color::WHITE);
        graphics.draw_text(&format!("{}", enemies_alive), Vec2::new(120.0, 90.0), 20.0, Color::WHITE);
    } else {
        graphics.draw_text(
            "YOU DIED",
            Vec2::new(screen_width / 2.0 - 100.0, screen_height / 2.0),
            60.0,
            Color::RED,
        );
        graphics.draw_text(
            "Press R to restart",
            Vec2::new(screen_width / 2.0 - 120.0, screen_height / 2.0 + 40.0),
            30.0,
            Color::WHITE,
        );
    }

    // Controls info
    graphics.draw_text(
        "WASD: Move | Mouse: Aim | Left Click: Shoot | 1-4: Weapons",
        Vec2::new(10.0, screen_height - 20.0),
        16.0,
        Color::GRAY,
    );
}
