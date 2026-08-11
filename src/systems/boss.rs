use crate::components::{
    AIState, Boss, Health, Player, Position, Rotation, Speed, Stunned, Velocity, AI,
};
use crate::ecs::{Entity, System, World};

/// Boss tuning.
pub const BOSS_MAX_HEALTH: i32 = 360;
pub const BOSS_RADIUS: f32 = 42.0;
/// Movement speed while the smiley mask is intact (slow, lumbering).
pub const BOSS_MASK_SPEED: f32 = 55.0;
/// Movement speed once the mask cracks off (fast, frantic).
pub const BOSS_ENRAGED_SPEED: f32 = 175.0;
/// Contact range at which the boss can hit the player.
pub const BOSS_ATTACK_RANGE: f32 = 62.0;

/// Fraction of max health at which the smiley mask cracks off and the shoggoth
/// enrages.
const MASK_CRACK_FRACTION: f32 = 0.5;

/// System that drives the shoggoth boss: relentless pursuit (ignoring the normal
/// vision-cone rules), a mask that cracks off at half health to enrage it, and
/// immunity to knockdown.
pub struct BossSystem;

impl BossSystem {
    fn player_position(world: &World) -> Option<Position> {
        world
            .query::<Player>()
            .first()
            .and_then(|&e| world.get_component::<Position>(e))
            .copied()
    }
}

impl System for BossSystem {
    fn run(&mut self, world: &mut World, _dt: f32) {
        let player_pos = match Self::player_position(world) {
            Some(p) => p,
            None => return,
        };

        let bosses: Vec<Entity> = world.query::<Boss>();
        for boss in bosses {
            // The boss shrugs off knockdown — it can never be stunned.
            if world.has_component::<Stunned>(boss) {
                world.remove_component::<Stunned>(boss);
            }

            let (pos, health) = match (
                world.get_component::<Position>(boss),
                world.get_component::<Health>(boss),
            ) {
                (Some(p), Some(h)) => (*p, *h),
                _ => continue,
            };

            if health.is_dead() {
                if let Some(v) = world.get_component_mut::<Velocity>(boss) {
                    v.x = 0.0;
                    v.y = 0.0;
                }
                continue;
            }

            // Crack the mask off at half health -> enrage (once).
            let crack_at = (health.max as f32 * MASK_CRACK_FRACTION) as i32;
            let mut enraged = world
                .get_component::<Boss>(boss)
                .map(|b| b.enraged)
                .unwrap_or(false);
            if !enraged && health.current <= crack_at {
                enraged = true;
                if let Some(b) = world.get_component_mut::<Boss>(boss) {
                    b.enraged = true;
                }
                if let Some(s) = world.get_component_mut::<Speed>(boss) {
                    s.value = BOSS_ENRAGED_SPEED;
                }
            }

            let speed = world
                .get_component::<Speed>(boss)
                .map(|s| s.value)
                .unwrap_or(if enraged {
                    BOSS_ENRAGED_SPEED
                } else {
                    BOSS_MASK_SPEED
                });

            // Hunt the player directly (movement resolves walls afterward).
            let dx = player_pos.x - pos.x;
            let dy = player_pos.y - pos.y;
            let len = (dx * dx + dy * dy).sqrt();
            if let Some(v) = world.get_component_mut::<Velocity>(boss) {
                if len > 0.0 {
                    v.x = dx / len * speed;
                    v.y = dy / len * speed;
                } else {
                    v.x = 0.0;
                    v.y = 0.0;
                }
            }
            if let Some(r) = world.get_component_mut::<Rotation>(boss) {
                r.angle = dy.atan2(dx);
            }

            // Keep the boss permanently aware so combat lets it attack, and it
            // never loses the player to the vision-cone state machine.
            if let Some(ai) = world.get_component_mut::<AI>(boss) {
                ai.state = AIState::SurePlayerSeen;
                ai.last_known_player_position = Some(player_pos);
                ai.attack_range = BOSS_ATTACK_RANGE;
                ai.detection_range = 5000.0;
            }
        }
    }
}

