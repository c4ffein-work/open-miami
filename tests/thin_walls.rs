// Thin-wall regression tests: levels can carry very thin walls (20 px in the
// data, any size in the editor). These tests pin down that a 4-6 px wall is
// impassable for every kind of mover — walking players, pathfinding enemies,
// bullets, knocked-back bots and thrown weapons — and that a knockback-driven
// mover BOUNCES off a wall instead of sticking to (or tunneling through) it.

use open_miami::collision::circle_rect_collision;
use open_miami::components::*;
use open_miami::ecs::*;
use open_miami::math::Vec2;
use open_miami::systems::*;

const DT: f32 = 1.0 / 60.0;

/// A thin vertical wall: x 500..504, y 0..200 (4 px thick).
const WALL_X: f32 = 500.0;
const WALL_THICKNESS: f32 = 4.0;

fn add_thin_vertical_wall(world: &mut World) {
    world.add_wall(WALL_X, 0.0, WALL_THICKNESS, 200.0);
}

fn spawn_mover(world: &mut World, x: f32, y: f32, vx: f32, vy: f32, radius: f32) -> Entity {
    let e = world.spawn();
    world.add_component(e, Position::new(x, y));
    world.add_component(e, Velocity::new(vx, vy));
    world.add_component(e, Radius::new(radius));
    e
}

// ---------------------------------------------------------------------------
// a. PLAYER
// ---------------------------------------------------------------------------

#[test]
fn thin_wall_blocks_player() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    // Player-shaped mover (radius 15) walking at full speed (200 px/s)
    // straight into the thin wall.
    let player = spawn_mover(&mut world, 450.0, 100.0, 200.0, 0.0, 15.0);

    let mut movement = MovementSystem;
    for _ in 0..600 {
        movement.run(&mut world, DT);
        let pos = world.get_component::<Position>(player).unwrap();
        assert!(
            pos.x <= WALL_X - 15.0 + 0.001,
            "player crossed / entered a 4px wall: x = {}",
            pos.x
        );
        assert!(!circle_rect_collision(
            Vec2::new(pos.x, pos.y),
            15.0,
            WALL_X,
            0.0,
            WALL_THICKNESS,
            200.0
        ));
    }
    // ...and it actually reached the wall (the test wasn't vacuous).
    let pos = world.get_component::<Position>(player).unwrap();
    assert!(
        (pos.x - (WALL_X - 15.0)).abs() < 0.5,
        "player should rest against the wall"
    );
}

#[test]
fn thin_wall_blocks_player_diagonally_at_corner() {
    let mut world = World::new();
    // An inside corner made of two 4 px walls: vertical x 500..504 (y 0..200)
    // and horizontal y 200..204 (x 300..504). Player pushes diagonally into
    // the inner corner at full speed.
    world.add_wall(500.0, 0.0, 4.0, 200.0);
    world.add_wall(300.0, 200.0, 204.0, 4.0);
    let player = spawn_mover(&mut world, 460.0, 160.0, 141.4, 141.4, 15.0);

    let mut movement = MovementSystem;
    for _ in 0..600 {
        movement.run(&mut world, DT);
        let pos = world.get_component::<Position>(player).unwrap();
        assert!(
            pos.x <= 500.0 - 15.0 + 0.001 && pos.y <= 200.0 - 15.0 + 0.001,
            "player pushed through a thin corner: ({}, {})",
            pos.x,
            pos.y
        );
    }
}

// ---------------------------------------------------------------------------
// b. ENEMY (AI chasing / pathfinding)
// ---------------------------------------------------------------------------

#[test]
fn thin_wall_blocks_pathfinding_enemy() {
    let mut world = World::new();
    // A thin wall spanning the whole world vertically: no route around it.
    world.add_wall(WALL_X, 0.0, WALL_THICKNESS, 2000.0);

    // Player on the far (east) side.
    let player = world.spawn();
    world.add_component(player, Player);
    world.add_component(player, Position::new(600.0, 1000.0));

    // Enemy on the west side, already convinced of the player's position so
    // it chases (the wall would block detection by sight).
    let enemy = world.spawn();
    world.add_component(enemy, Enemy);
    let enemy_pos = Position::new(420.0, 1000.0);
    world.add_component(enemy, enemy_pos);
    let mut ai = AI::new_with_type(EnemyType::Idle, enemy_pos);
    ai.state = AIState::SurePlayerSeen;
    ai.last_known_player_position = Some(Position::new(600.0, 1000.0));
    world.add_component(enemy, ai);
    world.add_component(enemy, Velocity::zero());
    world.add_component(enemy, Speed::new(100.0));
    world.add_component(enemy, Health::new(100));
    world.add_component(enemy, Rotation::new(0.0));

    let mut ai_system = AISystem::default();
    let mut movement = MovementSystem;
    for _ in 0..300 {
        ai_system.run(&mut world, DT);
        movement.run(&mut world, DT);
        let pos = world.get_component::<Position>(enemy).unwrap();
        assert!(
            pos.x <= WALL_X - 12.0 + 0.001,
            "chasing enemy crossed / entered a 4px wall: x = {}",
            pos.x
        );
    }
}

