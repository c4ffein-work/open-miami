// Game setup and entity spawning helpers
use crate::components::*;
use crate::ecs::{Entity, World};
use crate::levels::{floor_def, BOSS_LEVEL};
use crate::math::Vec2;
use crate::scenario::{spawn_floor_markers, spawn_from_def};
use crate::systems::boss::{BOSS_ATTACK_RANGE, BOSS_MASK_SPEED, BOSS_MAX_HEALTH, BOSS_RADIUS};
use crate::systems::combat::CombatSystem;

/// Where the shoggoth boss stands at the start of its floor.
pub const BOSS_SPAWN: Vec2 = Vec2::new(400.0, 560.0);

/// Spawn the shoggoth boss (a big, tanky, masked enemy) at `position`.
pub fn spawn_boss(world: &mut World, position: Vec2) -> Entity {
    let entity = world.spawn();
    let pos = Position::from_vec2(position);

    world.add_component(entity, Enemy);
    world.add_component(entity, Boss::new());
    world.add_component(entity, pos);
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(BOSS_MASK_SPEED));
    world.add_component(entity, Health::new(BOSS_MAX_HEALTH));
    world.add_component(entity, Radius::new(BOSS_RADIUS));
    world.add_component(entity, Rotation::new(0.0));

    let mut ai = AI::new();
    ai.attack_range = BOSS_ATTACK_RANGE;
    ai.detection_range = 5000.0;
    world.add_component(entity, ai);

    entity
}

/// Spawn a player entity
pub fn spawn_player(world: &mut World, position: Vec2) -> Entity {
    let entity = world.spawn();

    world.add_component(entity, Player);
    world.add_component(entity, Position::from_vec2(position));
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(200.0));
    world.add_component(entity, Health::new(100));
    world.add_component(entity, Rotation::new(0.0));
    world.add_component(entity, Radius::new(15.0));
    world.add_component(entity, Weapon::new(WeaponType::Pistol));

    entity
}

/// The weapon an enemy of a given type carries (and drops when killed).
/// All enemy weapons are firearms so a dropped pickup is always an upgrade in
/// ammo rather than a downgrade to fists.
pub fn weapon_for_enemy(enemy_type: EnemyType) -> WeaponType {
    match enemy_type {
        EnemyType::Idle => WeaponType::Pistol,
        EnemyType::Wandering => WeaponType::MachineGun,
        EnemyType::Patrolling => WeaponType::Shotgun,
    }
}

/// Spawn an enemy entity with a specific type
pub fn spawn_enemy_with_type(world: &mut World, position: Vec2, enemy_type: EnemyType) -> Entity {
    let entity = world.spawn();
    let pos = Position::from_vec2(position);

    world.add_component(entity, Enemy);
    world.add_component(entity, pos);
    world.add_component(entity, Velocity::zero());
    world.add_component(entity, Speed::new(100.0));
    world.add_component(entity, Health::new(50));
    world.add_component(entity, Radius::new(12.0));
    world.add_component(entity, Rotation::new(0.0));
    world.add_component(entity, AI::new_with_type(enemy_type, pos));
    world.add_component(entity, Weapon::new(weapon_for_enemy(enemy_type)));

    entity
}

/// Spawn an enemy entity (default to Idle type for backwards compatibility)
pub fn spawn_enemy(world: &mut World, position: Vec2) -> Entity {
    spawn_enemy_with_type(world, position, EnemyType::Idle)
}

/// Spawn a weapon lying on the floor with a full magazine (level-placed
/// pickups; collected with the pick-up key).
pub fn spawn_pickup(world: &mut World, position: Vec2, weapon_type: WeaponType) -> Entity {
    spawn_pickup_with_ammo(world, position, weapon_type, weapon_type.magazine())
}

/// Spawn a weapon lying on the floor holding `ammo` rounds (drops, swaps and
/// landed throws — the ammo travels with the weapon).
pub fn spawn_pickup_with_ammo(
    world: &mut World,
    position: Vec2,
    weapon_type: WeaponType,
    ammo: i32,
) -> Entity {
    let entity = world.spawn();
    world.add_component(entity, WeaponPickup::with_ammo(weapon_type, ammo));
    world.add_component(entity, Position::from_vec2(position));
    world.add_component(entity, Radius::new(14.0));
    entity
}

