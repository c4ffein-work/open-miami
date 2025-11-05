use crate::ecs::{World, System};
use crate::components::Weapon;

/// System that updates weapon timers
pub struct WeaponUpdateSystem;

impl System for WeaponUpdateSystem {
    fn run(&mut self, world: &mut World, dt: f32) {
        let entities: Vec<_> = world.query::<Weapon>();

        for entity in entities {
            if let Some(weapon) = world.get_component_mut::<Weapon>(entity) {
                weapon.update(dt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::WeaponType;

    #[test]
    fn test_weapon_update_system_decreases_timer() {
        let mut world = World::new();
        let entity = world.spawn();

        let mut weapon = Weapon::new(WeaponType::Pistol);
        weapon.fire(); // Start cooldown
        world.add_component(entity, weapon);

        let mut system = WeaponUpdateSystem;
        system.run(&mut world, 0.3);

        let weapon = world.get_component::<Weapon>(entity).unwrap();
        assert!((weapon.fire_timer - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_weapon_update_system_multiple_weapons() {
        let mut world = World::new();

        let e1 = world.spawn();
        let mut w1 = Weapon::new(WeaponType::Pistol);
        w1.fire();
        world.add_component(e1, w1);

        let e2 = world.spawn();
        let mut w2 = Weapon::new(WeaponType::Shotgun);
        w2.fire();
        world.add_component(e2, w2);

        let mut system = WeaponUpdateSystem;
        system.run(&mut world, 0.5);

        let weapon1 = world.get_component::<Weapon>(e1).unwrap();
        let weapon2 = world.get_component::<Weapon>(e2).unwrap();

        assert_eq!(weapon1.fire_timer, 0.0);
        assert_eq!(weapon2.fire_timer, 0.5);
    }
}