// ---------------------------------------------------------------------------
// c. BULLETS
// ---------------------------------------------------------------------------

/// An enemy standing behind the wall: if a bullet tunnels, it gets hit.
fn spawn_enemy_behind_wall(world: &mut World) -> Entity {
    let enemy = world.spawn();
    world.add_component(enemy, Enemy);
    world.add_component(enemy, Position::new(560.0, 100.0));
    world.add_component(enemy, Radius::new(12.0));
    world.add_component(enemy, Health::new(100));
    enemy
}

#[test]
fn bullet_cannot_tunnel_thin_wall() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    let enemy = spawn_enemy_behind_wall(&mut world);

    // Bullet placed so one 60 Hz step (800 px/s -> 13.3 px) straddles the
    // wall entirely: endpoint-only collision would fly clean through.
    let bullet = world.spawn();
    world.add_component(bullet, Bullet::new(WeaponType::Pistol, 30));
    world.add_component(bullet, Position::new(493.5, 100.0));
    world.add_component(bullet, Velocity::new(800.0, 0.0));
    world.add_component(bullet, Radius::new(2.0));

    let mut bullets = BulletSystem;
    for _ in 0..120 {
        bullets.run(&mut world, DT);
        for b in world.query::<Bullet>() {
            let pos = world.get_component::<Position>(b).unwrap();
            assert!(
                pos.x < WALL_X,
                "bullet tunneled through a 4px wall: x = {}",
                pos.x
            );
        }
    }
    assert!(world.query::<Bullet>().is_empty(), "bullet should be gone");
    assert_eq!(
        world.get_component::<Health>(enemy).unwrap().current,
        100,
        "enemy behind the wall must not be hit"
    );
    assert!(world.drain_events().is_empty());
}

#[test]
fn bullet_cannot_tunnel_thin_wall_full_pipeline() {
    // Same, but through the real per-frame system order (MovementSystem also
    // advances bullets — they carry Position + Velocity — and its push-out
    // resolution must not shove one across a thin wall either).
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    let enemy = spawn_enemy_behind_wall(&mut world);

    let bullet = world.spawn();
    world.add_component(bullet, Bullet::new(WeaponType::Pistol, 30));
    world.add_component(bullet, Position::new(450.0, 100.0));
    world.add_component(bullet, Velocity::new(800.0, 0.0));
    world.add_component(bullet, Radius::new(2.0));

    let mut movement = MovementSystem;
    let mut bullets = BulletSystem;
    for _ in 0..120 {
        movement.run(&mut world, DT);
        bullets.run(&mut world, DT);
        for b in world.query::<Bullet>() {
            let pos = world.get_component::<Position>(b).unwrap();
            assert!(
                pos.x < WALL_X,
                "bullet crossed a 4px wall in the full pipeline: x = {}",
                pos.x
            );
        }
    }
    assert_eq!(
        world.get_component::<Health>(enemy).unwrap().current,
        100,
        "enemy behind the wall must not be hit"
    );
}

// ---------------------------------------------------------------------------
// d. KNOCKED-BACK ENEMY
// ---------------------------------------------------------------------------

#[test]
fn knocked_back_enemy_cannot_tunnel_thin_wall() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    // Enemy near the wall, shoved hard toward it (stacked hits reach well
    // over 1000 px/s -> 25 px in the first frame: enough to put the centre
    // past a 4px wall's midline, where the old nearest-edge resolution
    // ejected it out the FAR side).
    let enemy = spawn_mover(&mut world, 486.0, 100.0, 0.0, 0.0, 12.0);
    world.add_component(enemy, Knockback::new(1500.0, 0.0));

    let mut movement = MovementSystem;
    for _ in 0..120 {
        movement.run(&mut world, DT);
        let pos = world.get_component::<Position>(enemy).unwrap();
        assert!(
            pos.x < WALL_X,
            "knocked-back enemy tunneled through a 4px wall: x = {}",
            pos.x
        );
    }
    let pos = world.get_component::<Position>(enemy).unwrap();
    assert!(pos.x <= WALL_X - 12.0 + 0.001);
}

// ---------------------------------------------------------------------------
// e. THROWN WEAPON
// ---------------------------------------------------------------------------

#[test]
fn thrown_weapon_stops_on_thin_wall() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);

    let player = world.spawn();
    world.add_component(player, Player);
    world.add_component(player, Position::new(400.0, 100.0));
    world.add_component(player, Radius::new(15.0));
    world.add_component(player, Weapon::new(WeaponType::Pistol));
    assert!(ThrownWeaponSystem::throw_from_player(
        &mut world,
        Vec2::new(1.0, 0.0)
    ));

    let mut system = ThrownWeaponSystem;
    for _ in 0..200 {
        system.run(&mut world, DT);
        for t in world.query::<ThrownWeapon>() {
            let pos = world.get_component::<Position>(t).unwrap();
            assert!(pos.x < WALL_X, "thrown weapon crossed the wall");
        }
        if world.query::<ThrownWeapon>().is_empty() {
            break;
        }
    }
    // It dropped as a pickup on the near side of the wall.
    let pickups = world.query::<WeaponPickup>();
    assert_eq!(pickups.len(), 1);
    let pos = world.get_component::<Position>(pickups[0]).unwrap();
    assert!(
        pos.x < WALL_X,
        "thrown weapon should land before the wall, landed at x = {}",
        pos.x
    );
}