/// Whether any boss in the world has cracked its mask (enraged). Used for
/// audio/visual cues.
pub fn any_boss_enraged(world: &World) -> bool {
    world.query::<Boss>().iter().any(|&e| {
        world
            .get_component::<Boss>(e)
            .map(|b| b.enraged)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Enemy, Radius};
    use crate::math::Vec2;

    fn spawn_test_boss(world: &mut World, pos: Vec2) -> Entity {
        let e = world.spawn();
        world.add_component(e, Enemy);
        world.add_component(e, Boss::new());
        world.add_component(e, Position::from_vec2(pos));
        world.add_component(e, Velocity::zero());
        world.add_component(e, Speed::new(BOSS_MASK_SPEED));
        world.add_component(e, Health::new(BOSS_MAX_HEALTH));
        world.add_component(e, Radius::new(BOSS_RADIUS));
        world.add_component(e, Rotation::new(0.0));
        world.add_component(e, AI::new());
        e
    }

    fn spawn_test_player(world: &mut World, pos: Vec2) -> Entity {
        let p = world.spawn();
        world.add_component(p, Player);
        world.add_component(p, Position::from_vec2(pos));
        p
    }

    #[test]
    fn test_boss_hunts_player() {
        let mut world = World::new();
        spawn_test_player(&mut world, Vec2::new(0.0, 0.0));
        let boss = spawn_test_boss(&mut world, Vec2::new(100.0, 0.0));

        BossSystem.run(&mut world, 0.016);

        // Velocity points toward the player (negative x).
        let v = world.get_component::<Velocity>(boss).unwrap();
        assert!(v.x < 0.0);
        // And it is set to attack.
        let ai = world.get_component::<AI>(boss).unwrap();
        assert_eq!(ai.state, AIState::SurePlayerSeen);
    }

    #[test]
    fn test_mask_cracks_and_enrages_at_half_health() {
        let mut world = World::new();
        spawn_test_player(&mut world, Vec2::new(0.0, 0.0));
        let boss = spawn_test_boss(&mut world, Vec2::new(100.0, 0.0));

        // Above half: still masked.
        BossSystem.run(&mut world, 0.016);
        assert!(!world.get_component::<Boss>(boss).unwrap().enraged);
        assert_eq!(
            world.get_component::<Speed>(boss).unwrap().value,
            BOSS_MASK_SPEED
        );

        // Drop to half health -> mask cracks, speed jumps.
        world.get_component_mut::<Health>(boss).unwrap().current = BOSS_MAX_HEALTH / 2;
        BossSystem.run(&mut world, 0.016);
        assert!(world.get_component::<Boss>(boss).unwrap().enraged);
        assert_eq!(
            world.get_component::<Speed>(boss).unwrap().value,
            BOSS_ENRAGED_SPEED
        );
        assert!(any_boss_enraged(&world));
    }

    #[test]
    fn test_boss_is_immune_to_stun() {
        let mut world = World::new();
        spawn_test_player(&mut world, Vec2::new(0.0, 0.0));
        let boss = spawn_test_boss(&mut world, Vec2::new(100.0, 0.0));
        world.add_component(boss, Stunned::new(3.0));

        BossSystem.run(&mut world, 0.016);

        assert!(!world.has_component::<Stunned>(boss));
    }

    #[test]
    fn test_dead_boss_stops() {
        let mut world = World::new();
        spawn_test_player(&mut world, Vec2::new(0.0, 0.0));
        let boss = spawn_test_boss(&mut world, Vec2::new(100.0, 0.0));
        world.get_component_mut::<Health>(boss).unwrap().current = 0;

        BossSystem.run(&mut world, 0.016);

        let v = world.get_component::<Velocity>(boss).unwrap();
        assert_eq!(v.x, 0.0);
        assert_eq!(v.y, 0.0);
    }
}
