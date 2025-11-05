// Rendering system for drawing entities
use crate::components::*;
use crate::ecs::{Entity, World};
use macroquad::prelude::*;

/// Render all entities in the world
pub fn render_entities(world: &World) {
    // Render enemies first (so player appears on top)
    render_enemies(world);

    // Render player
    render_player(world);
}

/// Render all enemies
fn render_enemies(world: &World) {
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
            RED
        } else {
            Color::from_rgba(100, 0, 0, 255) // Dark red for dead
        };

        draw_circle(pos.x, pos.y, radius.value, color);
    }
}

/// Render the player
fn render_player(world: &World) {
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
        draw_circle(pos.x, pos.y, 15.0, BLUE);

        // Draw direction indicator
        let dir_len = 20.0;
        let end_x = pos.x + rotation.cos() * dir_len;
        let end_y = pos.y + rotation.sin() * dir_len;
        draw_line(pos.x, pos.y, end_x, end_y, 3.0, WHITE);
    }
}

/// Render UI (health, ammo, etc.)
pub fn render_ui(health: i32, ammo: i32, enemies_alive: usize, player_alive: bool) {
    if player_alive {
        draw_text(&format!("Health: {}", health), 10.0, 30.0, 30.0, WHITE);
        draw_text(&format!("Ammo: {}", ammo), 10.0, 60.0, 30.0, WHITE);
        draw_text(
            &format!("Enemies: {}", enemies_alive),
            10.0,
            90.0,
            30.0,
            WHITE,
        );
    } else {
        draw_text(
            "YOU DIED",
            screen_width() / 2.0 - 100.0,
            screen_height() / 2.0,
            60.0,
            RED,
        );
        draw_text(
            "Press R to restart",
            screen_width() / 2.0 - 120.0,
            screen_height() / 2.0 + 40.0,
            30.0,
            WHITE,
        );
    }

    // Controls info
    draw_text(
        "WASD: Move | Mouse: Aim | Left Click: Shoot | 1-4: Weapons",
        10.0,
        screen_height() - 20.0,
        20.0,
        GRAY,
    );
}
