// Game Components - Pure data structures
use crate::math::Vec2;

/// Position in 2D space
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    pub fn new(x: f32, y: f32) -> Self {
        Position { x, y }
    }

    pub fn from_vec2(vec: Vec2) -> Self {
        Position { x: vec.x, y: vec.y }
    }

    pub fn to_vec2(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    pub fn distance_to(&self, other: &Position) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Velocity for movement
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}

impl Velocity {
    pub fn new(x: f32, y: f32) -> Self {
        Velocity { x, y }
    }

    pub fn zero() -> Self {
        Velocity { x: 0.0, y: 0.0 }
    }
}

/// Health component
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Health {
    pub current: i32,
    pub max: i32,
}

impl Health {
    pub fn new(max: i32) -> Self {
        Health { current: max, max }
    }

    pub fn take_damage(&mut self, damage: i32) {
        self.current = (self.current - damage).max(0);
    }

    pub fn is_alive(&self) -> bool {
        self.current > 0
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0
    }
}

/// Speed component
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed {
    pub value: f32,
}

impl Speed {
    pub fn new(value: f32) -> Self {
        Speed { value }
    }
}

/// Rotation/facing direction in radians
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    pub angle: f32,
}

impl Rotation {
    pub fn new(angle: f32) -> Self {
        Rotation { angle }
    }
}

/// Circular collision radius
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radius {
    pub value: f32,
}

impl Radius {
    pub fn new(value: f32) -> Self {
        Radius { value }
    }
}

/// Tag component for the player
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player;

/// Tag component for enemies
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Enemy;

/// Enemy initial behavior type (determines color and patrol behavior)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnemyType {
    Idle,       // Red - stays in place when unaware
    Wandering,  // Yellow - wanders around spawn area
    Patrolling, // Green - patrols around spawn area (same as wandering for now)
}

/// Enemy AI state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AIState {
    /// Never saw the player
    Unaware,
    /// Has briefly seen the player, will go check then return to spot
    SpottedUnsure,
    /// Sure that the player has been seen - chase and attack
    SurePlayerSeen,
    /// Looking around for player, then transitions based on initial type
    Confused,
    // Legacy states for compatibility (will be removed)
    #[allow(dead_code)]
    Idle,
    #[allow(dead_code)]
    Patrol,
    #[allow(dead_code)]
    Chase,
    #[allow(dead_code)]
    Attack,
}

/// AI component for enemies
#[derive(Debug, Clone, PartialEq)]
pub struct AI {
    pub state: AIState,
    pub initial_type: EnemyType,
    pub spawn_position: Position,
    pub last_known_player_position: Option<Position>,
    pub check_position: Option<Position>, // Where enemy was when first spotted player
    pub detection_range: f32,
    pub attack_range: f32,
    pub attack_cooldown: f32,
    pub attack_timer: f32,

    // State transition timers
    pub state_timer: f32,
    pub spot_duration: f32, // How long to see player before becoming unsure (0.3s)
    pub unsure_check_duration: f32, // How long to check before returning (2.0s)
    pub lost_player_duration: f32, // How long at last known position before confused (3.0s)
    pub confusion_duration: f32, // How long to look around (3.0s)

    // Confusion state
    pub confusion_look_timer: f32,
    pub confusion_looks_remaining: i32,
    pub confusion_look_duration: f32, // 0.3s per look

    // Wandering/Patrolling behavior
    pub wander_timer: f32,
    pub wander_look_timer: f32,
    pub wander_state: WanderState,
    pub wander_direction: f32,     // Current movement angle in radians
    pub movement_square_size: f32, // 150 pixels from spawn
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WanderState {
    Moving,
    LookingAround,
    Waiting,
}

impl AI {
    pub fn new() -> Self {
        AI {
            state: AIState::Unaware,
            initial_type: EnemyType::Idle,
            spawn_position: Position::new(0.0, 0.0),
            last_known_player_position: None,
            check_position: None,
            detection_range: 900.0,
            attack_range: 40.0,
            attack_cooldown: 1.0,
            attack_timer: 0.0,
            state_timer: 0.0,
            spot_duration: 0.3,
            unsure_check_duration: 2.0,
            lost_player_duration: 3.0,
            confusion_duration: 3.0,
            confusion_look_timer: 0.0,
            confusion_looks_remaining: 0,
            confusion_look_duration: 0.3,
            wander_timer: 0.0,
            wander_look_timer: 0.0,
            wander_state: WanderState::Waiting,
            wander_direction: 0.0,
            movement_square_size: 150.0,
        }
    }

    pub fn new_with_type(enemy_type: EnemyType, spawn_pos: Position) -> Self {
        let mut ai = Self::new();
        ai.initial_type = enemy_type;
        ai.spawn_position = spawn_pos;
        ai
    }

    pub fn can_attack(&self) -> bool {
        self.attack_timer <= 0.0
    }

