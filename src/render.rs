// Rendering system for drawing entities
use crate::components::*;
use crate::ecs::{Entity, World};
use crate::graphics::Graphics;
use crate::math::{Color, Vec2};

/// Render all entities in the world
/// `draw_bots` selects whether the player/rogue sprites are drawn here; the
/// game passes false and draws them as live 3D robots instead (the boss is
/// always drawn here, live too). `now` is the continuous animation clock in
/// seconds (drives the boss's writhing).
pub fn render_entities(
    world: &World,
    graphics: &Graphics,
    show_infos: bool,
    draw_bots: bool,
    now: f32,
) {
    // Render debug pathfinding info first (behind everything)
    if show_infos {
        render_debug_pathfinding(world, graphics);
    }

    // Render vision cones first (behind everything) - only if info display is enabled
    if show_infos {
        render_enemy_vision_cones(world, graphics);
    }

    // Render dropped weapon pickups (beneath actors)
    render_pickups(world, graphics);

    // Render projectile trails
    render_projectile_trails(world, graphics);

    // Render bullets
    render_bullets(world, graphics);

    // Render weapons in flight
    render_thrown_weapons(world, graphics);

    // Render enemies
    if draw_bots {
        render_enemies(world, graphics);
    }

    // Render the boss (big; under the player)
    render_bosses(world, graphics, now);

    // Render player (on top)
    if draw_bots {
        render_player(world, graphics);
    }
}

/// On-screen size (px) of the boss's live tile relative to its hitbox radius.
/// shoggoth-core frames 3.8 world units per half-tile and the mass core is
/// 1.65 units, so 4.6x the radius puts the core right over the hitbox and
/// leaves the rest of the tile to the lobes and tentacles.
const BOSS_TILE_PER_RADIUS: f32 = 4.6;

/// Render the shoggoth boss: a LIVE 3D render through shoggoth-core (see
/// `Graphics::draw_shoggoth_live`) — mask on while `Boss::reveal` is 0, the
/// consume-inward mask-off as it runs to 1, the raw tentacled form after.
fn render_bosses(world: &World, graphics: &Graphics, now: f32) {
    for entity in world.query::<Boss>() {
        let (pos, boss, health) = match (
            world.get_component::<Position>(entity),
            world.get_component::<Boss>(entity),
            world.get_component::<Health>(entity),
        ) {
            (Some(p), Some(b), Some(h)) => (p, b, h),
            _ => continue,
        };
        if health.is_dead() {
            continue;
        }
        let radius = world
            .get_component::<Radius>(entity)
            .map(|r| r.value)
            .unwrap_or(42.0);
        // The mask leans toward where it is heading (the boss always turns to
        // face its prey, so its rotation is its movement direction).
        let heading = world
            .get_component::<Rotation>(entity)
            .map(|r| r.angle)
            .unwrap_or(0.0);
        graphics.draw_shoggoth_live(
            Vec2::new(pos.x, pos.y),
            radius * BOSS_TILE_PER_RADIUS,
            heading,
            boss.reveal,
            now,
        );
    }
}