// ---------------------------------------------------------------------------
// TASK 2: knockback BOUNCE off walls
// ---------------------------------------------------------------------------

#[test]
fn knockback_bounces_off_east_wall() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    // Enemy resting against the wall on its EAST side, shot from the west:
    // the shove slams it into the wall at 900 px/s.
    let enemy = spawn_mover(&mut world, WALL_X - 12.0, 100.0, 0.0, 0.0, 12.0);
    world.add_component(enemy, Knockback::new(900.0, 0.0));

    let mut movement = MovementSystem;
    movement.run(&mut world, DT);

    // The impulse reflected off the wall: it now points WEST (away from it).
    let kb = world.get_component::<Knockback>(enemy).unwrap();
    assert!(
        kb.x < 0.0,
        "knockback should rebound off the east wall, kb.x = {}",
        kb.x
    );
    let pos = world.get_component::<Position>(enemy).unwrap();
    assert!(pos.x <= WALL_X - 12.0 + 0.001, "never inside the wall");
    assert!(!circle_rect_collision(
        Vec2::new(pos.x, pos.y),
        12.0,
        WALL_X,
        0.0,
        WALL_THICKNESS,
        200.0
    ));

    // A few more frames: it visibly rebounds away from the wall.
    for _ in 0..10 {
        movement.run(&mut world, DT);
    }
    let pos = world.get_component::<Position>(enemy).unwrap();
    assert!(
        pos.x < WALL_X - 12.0 - 5.0,
        "enemy should visibly rebound off the wall, x = {}",
        pos.x
    );
}

#[test]
fn knockback_bounces_off_north_wall() {
    let mut world = World::new();
    // A thin horizontal wall NORTH of the enemy: y 96..100.
    world.add_wall(0.0, 96.0, 200.0, 4.0);
    // Enemy just under it, shot from the south (shoved north, -y).
    let enemy = spawn_mover(&mut world, 100.0, 112.0, 0.0, 0.0, 12.0);
    world.add_component(enemy, Knockback::new(0.0, -900.0));

    let mut movement = MovementSystem;
    movement.run(&mut world, DT);

    let kb = world.get_component::<Knockback>(enemy).unwrap();
    assert!(
        kb.y > 0.0,
        "knockback should rebound off the north wall, kb.y = {}",
        kb.y
    );
    let pos = world.get_component::<Position>(enemy).unwrap();
    assert!(pos.y >= 112.0 - 0.001, "never inside the wall");

    for _ in 0..10 {
        movement.run(&mut world, DT);
    }
    let pos = world.get_component::<Position>(enemy).unwrap();
    assert!(
        pos.y > 112.0 + 5.0,
        "enemy should visibly rebound off the wall, y = {}",
        pos.y
    );
}

#[test]
fn slow_walk_into_wall_does_not_bounce() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    // Ordinary walking (plain Velocity, no Knockback) into the wall: the
    // mover stops flush against it and STAYS there — no rebound.
    let player = spawn_mover(&mut world, 470.0, 100.0, 200.0, 0.0, 15.0);

    let mut movement = MovementSystem;
    for _ in 0..60 {
        movement.run(&mut world, DT);
    }
    // Pressed against the wall...
    for _ in 0..30 {
        movement.run(&mut world, DT);
        let pos = world.get_component::<Position>(player).unwrap();
        assert!(
            (pos.x - (WALL_X - 15.0)).abs() < 0.001,
            "walking should pin against the wall, not bounce: x = {}",
            pos.x
        );
    }
    // ...and the walking velocity itself is untouched.
    let vel = world.get_component::<Velocity>(player).unwrap();
    assert_eq!(vel.x, 200.0);
}

#[test]
fn weak_knockback_does_not_bounce() {
    let mut world = World::new();
    add_thin_vertical_wall(&mut world);
    // A shove below the bounce threshold (250 px/s into the wall) just stops
    // against it, like walking — the decayed tail of an impulse must not
    // jitter off walls.
    let enemy = spawn_mover(&mut world, WALL_X - 12.0, 100.0, 0.0, 0.0, 12.0);
    world.add_component(enemy, Knockback::new(200.0, 0.0));

    let mut movement = MovementSystem;
    for _ in 0..60 {
        movement.run(&mut world, DT);
        if let Some(kb) = world.get_component::<Knockback>(enemy) {
            assert!(kb.x >= 0.0, "weak knockback must not bounce");
        }
        let pos = world.get_component::<Position>(enemy).unwrap();
        assert!((pos.x - (WALL_X - 12.0)).abs() < 0.001);
    }
}
