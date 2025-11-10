// Rendering system for drawing entities
use crate::components::*;
use crate::ecs::{Entity, World};
use crate::graphics::Graphics;
use crate::math::{Color, Vec2};

/// Render all entities in the world
pub fn render_entities(world: &World, graphics: &Graphics, show_infos: bool) {
    // Render vision cones first (behind everything) - only if info display is enabled
    if show_infos {
        render_enemy_vision_cones(world, graphics);
    }

    // Render projectile trails
    render_projectile_trails(world, graphics);

    // Render bullets
    render_bullets(world, graphics);

    // Render enemies
    render_enemies(world, graphics);

    // Render player (on top)
    render_player(world, graphics);
}

/// Render walls from the world
pub fn render_walls(world: &World, graphics: &Graphics) {
    for wall in world.walls() {
        // Draw wall with dark purple color
        graphics.draw_rectangle(
            Vec2::new(wall.x, wall.y),
            wall.width,
            wall.height,
            Color::new(80.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0),
        );
        // Border for visual depth
        graphics.draw_rectangle_lines(
            Vec2::new(wall.x, wall.y),
            wall.width,
            wall.height,
            2.0,
            Color::new(100.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0),
        );
    }
}

/// Render enemy vision cones
fn render_enemy_vision_cones(world: &World, graphics: &Graphics) {
    let enemies: Vec<Entity> = world.query::<Enemy>();

    for entity in enemies {
        let (pos, rotation, ai, health) = match (
            world.get_component::<Position>(entity),
            world.get_component::<Rotation>(entity),
            world.get_component::<AI>(entity),
            world.get_component::<Health>(entity),
        ) {
            (Some(p), Some(r), Some(a), Some(h)) => (p, r, a, h),
            _ => continue,
        };

        // Only draw vision cone for alive enemies
        if health.is_dead() {
            continue;
        }

        // Draw a 90-degree cone in the direction the enemy is facing
        let cone_angle = std::f32::consts::PI / 2.0; // 90 degrees
        let start_angle = rotation.angle - cone_angle / 2.0;
        let end_angle = rotation.angle + cone_angle / 2.0;

        // Semi-transparent red cone
        let color = Color::new(1.0, 0.0, 0.0, 0.1);
        graphics.draw_arc(
            Vec2::new(pos.x, pos.y),
            ai.detection_range,
            start_angle,
            end_angle,
            color,
        );
    }
}

/// Render projectile trails
fn render_projectile_trails(world: &World, graphics: &Graphics) {
    let trails: Vec<Entity> = world.query::<ProjectileTrail>();

    for entity in trails {
        let trail = match world.get_component::<ProjectileTrail>(entity) {
            Some(t) => t,
            None => continue,
        };

        // Calculate alpha based on remaining lifetime (fade out effect)
        let alpha = trail.alpha();
        let color = Color::new(1.0, 0.9, 0.3, alpha); // Yellow-ish color with fade

        graphics.draw_line(
            Vec2::new(trail.start.x, trail.start.y),
            Vec2::new(trail.end.x, trail.end.y),
            2.0, // Line width
            color,
        );
    }
}

/// Render bullets
fn render_bullets(world: &World, graphics: &Graphics) {
    let bullets: Vec<Entity> = world.query::<Bullet>();

    for entity in bullets {
        let pos = match world.get_component::<Position>(entity) {
            Some(p) => p,
            None => continue,
        };

        let radius = world
            .get_component::<Radius>(entity)
            .map(|r| r.value)
            .unwrap_or(2.0);

        // Yellow bullets
        let color = Color::new(1.0, 0.9, 0.3, 1.0);
        graphics.draw_circle(Vec2::new(pos.x, pos.y), radius, color);
    }
}

/// Render all enemies
fn render_enemies(world: &World, graphics: &Graphics) {
    let enemies: Vec<Entity> = world.query::<Enemy>();

    for entity in enemies {
        let (pos, rotation, health) = match (
            world.get_component::<Position>(entity),
            world.get_component::<Rotation>(entity),
            world.get_component::<Health>(entity),
        ) {
            (Some(p), Some(r), Some(h)) => (p, r, h),
            _ => continue,
        };

        let base_color = Color::RED;
        let is_dead = health.is_dead();

        graphics.draw_pixelated_sprite(
            Vec2::new(pos.x, pos.y),
            rotation.angle,
            base_color,
            is_dead,
        );
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
        // Draw player as pixelated sprite
        let base_color = Color::BLUE;
        graphics.draw_pixelated_sprite(
            Vec2::new(pos.x, pos.y),
            rotation,
            base_color,
            false, // Player is alive
        );
    }
}

