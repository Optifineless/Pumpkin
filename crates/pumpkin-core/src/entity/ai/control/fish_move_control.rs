use crate::entity::ai::control::move_control::MoveControl;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::mob::Mob;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::vector3::Vector3;

/// Vanilla's AbstractFish.FishMoveControl.
#[derive(Default)]
pub struct FishMoveControl {
    inner: MoveControl,
    speed: f32,
}

impl Control for FishMoveControl {}

impl MoveControlTrait for FishMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let mut velocity = entity.velocity.load();
        if entity.is_submerged_in_water() {
            velocity.y += 0.005;
        }

        let navigation_done = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_done();
        if self.has_wanted() && !navigation_done {
            let target_speed = (self.inner.speed_modifier
                * living.get_attribute_value(&Attributes::MOVEMENT_SPEED))
                as f32;
            self.speed += 0.125 * (target_speed - self.speed);
            let offset = Vector3::new(
                self.inner.wanted_x,
                self.inner.wanted_y,
                self.inner.wanted_z,
            ) - entity.pos.load();
            if offset.y != 0.0 {
                velocity.y += f64::from(self.speed) * (offset.y / offset.length()) * 0.1;
            }
            if offset.x != 0.0 || offset.z != 0.0 {
                let yaw = (offset.z.atan2(offset.x) * 180.0 / f64::from(std::f32::consts::PI))
                    as f32
                    - 90.0;
                let yaw = self.rotlerp(entity.yaw.load(), yaw, 90.0);
                entity.yaw.store(yaw);
                entity.body_yaw.store(yaw);
            }
        } else {
            self.speed = 0.0;
        }
        mob.get_mob_entity().movement_speed.store(self.speed);
        // Mob.setSpeed also sets forward input; the tracker sends delta movement.
        living
            .movement_input
            .store(Vector3::new(0.0, 0.0, f64::from(self.speed)));
        entity.velocity.store(velocity);
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.inner.set_wanted_position(x, y, z, speed_modifier);
    }

    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }
}
