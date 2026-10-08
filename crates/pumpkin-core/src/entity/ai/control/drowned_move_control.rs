use super::{Control, MoveControlTrait, move_control::MoveControl};
use crate::entity::mob::{Mob, zombie::drowned::DrownedEntity};
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::vector3::Vector3;
use std::sync::{PoisonError, atomic::Ordering};

/// Drowned.DrownedMoveControl, including the extra downward impulse while walking.
#[derive(Default)]
pub struct DrownedMoveControl {
    inner: MoveControl,
}
impl Control for DrownedMoveControl {}
impl MoveControlTrait for DrownedMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(drowned) = mob.cast_any().downcast_ref::<DrownedEntity>() else {
            return;
        };
        let m = mob.get_mob_entity();
        let entity = &m.living_entity.entity;
        if drowned.wants_to_swim() && entity.touching_water.load(Ordering::Relaxed) {
            let mut velocity = entity.velocity.load();
            if drowned.is_searching_for_land()
                || m.get_target()
                    .is_some_and(|t| t.get_entity().pos.load().y > entity.pos.load().y)
            {
                velocity.y += 0.002;
            }
            if !self.has_wanted()
                || m.navigator
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_done()
            {
                m.set_speed(0.0);
                entity.velocity.store(velocity);
                return;
            }
            let delta = Vector3::new(
                self.inner.wanted_x,
                self.inner.wanted_y,
                self.inner.wanted_z,
            ) - entity.pos.load();
            let yd = if delta.length() > 0.0 {
                delta.y / delta.length()
            } else {
                0.0
            };
            let yaw = self.rotlerp(
                entity.yaw.load(),
                delta.z.atan2(delta.x).to_degrees() as f32 - 90.0,
                90.0,
            );
            entity.yaw.store(yaw);
            entity.body_yaw.store(yaw);
            let target = (self.inner.speed_modifier
                * m.living_entity
                    .get_attribute_value(&Attributes::MOVEMENT_SPEED))
                as f32;
            let speed = m.movement_speed.load() + 0.125 * (target - m.movement_speed.load());
            m.set_speed(speed);
            entity.velocity.store(
                velocity
                    + Vector3::new(
                        f64::from(speed) * delta.x * 0.005,
                        f64::from(speed) * yd * 0.1,
                        f64::from(speed) * delta.z * 0.005,
                    ),
            );
        } else {
            if !entity.on_ground.load(Ordering::Relaxed) {
                entity
                    .velocity
                    .store(entity.velocity.load() + Vector3::new(0.0, -0.008, 0.0));
            }
            self.inner.tick(mob);
        }
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.inner.set_wanted_position(x, y, z, speed);
    }
    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }
}
