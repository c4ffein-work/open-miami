use crate::math::{Color, Vec2};
use crate::player::Player;

#[derive(PartialEq)]
pub enum EnemyState {
    Idle,
    Patrol,
    Chase,
    Attack,
}

pub struct Enemy {
    pub pos: Vec2,
    pub health: i32,
    pub alive: bool,
    pub state: EnemyState,
    pub speed: f32,
    pub radius: f32,
    pub detection_range: f32,
    pub attack_range: f32,
    pub attack_cooldown: f32,
    pub attack_timer: f32,
}

impl Enemy {
    pub fn new(pos: Vec2) -> Self {
        Self {
            pos,
            health: 50,
            alive: true,
            state: EnemyState::Idle,
            speed: 100.0,
            radius: 12.0,
            detection_range: 300.0,
            attack_range: 40.0,
            attack_cooldown: 1.0,
            attack_timer: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32, player_pos: Vec2) {
        if !self.alive {
            return;
        }

        self.attack_timer -= dt;

        let distance_to_player = self.pos.distance(player_pos);

        // State machine
        if distance_to_player < self.attack_range {
            self.state = EnemyState::Attack;
        } else if distance_to_player < self.detection_range {
            self.state = EnemyState::Chase;
        } else {
            self.state = EnemyState::Idle;
        }

        // Move towards player if chasing
        if self.state == EnemyState::Chase || self.state == EnemyState::Attack {
            let dir = (player_pos - self.pos).normalize();

            // Stop at attack range
            if distance_to_player > self.attack_range + 5.0 {
                self.pos += dir * self.speed * dt;
            }
        }
    }

    pub fn render(&self) {
        // Legacy rendering code - stubbed out
        // Would draw enemy circle with state-based colors and health bar
        // Now handled by the ECS rendering system
        let _color = match self.state {
            EnemyState::Idle => Color::from_rgba(100, 100, 200, 255),
            EnemyState::Patrol => Color::from_rgba(100, 150, 200, 255),
            EnemyState::Chase => Color::from_rgba(200, 150, 100, 255),
            EnemyState::Attack => Color::from_rgba(200, 100, 100, 255),
        };
    }

    pub fn try_attack_player(&mut self, player: &mut Player) {
        if !self.alive || self.state != EnemyState::Attack {
            return;
        }

        if self.attack_timer <= 0.0 {
            player.take_damage(20);
            self.attack_timer = self.attack_cooldown;
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
