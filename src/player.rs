use crate::enemy::Enemy;
use crate::weapon::{Weapon, WeaponType};
use macroquad::prelude::*;

pub struct Player {
    pub pos: Vec2,
    pub health: i32,
    pub alive: bool,
    pub speed: f32,
    pub weapon: Weapon,
    pub rotation: f32,
}

impl Player {
    pub fn new(pos: Vec2) -> Self {
        Self {
            pos,
            health: 100,
            alive: true,
            speed: 200.0,
            weapon: Weapon::new(WeaponType::Pistol),
            rotation: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32) {
        if !self.alive {
            return;
        }

        let mut movement = Vec2::ZERO;

        if is_key_down(KeyCode::W) {
            movement.y -= 1.0;
        }
        if is_key_down(KeyCode::S) {
            movement.y += 1.0;
        }
        if is_key_down(KeyCode::A) {
            movement.x -= 1.0;
        }
        if is_key_down(KeyCode::D) {
            movement.x += 1.0;
        }

        if movement != Vec2::ZERO {
            movement = movement.normalize();
            self.pos += movement * self.speed * dt;
        }
    }

    pub fn render(&mut self, mouse_world_pos: Vec2) {
        if !self.alive {
            return;
        }

        // Calculate rotation to face mouse
        let dir = mouse_world_pos - self.pos;
        self.rotation = dir.y.atan2(dir.x);

        // Draw player body
        draw_circle(
            self.pos.x,
            self.pos.y,
            15.0,
            Color::from_rgba(255, 100, 100, 255),
        );

        // Draw direction indicator
        let indicator_end = self.pos + Vec2::new(self.rotation.cos(), self.rotation.sin()) * 20.0;
        draw_line(
            self.pos.x,
            self.pos.y,
            indicator_end.x,
            indicator_end.y,
            3.0,
            WHITE,
        );
    }

    pub fn shoot(&mut self, target_pos: Vec2, enemies: &mut Vec<Enemy>) {
        if !self.alive || self.weapon.ammo <= 0 {
            return;
        }

        self.weapon.ammo -= 1;

        // Calculate bullet direction
        let dir = (target_pos - self.pos).normalize();
        let bullet_range = 1000.0;

        // Check if bullet hits any enemy
        for enemy in enemies.iter_mut() {
            if !enemy.alive {
                continue;
            }

            // Simple line-circle collision
            let to_enemy = enemy.pos - self.pos;
            let projection = to_enemy.dot(dir);

            if projection > 0.0 && projection < bullet_range {
                let closest_point = self.pos + dir * projection;
                let distance = (enemy.pos - closest_point).length();

                if distance < enemy.radius + 5.0 {
                    enemy.take_damage(self.weapon.damage);
                    break; // Bullet stops at first enemy
                }
            }
        }

        // Visual feedback - draw bullet trail
        let end_pos = self.pos + dir * bullet_range;
        draw_line(
            self.pos.x,
            self.pos.y,
            end_pos.x,
            end_pos.y,
            2.0,
            Color::from_rgba(255, 255, 100, 100),
        );
    }

    pub fn melee_attack(&mut self, target_pos: Vec2, enemies: &mut Vec<Enemy>) {
        if !self.alive {
            return;
        }

        let melee_range = 50.0;
        let dir = (target_pos - self.pos).normalize();

        // Check enemies in melee range
        for enemy in enemies.iter_mut() {
            if !enemy.alive {
                continue;
            }

            let to_enemy = enemy.pos - self.pos;
            let distance = to_enemy.length();

            if distance < melee_range {
                // Check if enemy is roughly in the direction we're attacking
                let angle = to_enemy.normalize().dot(dir);
                if angle > 0.5 {
                    enemy.take_damage(50);
                }
            }
        }

        // Visual feedback - draw attack arc
        for i in 0..8 {
            let angle = self.rotation - 0.5 + (i as f32 * 0.125);
            let end = self.pos + Vec2::new(angle.cos(), angle.sin()) * melee_range;
            draw_line(
                self.pos.x,
                self.pos.y,
                end.x,
                end.y,
                3.0,
                Color::from_rgba(255, 100, 100, 50),
            );
        }
    }

    pub fn take_damage(&mut self, damage: i32) {
        self.health -= damage;
        if self.health <= 0 {
            self.health = 0;
            self.alive = false;
        }
    }
}