/// Render UI (health, ammo, etc.)
pub fn render_ui(
    graphics: &Graphics,
    health: i32,
    ammo: i32,
    enemies_alive: usize,
    player_alive: bool,
    death_time: f32,
    level_complete: bool,
    level_complete_time: f32,
    debug_enabled: bool,
    show_infos: bool,
) {
    let screen_width = graphics.width();
    let screen_height = graphics.height();

    if player_alive && !level_complete {
        graphics.draw_text("Health:", Vec2::new(10.0, 30.0), 20.0, Color::WHITE);
        graphics.draw_text(
            &format!("{}", health),
            Vec2::new(100.0, 30.0),
            20.0,
            Color::WHITE,
        );

        graphics.draw_text("Ammo:", Vec2::new(10.0, 60.0), 20.0, Color::WHITE);
        graphics.draw_text(
            &format!("{}", ammo),
            Vec2::new(100.0, 60.0),
            20.0,
            Color::WHITE,
        );

        graphics.draw_text("Enemies:", Vec2::new(10.0, 90.0), 20.0, Color::WHITE);
        graphics.draw_text(
            &format!("{}", enemies_alive),
            Vec2::new(120.0, 90.0),
            20.0,
            Color::WHITE,
        );
    } else if !player_alive {
        // Death screen with animations

        // "YOU DIED" - reveal left to right
        let message = "YOU DIED";
        let reveal_duration = 1.0; // 1 second to fully reveal
        let reveal_progress = (death_time / reveal_duration).min(1.0);
        let chars_to_show = (message.len() as f32 * reveal_progress) as usize;
        let revealed_text = &message[0..chars_to_show.min(message.len())];

        graphics.draw_text(
            revealed_text,
            Vec2::new(screen_width / 2.0 - 100.0, screen_height / 2.0),
            60.0,
            Color::RED,
        );

        // "Press R to restart" - wobbling animation
        // Only show after main message is fully revealed
        if death_time > reveal_duration {
            let anim_time = death_time - reveal_duration;

            // Wobble position (move up and down)
            let y_amplitude = 5.0; // pixels
            let y_speed = 1.5; // Hz
            let y_offset = y_amplitude * (anim_time * y_speed * 2.0 * std::f32::consts::PI).sin();

            graphics.draw_text(
                "Press R to restart",
                Vec2::new(
                    screen_width / 2.0 - 120.0,
                    screen_height / 2.0 + 80.0 + y_offset,
                ),
                30.0,
                Color::WHITE,
            );
        }
    } else if level_complete {
        // Level complete screen with animations

        // "LEVEL COMPLETE" - reveal left to right
        let message = "LEVEL COMPLETE";
        let reveal_duration = 1.0;
        let reveal_progress = (level_complete_time / reveal_duration).min(1.0);
        let chars_to_show = (message.len() as f32 * reveal_progress) as usize;
        let revealed_text = &message[0..chars_to_show.min(message.len())];

        graphics.draw_text(
            revealed_text,
            Vec2::new(screen_width / 2.0 - 140.0, screen_height / 2.0),
            60.0,
            Color::new(0.0, 1.0, 0.0, 1.0), // Green
        );

        // "TIME TO EXTRACT" - wobbling animation
        if level_complete_time > reveal_duration {
            let anim_time = level_complete_time - reveal_duration;

            // Wobble position
            let y_amplitude = 5.0;
            let y_speed = 1.5;
            let y_offset = y_amplitude * (anim_time * y_speed * 2.0 * std::f32::consts::PI).sin();

            graphics.draw_text(
                "TIME TO EXTRACT",
                Vec2::new(
                    screen_width / 2.0 - 120.0,
                    screen_height / 2.0 + 80.0 + y_offset,
                ),
                30.0,
                Color::WHITE,
            );
        }
    }

    // Info display indicator
    if debug_enabled {
        let info_text = if show_infos {
            "Infos: ON (Press I to toggle)"
        } else {
            "Infos: OFF (Press I to toggle)"
        };
        let info_color = if show_infos {
            Color::new(0.0, 1.0, 0.0, 1.0) // Green when active
        } else {
            Color::GRAY // Gray when inactive
        };
        graphics.draw_text(
            info_text,
            Vec2::new(screen_width - 280.0, 30.0),
            16.0,
            info_color,
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
