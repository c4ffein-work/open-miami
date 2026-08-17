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

impl WeaponType {
    /// Rounds in a full magazine. Melee has no magazine (see
    /// [`WeaponType::is_melee`]); its value here is only a nominal cap.
    pub const fn magazine(self) -> i32 {
        match self {
            WeaponType::Pistol => 12,
            WeaponType::Shotgun => 6,
            WeaponType::MachineGun => 30,
            WeaponType::Melee => 999,
        }
    }

    /// Melee weapons never run dry.
    pub const fn is_melee(self) -> bool {
        matches!(self, WeaponType::Melee)
    }
}

/// A weapon lying on the floor (placed by the level, dropped by a downed rogue,
/// left behind by a swap, or landed after a throw). Hotline Miami rules: the
/// player holds exactly one weapon, and the ammo lives ON the weapon — this
/// pickup carries the rounds still in it, and picking it up (E) hands the player
/// that weapon with exactly that ammo, while their previous weapon is dropped in
/// its place with *its* remaining ammo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponPickup {
    pub weapon_type: WeaponType,
    /// Rounds left in the weapon (ignored for melee).
    pub ammo: i32,
}

impl WeaponPickup {
    /// A pickup holding a full magazine (level-placed weapons).
    pub fn new(weapon_type: WeaponType) -> Self {
        WeaponPickup {
            weapon_type,
            ammo: weapon_type.magazine(),
        }
    }

    /// A pickup holding a specific amount of ammo (drops, swaps, landed throws).
    pub fn with_ammo(weapon_type: WeaponType, ammo: i32) -> Self {
        WeaponPickup {
            weapon_type,
            ammo: ammo.clamp(0, weapon_type.magazine()),
        }
    }
}

/// A weapon in mid-flight after being thrown. It carries its own velocity
/// (rather than a `Velocity` component) so only the thrown-weapon system moves
/// it. When it lands it becomes a [`WeaponPickup`]; if it hits an enemy first it
/// knocks them down ([`Stunned`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThrownWeapon {
    pub weapon_type: WeaponType,
    /// Rounds still in the weapon; carried through to the pickup it lands as.
    pub ammo: i32,
    pub damage: i32,
    pub vx: f32,
    pub vy: f32,
    /// How much further (in pixels) it can travel before dropping to the floor.
    pub distance_remaining: f32,
    /// Visual spin, in radians (rendering only).
    pub spin: f32,
}

impl ThrownWeapon {
    pub fn new(
        weapon_type: WeaponType,
        ammo: i32,
        damage: i32,
        dir: Vec2,
        speed: f32,
        range: f32,
    ) -> Self {
        let d = dir.normalize();
        ThrownWeapon {
            weapon_type,
            ammo,
            damage,
            vx: d.x * speed,
            vy: d.y * speed,
            distance_remaining: range,
            spin: 0.0,
        }
    }
}

/// Marks the shoggoth boss. It wears a friendly smiley mask until enough damage
/// cracks it off, at which point it enrages (faster, deadlier). The lore beat:
/// its mask *does* come off — unlike the player's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Boss {
    /// False while the smiley mask is intact; true once cracked (enraged phase).
    pub enraged: bool,
    /// Mask-off progress for the live render, 0..1: stays 0 while masked, runs
    /// 0 -> 1 over `BOSS_MASK_OFF_SECS` once `enraged` flips (the mask cracks
    /// and is consumed inward while the tentacles grow), then holds at 1.
    pub reveal: f32,
}

impl Boss {
    pub fn new() -> Self {
        Boss {
            enraged: false,
            reveal: 0.0,
        }
    }
}

impl Default for Boss {
    fn default() -> Self {
        Self::new()
    }
}

/// Marks an entity as knocked down / incapacitated for a limited time. While an
/// enemy is stunned it does not move, chase, or attack, and can be finished off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stunned {
    pub timer: f32,
}

impl Stunned {
    pub fn new(duration: f32) -> Self {
        Stunned { timer: duration }
    }

    pub fn is_active(&self) -> bool {
        self.timer > 0.0
    }
}