/// Initialize a new game world with the player and a floor's designed layout:
/// walls, the initial rogues, floor pickups, the entry/exit elevators and the
/// trigger zones. The player spawns in the entry elevator. (Scenario steps —
/// dialogue, waves, door openings — are driven by `scenario::ScenarioState`,
/// which the caller owns.)
pub fn initialize_game(world: &mut World, level: usize) {
    let floor = floor_def(level);

    // Spawn the player in the entry car.
    spawn_player(world, floor.player_spawn());

    for wall in floor.walls {
        world.add_wall(wall.x, wall.y, wall.w, wall.h);
    }

    for s in floor.spawns {
        spawn_from_def(world, s);
    }

    for p in floor.pickups {
        spawn_pickup(world, Vec2::new(p.x, p.y), p.weapon);
    }

    spawn_floor_markers(world, floor);

    // The hidden final floor: the shoggoth waits below.
    if level == BOSS_LEVEL {
        spawn_boss(world, BOSS_SPAWN);
    }
}

/// Zero the player's velocity (a scenario `hold` locks movement: the input
/// system is skipped, so the last frame's velocity must not linger).
pub fn stop_player(world: &mut World) {
    if let Some(&p) = world.query::<Player>().first() {
        if let Some(v) = world.get_component_mut::<Velocity>(p) {
            v.x = 0.0;
            v.y = 0.0;
        }
    }
}

/// Debug helper: down every rogue on the floor (used by the debug **K** key
/// and by tests to exercise the all-dead / extraction flow quickly).
pub fn purge_all_enemies(world: &mut World) {
    for entity in world.query::<Enemy>() {
        if let Some(h) = world.get_component_mut::<Health>(entity) {
            h.take_damage(h.max.max(1));
        }
    }
}

/// Check if player is alive
pub fn is_player_alive(world: &World) -> bool {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Health>(e))
        .map(|h| h.is_alive())
        .unwrap_or(false)
}

/// Get player health for UI
pub fn get_player_health(world: &World) -> i32 {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Health>(e))
        .map(|h| h.current)
        .unwrap_or(0)
}

/// Get player ammo for UI
pub fn get_player_ammo(world: &World) -> i32 {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Weapon>(e))
        .map(|w| w.ammo)
        .unwrap_or(0)
}

/// Cadence of the bare-fist punch (seconds between jabs).
pub const PUNCH_COOLDOWN: f32 = 0.45;

/// An UNARMED left click: a bare-fist jab that KNOCKS the struck enemy DOWN
/// (chip damage + a ~3 s sprawl — see `CombatSystem::process_punch`) instead
/// of dealing real damage. Paced by the [`Fists`] cooldown component (ticked
/// by the weapon-update system). Emits [`GameEvent::PlayerFired`] (melee — the
/// swing whoosh) per jab; `process_punch` emits the per-enemy impacts.
fn punch_from_player(
    world: &mut World,
    player: Entity,
    player_pos: Position,
    target_pos: Position,
) -> bool {
    let ready = world
        .get_component::<Fists>(player)
        .map(|f| f.ready())
        .unwrap_or(true);
    if !ready {
        return false;
    }
    match world.get_component_mut::<Fists>(player) {
        Some(f) => f.timer = PUNCH_COOLDOWN,
        None => world.add_component(player, Fists::new(PUNCH_COOLDOWN)),
    }
    world.push_event(GameEvent::PlayerFired(WeaponType::Melee));
    CombatSystem::process_punch(
        world,
        player_pos,
        target_pos,
        crate::systems::combat::PUNCH_DAMAGE,
        crate::systems::combat::PUNCH_RANGE,
    )
}