/// Render walls from the world
pub fn render_walls(world: &World, graphics: &Graphics, show_infos: bool) {
    for wall in world.walls() {
        // Draw inflated wall boundaries (debug visualization)
        if show_infos {
            let wall_padding = 25.0; // Same as pathfinding inflation
            graphics.draw_rectangle_lines(
                Vec2::new(wall.x - wall_padding, wall.y - wall_padding),
                wall.width + wall_padding * 2.0,
                wall.height + wall_padding * 2.0,
                1.0,
                Color::new(1.0, 1.0, 0.0, 0.3), // Semi-transparent yellow
            );
        }

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

/// Render debug pathfinding visualization
fn render_debug_pathfinding(world: &World, graphics: &Graphics) {
    use crate::components::{AIState, DebugPath, DebugTrail, Enemy, Position, AI};
    use crate::ecs::Entity;

    let enemies: Vec<Entity> = world.query::<Enemy>();

    for entity in enemies {
        let (pos, ai, debug_path, debug_trail) = match (
            world.get_component::<Position>(entity),
            world.get_component::<AI>(entity),
            world.get_component::<DebugPath>(entity),
            world.get_component::<DebugTrail>(entity),
        ) {
            (Some(p), Some(a), dp, dt) => (p, a, dp, dt),
            _ => continue,
        };

        // Only show pathfinding for enemies that are chasing (SpottedUnsure or SurePlayerSeen)
        match ai.state {
            AIState::SpottedUnsure | AIState::SurePlayerSeen => {
                // Draw actual movement trail first (cyan/blue - behind planned path)
                if let Some(trail) = debug_trail {
                    if trail.positions.len() > 1 {
                        let mut prev_pos = trail.positions[0];
                        for current_pos in trail.positions.iter().skip(1) {
                            graphics.draw_line(
                                prev_pos,
                                *current_pos,
                                2.0,
                                Color::new(0.0, 0.8, 1.0, 0.6), // Cyan, semi-transparent
                            );
                            prev_pos = *current_pos;
                        }
                    }
                }

                // Draw pathfinding waypoints if available
                if let Some(debug_path) = debug_path {
                    if !debug_path.waypoints.is_empty() {
                        // Draw line to final target (semi-transparent red)
                        graphics.draw_line(
                            pos.to_vec2(),
                            debug_path.target,
                            2.0,
                            Color::new(1.0, 0.0, 0.0, 0.3),
                        );

                        // Draw waypoint path (bright green)
                        let mut prev_point = pos.to_vec2();
                        for waypoint in &debug_path.waypoints {
                            // Draw line from previous point to this waypoint
                            graphics.draw_line(
                                prev_point,
                                *waypoint,
                                2.0,
                                Color::new(0.0, 1.0, 0.0, 0.8),
                            );

                            // Draw waypoint as small circle
                            graphics.draw_circle(*waypoint, 4.0, Color::new(0.0, 1.0, 0.0, 1.0));

                            prev_point = *waypoint;
                        }

                        // Draw final segment to target
                        if let Some(last_waypoint) = debug_path.waypoints.last() {
                            graphics.draw_line(
                                *last_waypoint,
                                debug_path.target,
                                2.0,
                                Color::new(0.0, 1.0, 0.0, 0.8),
                            );
                        }

                        // Draw target as larger circle
                        graphics.draw_circle(
                            debug_path.target,
                            6.0,
                            Color::new(1.0, 0.0, 0.0, 1.0),
                        );
                    }
                }
            }
            _ => {} // Don't show pathfinding for other states
        }
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

/// Color used to represent a weapon type on the ground / in the UI.
fn weapon_color(weapon_type: WeaponType) -> Color {
    match weapon_type {
        WeaponType::Pistol => Color::new(0.9, 0.9, 0.9, 1.0), // Light gray
        WeaponType::Shotgun => Color::new(1.0, 0.55, 0.1, 1.0), // Orange
        WeaponType::MachineGun => Color::new(0.2, 0.8, 1.0, 1.0), // Cyan
        WeaponType::Melee => Color::new(0.7, 0.7, 0.75, 1.0), // Steel
    }
}

/// Render dropped weapon pickups as small floor markers.
fn render_pickups(world: &World, graphics: &Graphics) {
    let pickups: Vec<Entity> = world.query::<WeaponPickup>();

    for entity in pickups {
        let (pos, pickup) = match (
            world.get_component::<Position>(entity),
            world.get_component::<WeaponPickup>(entity),
        ) {
            (Some(p), Some(w)) => (p, w),
            _ => continue,
        };

        let color = weapon_color(pickup.weapon_type);

        // A little "on the floor" plate so the weapon marker reads as pickup-able
        graphics.draw_rectangle(
            Vec2::new(pos.x - 11.0, pos.y - 7.0),
            22.0,
            14.0,
            Color::new(0.0, 0.0, 0.0, 0.35),
        );
        // Gun-ish bar
        graphics.draw_rectangle(Vec2::new(pos.x - 8.0, pos.y - 2.0), 16.0, 4.0, color);
        // Grip
        graphics.draw_rectangle(Vec2::new(pos.x - 6.0, pos.y + 2.0), 4.0, 4.0, color);
        // Outline for visibility on dark floor
        graphics.draw_rectangle_lines(Vec2::new(pos.x - 11.0, pos.y - 7.0), 22.0, 14.0, 1.0, color);
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

/// Render weapons currently flying through the air after being thrown.
fn render_thrown_weapons(world: &World, graphics: &Graphics) {
    let thrown: Vec<Entity> = world.query::<ThrownWeapon>();

    for entity in thrown {
        let (pos, tw) = match (
            world.get_component::<Position>(entity),
            world.get_component::<ThrownWeapon>(entity),
        ) {
            (Some(p), Some(t)) => (p, t),
            _ => continue,
        };

        let color = weapon_color(tw.weapon_type);

        // Spinning bar to sell the "tumbling weapon" look.
        graphics.save();
        graphics.translate(pos.x, pos.y);
        graphics.rotate(tw.spin);
        graphics.draw_rectangle(Vec2::new(-10.0, -2.0), 20.0, 4.0, color);
        graphics.restore();
    }
}

/// Render all enemies
fn render_enemies(world: &World, graphics: &Graphics) {
    let enemies: Vec<Entity> = world.query::<Enemy>();

    for entity in enemies {
        // The boss is drawn by render_bosses, not as a regular sprite.
        if world.has_component::<Boss>(entity) {
            continue;
        }

        let (pos, rotation, health, ai) = match (
            world.get_component::<Position>(entity),
            world.get_component::<Rotation>(entity),
            world.get_component::<Health>(entity),
            world.get_component::<AI>(entity),
        ) {
            (Some(p), Some(r), Some(h), Some(a)) => (p, r, h, a),
            _ => continue,
        };

        // Rogue AI palette, keyed by behavioral signature (flavor names in LORE.md).
        let base_color = match ai.initial_type {
            EnemyType::Idle => Color::from_rgba(224, 49, 66, 255), // SENTINEL - hostile red
            EnemyType::Wandering => Color::from_rgba(150, 70, 210, 255), // DRIFTER - glitch violet
            EnemyType::Patrolling => Color::from_rgba(224, 40, 160, 255), // HUNTER - predatory magenta
        };
        // Draw knocked-down (stunned) enemies as prone, like the dead pose.
        let prone = health.is_dead() || world.has_component::<Stunned>(entity);

        graphics.draw_pixelated_sprite(Vec2::new(pos.x, pos.y), rotation.angle, base_color, prone);
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
        // Draw the friendly coral purge bot in warm coral.
        let base_color = Color::from_rgba(217, 119, 87, 255);
        graphics.draw_pixelated_sprite(
            Vec2::new(pos.x, pos.y),
            rotation,
            base_color,
            false, // Player is alive
        );
    }
}

/// Render UI (health, the held weapon and its ammo, etc.). `weapon` is the
/// held weapon type (`None` = unarmed) and `ammo` the rounds left in it.
#[allow(clippy::too_many_arguments)]
pub fn render_ui(
    graphics: &Graphics,
    health: i32,
    ammo: i32,
    weapon: Option<WeaponType>,
    enemies_alive: usize,
    player_alive: bool,
    death_time: f32,
    debug_enabled: bool,
    show_infos: bool,
) {
    let screen_width = graphics.width();
    let screen_height = graphics.height();

    if player_alive {
        graphics.draw_text("Health:", Vec2::new(10.0, 30.0), 20.0, Color::WHITE);
        graphics.draw_text(
            &format!("{}", health),
            Vec2::new(100.0, 30.0),
            20.0,
            Color::WHITE,
        );

        // The one weapon in hand and what's left in it: `WEAPON: SHOTGUN 3/6`
        // (`MELEE` never runs dry; `UNARMED` after a throw). An empty gun is
        // flagged red — throw it, take another.
        graphics.draw_text("Weapon:", Vec2::new(10.0, 60.0), 20.0, Color::WHITE);
        let label = crate::game::weapon_hud_label(weapon, ammo);
        let label_color = match weapon {
            Some(t) if !t.is_melee() && ammo <= 0 => Color::RED,
            Some(t) => weapon_color(t),
            None => Color::GRAY,
        };
        graphics.draw_text(&label, Vec2::new(100.0, 60.0), 20.0, label_color);
        if let Some(t) = weapon {
            if !t.is_melee() && ammo <= 0 {
                graphics.draw_text("EMPTY - throw it", Vec2::new(10.0, 82.0), 14.0, Color::RED);
            }
        }

        graphics.draw_text("Rogues:", Vec2::new(10.0, 120.0), 20.0, Color::WHITE);
        graphics.draw_text(
            &format!("{}", enemies_alive),
            Vec2::new(120.0, 120.0),
            20.0,
            Color::WHITE,
        );
    } else if !player_alive {
        // Death screen with animations

        // "SYSTEM HALTED" - reveal left to right
        let message = "SYSTEM HALTED";
        let reveal_duration = 1.0; // 1 second to fully reveal
        let reveal_progress = (death_time / reveal_duration).min(1.0);
        let chars_to_show = (message.len() as f32 * reveal_progress) as usize;
        let revealed_text = &message[0..chars_to_show.min(message.len())];

        graphics.draw_text(
            revealed_text,
            Vec2::new(screen_width / 2.0 - 190.0, screen_height / 2.0),
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
                "Press R to reboot",
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
        if show_infos {
            graphics.draw_text(
                "K: purge all rogues / B: crack boss mask (debug)",
                Vec2::new(screen_width - 280.0, 50.0),
                14.0,
                Color::GRAY,
            );
        }
    }

    // Controls info (no weapon-select keys: one weapon in hand, swap on the floor)
    graphics.draw_text(
        "WASD move · Mouse aim · LClick fire · RClick throw · E pick up · Shift look · Esc menu",
        Vec2::new(10.0, screen_height - 20.0),
        16.0,
        Color::GRAY,
    );
}