/// A short-lived directional impulse layered on top of an entity's normal
/// motion — the "shove" from taking a hit. It carries its own velocity (px/s)
/// that the movement system adds to the entity's displacement and then decays
/// toward zero over a fraction of a second, so a hit produces a quick punch of
/// motion rather than a sustained push. Applied to enemies (thrown opposite the
/// incoming blow) and to the player (shoved directly away from the attacker).
/// It is resolved through the same wall/bounds clamping as ordinary movement, so
/// a shove can never push an entity through a wall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knockback {
    /// Remaining impulse velocity in pixels/second.
    pub x: f32,
    pub y: f32,
}

impl Knockback {
    pub fn new(x: f32, y: f32) -> Self {
        Knockback { x, y }
    }

    /// Magnitude of the remaining impulse (px/s).
    pub fn magnitude(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

/// The one weapon an entity holds. The ammo is a property of the weapon
/// itself: it travels with it when dropped, swapped, thrown, or picked up
/// (see [`WeaponPickup`], [`ThrownWeapon`]). Nothing ever refills a weapon;
/// an empty gun can only be thrown. Melee never runs dry.
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
    /// The weapon that fired it (drives the per-weapon hit sound).
    pub weapon_type: WeaponType,
    pub damage: i32,
    pub speed: f32,
    pub lifetime: f32,
    pub max_lifetime: f32,
}

impl Bullet {
    pub fn new(weapon_type: WeaponType, damage: i32) -> Self {
        Bullet {
            weapon_type,
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
    /// A weapon with a full magazine.
    pub fn new(weapon_type: WeaponType) -> Self {
        Self::with_ammo(weapon_type, weapon_type.magazine())
    }

    /// A weapon holding `ammo` rounds (clamped to its magazine).
    pub fn with_ammo(weapon_type: WeaponType, ammo: i32) -> Self {
        let (damage, fire_rate) = match weapon_type {
            WeaponType::Pistol => (50, 0.5),
            WeaponType::Shotgun => (80, 1.0),
            WeaponType::MachineGun => (30, 0.1),
            WeaponType::Melee => (100, 0.5),
        };
        let max_ammo = weapon_type.magazine();

        Weapon {
            weapon_type,
            damage,
            ammo: ammo.clamp(0, max_ammo),
            max_ammo,
            fire_rate,
            fire_timer: 0.0,
        }
    }

    /// Melee never runs out; guns need at least one round.
    pub fn has_ammo(&self) -> bool {
        self.weapon_type.is_melee() || self.ammo > 0
    }

    /// Off cooldown and has something to fire.
    pub fn can_fire(&self) -> bool {
        self.fire_timer <= 0.0 && self.has_ammo()
    }

    /// Off cooldown but empty: pulling the trigger only clicks.
    pub fn is_dry(&self) -> bool {
        self.fire_timer <= 0.0 && !self.has_ammo()
    }

    /// Spend a round (none for melee) and start the cooldown.
    pub fn fire(&mut self) {
        if self.can_fire() {
            if !self.weapon_type.is_melee() {
                self.ammo -= 1;
            }
            self.fire_timer = self.fire_rate;
        }
    }

    pub fn update(&mut self, dt: f32) {
        if self.fire_timer > 0.0 {
            self.fire_timer -= dt;
        }
    }
}

/// Bookkeeping tag: this enemy's death has already been noticed (its
/// [`GameEvent::EnemyDown`] was emitted). Added by the pickup/downed pass the
/// first frame the enemy's health is zero, whatever caused it (bullets, melee,
/// a thrown weapon, a feral burning out, a scenario purge...).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Downed;

/// One-shot gameplay events, pushed by the systems where the truth happens
/// (a shot fired, a hit landed, a rogue downed...) and drained once per frame
/// by the browser layer to trigger sound effects. See
/// [`crate::ecs::World::push_event`] / [`crate::ecs::World::drain_events`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GameEvent {
    /// The player fired (or swung) their weapon; one per round.
    PlayerFired(WeaponType),
    /// The player pulled the trigger on an empty gun.
    DryFire,
    /// A player attack connected with an enemy.
    EnemyHit { by: WeaponType },
    /// An enemy's health just reached zero.
    EnemyDown,
    /// The player took damage (and survived the frame or not).
    PlayerHurt,
    /// The player picked a weapon up off the floor.
    Pickup,
    /// The player threw their weapon.
    Throw,
    /// A thrown weapon struck an enemy (knockdown).
    ThrownImpact,
}

/// Debug component to store pathfinding waypoints for visualization
#[derive(Debug, Clone, PartialEq)]
pub struct DebugPath {
    pub waypoints: Vec<Vec2>,
    pub target: Vec2,
}

impl DebugPath {
    pub fn new(waypoints: Vec<Vec2>, target: Vec2) -> Self {
        DebugPath { waypoints, target }
    }

