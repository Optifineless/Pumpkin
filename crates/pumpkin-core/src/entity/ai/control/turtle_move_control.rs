use super::{Control, MoveControlTrait, move_control::MoveControl};
use crate::entity::{mob::Mob, passive::turtle::TurtleEntity};
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::vector3::Vector3;
use std::sync::{PoisonError, atomic::Ordering};

/// Turtle.TurtleMoveControl: buoyancy and speed depend on age, home and ground contact.
#[derive(Default)]
pub struct TurtleMoveControl {
    inner: MoveControl,
}
impl Control for TurtleMoveControl {}
impl MoveControlTrait for TurtleMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(turtle) = mob.cast_any().downcast_ref::<TurtleEntity>() else {
            return;
        };
        let m = mob.get_mob_entity();
        let entity = &m.living_entity.entity;
        let mut speed = m.movement_speed.load();
        let mut velocity = entity.velocity.load();
        if entity.touching_water.load(Ordering::Relaxed) {
            velocity.y += 0.005;
            if !turtle.close_to_home(16.0) {
                speed = (speed / 2.0).max(0.08);
            }
            if mob
                .as_ageable()
                .is_some_and(crate::entity::ageable::AgeableMob::is_baby)
            {
                speed = (speed / 3.0).max(0.06);
            }
        } else if entity.on_ground.load(Ordering::Relaxed) {
            speed = (speed / 2.0).max(0.06);
        }
        if self.has_wanted()
            && !m
                .navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_done()
        {
            let delta = Vector3::new(
                self.inner.wanted_x,
                self.inner.wanted_y,
                self.inner.wanted_z,
            ) - entity.pos.load();
            let length = delta.length();
            if length < f64::from(1.0e-5f32) {
                speed = 0.0;
            } else {
                let yaw = self.rotlerp(
                    entity.yaw.load(),
                    delta.z.atan2(delta.x).to_degrees() as f32 - 90.0,
                    90.0,
                );
                entity.yaw.store(yaw);
                entity.body_yaw.store(yaw);
                let target_speed = (self.inner.speed_modifier
                    * m.living_entity
                        .get_attribute_value(&Attributes::MOVEMENT_SPEED))
                    as f32;
                speed += 0.125 * (target_speed - speed);
                velocity.y += f64::from(speed) * delta.y / length * 0.1;
            }
        } else {
            speed = 0.0;
        }
        m.set_speed(speed);
        entity.velocity.store(velocity);
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.inner.set_wanted_position(x, y, z, speed);
    }
    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }
}