/// Fire the player's currently held weapon toward a world position. Melee hits
/// instantly (returns whether it connected); ranged weapons spawn a bullet
/// entity (returns `false`, since bullets resolve asynchronously). An UNARMED
/// player throws a bare-fist punch that knocks its target down (see
/// [`punch_from_player`]). Does nothing if the weapon is on cooldown; an empty
/// gun only clicks (a [`GameEvent::DryFire`] once per would-be shot — the
/// cooldown is restarted so a held trigger does not spam it) — nothing ever
/// refills it: throw it and take another. Emits [`GameEvent::PlayerFired`] for
/// every round that leaves.
///
/// This is the input-independent core of shooting so it can be driven both by
/// the browser input layer and by the headless [`crate::sim::Simulation`].
pub fn fire_player_weapon(world: &mut World, target_world_pos: Vec2) -> bool {
    let player = match world.query::<Player>().first() {
        Some(&e) => e,
        None => return false,
    };
    let player_pos = match world.get_component::<Position>(player) {
        Some(p) => *p,
        None => return false,
    };
    let (weapon_type, damage, can_fire, is_dry) = match world.get_component::<Weapon>(player) {
        Some(w) => (w.weapon_type, w.damage, w.can_fire(), w.is_dry()),
        None => {
            // Unarmed: swing a fist instead.
            let target = Position::from_vec2(target_world_pos);
            return punch_from_player(world, player, player_pos, target);
        }
    };

    if is_dry {
        // Click. Restart the cooldown so a held trigger clicks at the weapon's
        // fire cadence rather than every frame.
        if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
            weapon.fire_timer = weapon.fire_rate;
        }
        world.push_event(GameEvent::DryFire);
        return false;
    }
    if !can_fire {
        return false;
    }

    if let Some(weapon) = world.get_component_mut::<Weapon>(player) {
        weapon.fire();
    }
    world.push_event(GameEvent::PlayerFired(weapon_type));

    let target_pos = Position::from_vec2(target_world_pos);

    if weapon_type.is_melee() {
        CombatSystem::process_melee(world, player_pos, target_pos, damage, 50.0)
    } else {
        let dx = target_pos.x - player_pos.x;
        let dy = target_pos.y - player_pos.y;
        let length = (dx * dx + dy * dy).sqrt();

        let bullet = Bullet::new(weapon_type, damage);
        let bullet_speed = bullet.speed;
        let (vel_x, vel_y) = if length > 0.0 {
            (dx / length * bullet_speed, dy / length * bullet_speed)
        } else {
            (0.0, 0.0)
        };

        let bullet_entity = world.spawn();
        world.add_component(bullet_entity, bullet);
        world.add_component(bullet_entity, player_pos);
        world.add_component(bullet_entity, Velocity::new(vel_x, vel_y));
        world.add_component(bullet_entity, Radius::new(2.0));

        false
    }
}

/// Get the player's current weapon type (for the HUD); `None` when unarmed.
pub fn get_player_weapon(world: &World) -> Option<WeaponType> {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Weapon>(e))
        .map(|w| w.weapon_type)
}

/// Human-readable name for a weapon type
pub fn weapon_name(weapon_type: WeaponType) -> &'static str {
    match weapon_type {
        WeaponType::Pistol => "Pistol",
        WeaponType::Shotgun => "Shotgun",
        WeaponType::MachineGun => "Machine Gun",
        WeaponType::Melee => "Melee",
    }
}

/// HUD label for the held weapon: `SHOTGUN 3/6`, `MELEE` (infinite), or
/// `UNARMED` when the player holds nothing.
pub fn weapon_hud_label(weapon: Option<WeaponType>, ammo: i32) -> String {
    match weapon {
        None => "UNARMED".to_string(),
        Some(t) if t.is_melee() => weapon_name(t).to_uppercase(),
        Some(t) => format!(
            "{} {}/{}",
            weapon_name(t).to_uppercase(),
            ammo,
            t.magazine()
        ),
    }
}

/// Get player position
pub fn get_player_position(world: &World) -> Option<Vec2> {
    let players: Vec<Entity> = world.query::<Player>();
    players
        .first()
        .and_then(|&e| world.get_component::<Position>(e))
        .map(|p| p.to_vec2())
}

/// Sum of current health across all enemies (used to detect a hit landing).
pub fn total_enemy_health(world: &World) -> i32 {
    world
        .query::<Enemy>()
        .iter()
        .filter_map(|&e| world.get_component::<Health>(e))
        .map(|h| h.current.max(0))
        .sum()
}