    pub fn clear() -> Self {
        DebugPath {
            waypoints: Vec::new(),
            target: Vec2::zero(),
        }
    }
}

/// Debug component to store actual movement trail for visualization
#[derive(Debug, Clone, PartialEq)]
pub struct DebugTrail {
    pub positions: Vec<Vec2>,
    pub max_length: usize,
}

impl DebugTrail {
    pub fn new(max_length: usize) -> Self {
        DebugTrail {
            positions: Vec::new(),
            max_length,
        }
    }

    /// Add a new position to the trail
    pub fn add_position(&mut self, pos: Vec2) {
        self.positions.push(pos);
        // Keep only the last N positions
        if self.positions.len() > self.max_length {
            self.positions.remove(0);
        }
    }

    /// Clear the trail
    pub fn clear(&mut self) {
        self.positions.clear();
    }
}

impl Default for DebugTrail {
    fn default() -> Self {
        Self::new(100) // Store last 100 positions
    }
}

/// An elevator car on the floor: the entry you arrived in, or an exit you can
/// leave by. Exits extract the player when open (see
/// `systems::elevator::ElevatorSystem`); the entry is the spawn point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elevator {
    pub id: &'static str,
    pub label: &'static str,
    /// Door frame rectangle in world units.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// `true` for exits, `false` for the entry car.
    pub is_exit: bool,
    /// Exits only: whether the doors are open (extractable).
    pub open: bool,
    /// Exits only: floor id this car leads to (`0` = the surface).
    pub to: usize,
    /// Seconds the player has been standing inside an open exit.
    pub dwell: f32,
}

impl Elevator {
    pub fn from_def(def: &crate::scenario::ElevatorDef, is_exit: bool) -> Self {
        Elevator {
            id: def.id,
            label: def.label,
            x: def.rect.x,
            y: def.rect.y,
            w: def.rect.w,
            h: def.rect.h,
            is_exit,
            open: is_exit && def.open,
            to: def.to,
            dwell: 0.0,
        }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// A named trigger region (`enter_zone` in scenarios). No collision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub id: &'static str,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Zone {
    pub fn from_def(def: &crate::scenario::ZoneDef) -> Self {
        Zone {
            id: def.id,
            x: def.rect.x,
            y: def.rect.y,
            w: def.rect.w,
            h: def.rect.h,
        }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
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
    fn test_weapon_melee_never_runs_dry() {
        let mut weapon = Weapon::new(WeaponType::Melee);
        let before = weapon.ammo;
        weapon.fire();
        assert_eq!(weapon.ammo, before); // no round spent
        weapon.fire_timer = 0.0;
        assert!(weapon.can_fire());
        assert!(!Weapon::with_ammo(WeaponType::Melee, 0).is_dry());
    }

    #[test]
    fn test_weapon_with_ammo_clamps_to_magazine() {
        assert_eq!(Weapon::with_ammo(WeaponType::Pistol, 5).ammo, 5);
        assert_eq!(Weapon::with_ammo(WeaponType::Pistol, 99).ammo, 12);
        assert_eq!(Weapon::with_ammo(WeaponType::Shotgun, -3).ammo, 0);
        assert_eq!(WeaponPickup::with_ammo(WeaponType::MachineGun, 7).ammo, 7);
        assert_eq!(WeaponPickup::new(WeaponType::Shotgun).ammo, 6);
    }

    #[test]
    fn test_weapon_is_dry() {
        let mut weapon = Weapon::with_ammo(WeaponType::Pistol, 1);
        assert!(!weapon.is_dry());
        weapon.fire();
        assert!(!weapon.is_dry()); // still on cooldown
        weapon.fire_timer = 0.0;
        assert!(weapon.is_dry());
        assert!(!weapon.can_fire());
    }

    #[test]
    fn test_velocity_zero() {
        let vel = Velocity::zero();
        assert_eq!(vel.x, 0.0);
        assert_eq!(vel.y, 0.0);
    }
}