    pub fn reset_attack_timer(&mut self) {
        self.attack_timer = self.attack_cooldown;
    }
}

impl Default for AI {
    fn default() -> Self {
        Self::new()
    }
}

impl Copy for AI {}

/// Weapon type enum
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WeaponType {
    Pistol,
    Shotgun,
    MachineGun,
    Melee,
}

/// Weapon component
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weapon {
    pub weapon_type: WeaponType,
    pub damage: i32,
    pub ammo: i32,
    pub max_ammo: i32,
    pub fire_rate: f32,
    pub fire_timer: f32,
}

/// Projectile trail for visual feedback
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileTrail {
    pub start: Position,
    pub end: Position,
    pub lifetime: f32,
    pub max_lifetime: f32,
}

impl ProjectileTrail {
    pub fn new(start: Position, end: Position) -> Self {
        ProjectileTrail {
            start,
            end,
            lifetime: 0.15, // 150ms trail
            max_lifetime: 0.15,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.lifetime > 0.0
    }

    pub fn alpha(&self) -> f32 {
        self.lifetime / self.max_lifetime
    }
}

/// Physical bullet projectile
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bullet {
    pub damage: i32,
    pub speed: f32,
    pub lifetime: f32,
    pub max_lifetime: f32,
}

impl Bullet {
    pub fn new(damage: i32) -> Self {
        Bullet {
            damage,
            speed: 800.0,  // pixels per second
            lifetime: 3.0, // 3 seconds max lifetime
            max_lifetime: 3.0,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.lifetime > 0.0
    }
}

impl Weapon {
    pub fn new(weapon_type: WeaponType) -> Self {
        let (damage, max_ammo, fire_rate) = match weapon_type {
            WeaponType::Pistol => (50, 12, 0.5),
            WeaponType::Shotgun => (80, 6, 1.0),
            WeaponType::MachineGun => (30, 30, 0.1),
            WeaponType::Melee => (100, 999, 0.5),
        };

        Weapon {
            weapon_type,
            damage,
            ammo: max_ammo,
            max_ammo,
            fire_rate,
            fire_timer: 0.0,
        }
    }

    pub fn can_fire(&self) -> bool {
        self.fire_timer <= 0.0 && self.ammo > 0
    }

    pub fn fire(&mut self) {
        if self.can_fire() {
            self.ammo -= 1;
            self.fire_timer = self.fire_rate;
        }
    }

    pub fn update(&mut self, dt: f32) {
        if self.fire_timer > 0.0 {
            self.fire_timer -= dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_distance() {
        let p1 = Position::new(0.0, 0.0);
        let p2 = Position::new(3.0, 4.0);
        assert_eq!(p1.distance_to(&p2), 5.0);
    }

    #[test]
    fn test_position_vec2_conversion() {
        let pos = Position::new(10.0, 20.0);
        let vec = pos.to_vec2();
        let pos2 = Position::from_vec2(vec);
        assert_eq!(pos, pos2);
    }

    #[test]
    fn test_health_take_damage() {
        let mut health = Health::new(100);
        assert!(health.is_alive());

        health.take_damage(30);
        assert_eq!(health.current, 70);
        assert!(health.is_alive());

        health.take_damage(80);
        assert_eq!(health.current, 0);
        assert!(health.is_dead());
    }

    #[test]
    fn test_health_cannot_go_negative() {
        let mut health = Health::new(50);
        health.take_damage(100);
        assert_eq!(health.current, 0);
    }

    #[test]
    fn test_ai_state_transitions() {
        let mut ai = AI::new();
        assert_eq!(ai.state, AIState::Unaware);

        ai.state = AIState::SurePlayerSeen;
        assert_eq!(ai.state, AIState::SurePlayerSeen);
    }

    #[test]
    fn test_ai_attack_cooldown() {
        let mut ai = AI::new();
        assert!(ai.can_attack());

        ai.reset_attack_timer();
        assert!(!ai.can_attack());

        ai.attack_timer = 0.0;
        assert!(ai.can_attack());
    }

    #[test]
    fn test_weapon_pistol_stats() {
        let weapon = Weapon::new(WeaponType::Pistol);
        assert_eq!(weapon.damage, 50);
        assert_eq!(weapon.max_ammo, 12);
        assert_eq!(weapon.ammo, 12);
        assert_eq!(weapon.fire_rate, 0.5);
    }

    #[test]
    fn test_weapon_shotgun_stats() {
        let weapon = Weapon::new(WeaponType::Shotgun);
        assert_eq!(weapon.damage, 80);
        assert_eq!(weapon.max_ammo, 6);
    }

    #[test]
    fn test_weapon_fire() {
        let mut weapon = Weapon::new(WeaponType::Pistol);
        assert!(weapon.can_fire());

        weapon.fire();
        assert_eq!(weapon.ammo, 11);
        assert!(!weapon.can_fire()); // Fire timer active

        weapon.fire_timer = 0.0;
        assert!(weapon.can_fire());
    }

    #[test]
    fn test_weapon_empty_ammo() {
        let mut weapon = Weapon::new(WeaponType::Pistol);
        weapon.ammo = 0;
        assert!(!weapon.can_fire());
    }

    #[test]
    fn test_weapon_update_timer() {
        let mut weapon = Weapon::new(WeaponType::Pistol);
        weapon.fire();

        assert_eq!(weapon.fire_timer, 0.5);

        weapon.update(0.3);
        assert!((weapon.fire_timer - 0.2).abs() < 0.001); // Use approximate comparison for floats

        weapon.update(0.3);
        assert!((weapon.fire_timer - (-0.1)).abs() < 0.001);
        assert!(weapon.can_fire()); // Can fire again after cooldown
    }

    #[test]
    fn test_velocity_zero() {
        let vel = Velocity::zero();
        assert_eq!(vel.x, 0.0);
        assert_eq!(vel.y, 0.0);
    }
}