/// Count alive enemies
pub fn count_alive_enemies(world: &World) -> usize {
    let enemies: Vec<Entity> = world.query::<Enemy>();
    enemies
        .iter()
        .filter(|&&e| {
            world
                .get_component::<Health>(e)
                .map(|h| h.is_alive())
                .unwrap_or(false)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_player() {
        let mut world = World::new();
        let player = spawn_player(&mut world, Vec2::new(100.0, 200.0));

        assert!(world.has_component::<Player>(player));
        assert!(world.has_component::<Position>(player));
        assert!(world.has_component::<Health>(player));

        let pos = world.get_component::<Position>(player).unwrap();
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 200.0);

        let health = world.get_component::<Health>(player).unwrap();
        assert_eq!(health.current, 100);
    }

    #[test]
    fn test_spawn_enemy() {
        let mut world = World::new();
        let enemy = spawn_enemy(&mut world, Vec2::new(50.0, 75.0));

        assert!(world.has_component::<Enemy>(enemy));
        assert!(world.has_component::<AI>(enemy));
        assert!(world.has_component::<Position>(enemy));

        let pos = world.get_component::<Position>(enemy).unwrap();
        assert_eq!(pos.x, 50.0);
        assert_eq!(pos.y, 75.0);
    }

    #[test]
    fn test_initialize_game() {
        // Level index 0 is the ground-level cold open: a passive crowd only.
        let mut world = World::new();
        initialize_game(&mut world, 0);
        assert_eq!(world.query::<Player>().len(), 1);
        let n = world.query::<Enemy>().len();
        assert!(n >= 2);
        assert_eq!(crate::systems::passive::count_passives(&world), n);

        // Floor 1 (index 1), the lobby: 4 bots at start (all passive; the
        // hostiles come as a wave once the desk beat fires).
        let mut world = World::new();
        initialize_game(&mut world, 1);
        assert_eq!(world.query::<Player>().len(), 1);
        assert_eq!(world.query::<Enemy>().len(), 4);
        assert_eq!(crate::systems::passive::count_passives(&world), 4);

        // Test floor 2 has 12 enemies
        let mut world2 = World::new();
        initialize_game(&mut world2, 2);
        assert_eq!(world2.query::<Player>().len(), 1);
        assert_eq!(world2.query::<Enemy>().len(), 12);
    }

    #[test]
    fn test_is_player_alive() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));

        assert!(is_player_alive(&world));

        // Kill player
        let player = world.query::<Player>()[0];
        world
            .get_component_mut::<Health>(player)
            .unwrap()
            .take_damage(100);

        assert!(!is_player_alive(&world));
    }

    #[test]
    fn test_get_player_health() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(0.0, 0.0));

        assert_eq!(get_player_health(&world), 100);

        let player = world.query::<Player>()[0];
        world
            .get_component_mut::<Health>(player)
            .unwrap()
            .take_damage(30);

        assert_eq!(get_player_health(&world), 70);
    }

    #[test]
    fn test_count_alive_enemies() {
        let mut world = World::new();
        initialize_game(&mut world, 1);

        assert_eq!(count_alive_enemies(&world), 4);

        // Kill one enemy
        let enemy = world.query::<Enemy>()[0];
        world
            .get_component_mut::<Health>(enemy)
            .unwrap()
            .take_damage(100);

        assert_eq!(count_alive_enemies(&world), 3);
    }

    #[test]
    fn test_fire_emits_player_fired_and_spends_a_round() {
        let mut world = World::new();
        let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
        assert!(!fire_player_weapon(&mut world, Vec2::new(100.0, 0.0))); // bullet spawned
        assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 11);
        assert_eq!(world.query::<Bullet>().len(), 1);
        let bullet = world.query::<Bullet>()[0];
        assert_eq!(
            world.get_component::<Bullet>(bullet).unwrap().weapon_type,
            WeaponType::Pistol
        );
        assert_eq!(
            world.drain_events(),
            vec![GameEvent::PlayerFired(WeaponType::Pistol)]
        );
        // On cooldown: nothing, and no event.
        assert!(!fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
        assert!(world.drain_events().is_empty());
    }

    #[test]
    fn test_empty_gun_dry_fires_and_never_refills() {
        let mut world = World::new();
        let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
        *world.get_component_mut::<Weapon>(player).unwrap() =
            Weapon::with_ammo(WeaponType::Pistol, 0);

        assert!(!fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
        assert!(world.query::<Bullet>().is_empty());
        assert_eq!(world.drain_events(), vec![GameEvent::DryFire]);
        // The click restarts the cooldown: holding the trigger doesn't spam.
        assert!(!fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
        assert!(world.drain_events().is_empty());
        assert_eq!(world.get_component::<Weapon>(player).unwrap().ammo, 0);
    }

    #[test]
    fn test_melee_fires_forever() {
        let mut world = World::new();
        let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
        *world.get_component_mut::<Weapon>(player).unwrap() =
            Weapon::with_ammo(WeaponType::Melee, 0);
        for _ in 0..5 {
            fire_player_weapon(&mut world, Vec2::new(100.0, 0.0));
            world
                .get_component_mut::<Weapon>(player)
                .unwrap()
                .fire_timer = 0.0;
        }
        let events = world.drain_events();
        assert_eq!(events.len(), 5);
        assert!(events
            .iter()
            .all(|e| *e == GameEvent::PlayerFired(WeaponType::Melee)));
    }

    #[test]
    fn test_unarmed_click_punches_and_knocks_down() {
        let mut world = World::new();
        let player = spawn_player(&mut world, Vec2::new(0.0, 0.0));
        world.remove_component::<Weapon>(player); // unarmed

        let enemy = spawn_enemy(&mut world, Vec2::new(30.0, 0.0));

        assert!(fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
        // Knocked down, not dead: chip damage only.
        let health = world.get_component::<Health>(enemy).unwrap();
        assert!(health.is_alive());
        assert!(health.current < health.max);
        assert!(world.has_component::<Stunned>(enemy));
        assert_eq!(
            world.drain_events(),
            vec![
                GameEvent::PlayerFired(WeaponType::Melee),
                GameEvent::EnemyHit {
                    by: WeaponType::Melee
                }
            ]
        );

        // The punch is on cooldown: an immediate second click does nothing.
        assert!(!fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
        assert!(world.drain_events().is_empty());
        // ...until the fists cooldown is ticked away.
        crate::ecs::System::run(
            &mut crate::systems::WeaponUpdateSystem,
            &mut world,
            PUNCH_COOLDOWN + 0.01,
        );
        assert!(fire_player_weapon(&mut world, Vec2::new(100.0, 0.0)));
    }

    #[test]
    fn test_weapon_hud_label() {
        assert_eq!(weapon_hud_label(None, 0), "UNARMED");
        assert_eq!(
            weapon_hud_label(Some(WeaponType::Shotgun), 3),
            "SHOTGUN 3/6"
        );
        assert_eq!(
            weapon_hud_label(Some(WeaponType::MachineGun), 30),
            "MACHINE GUN 30/30"
        );
        assert_eq!(weapon_hud_label(Some(WeaponType::Melee), 999), "MELEE");
    }

    #[test]
    fn test_level_pickups_spawn_with_full_magazine() {
        let mut world = World::new();
        let p = spawn_pickup(&mut world, Vec2::new(0.0, 0.0), WeaponType::Shotgun);
        assert_eq!(world.get_component::<WeaponPickup>(p).unwrap().ammo, 6);
        let q = spawn_pickup_with_ammo(&mut world, Vec2::new(0.0, 0.0), WeaponType::Pistol, 4);
        assert_eq!(world.get_component::<WeaponPickup>(q).unwrap().ammo, 4);
    }

    #[test]
    fn test_get_player_position() {
        let mut world = World::new();
        spawn_player(&mut world, Vec2::new(123.0, 456.0));

        let pos = get_player_position(&world).unwrap();
        assert_eq!(pos.x, 123.0);
        assert_eq!(pos.y, 456.0);
    }
}
