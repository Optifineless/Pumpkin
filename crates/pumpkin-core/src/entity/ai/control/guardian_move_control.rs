use super::{Control, MoveControlTrait, move_control::MoveControl};
use crate::entity::mob::Mob;
use pumpkin_data::{attributes::Attributes, tracked_data};
use pumpkin_util::math::vector3::Vector3;
use std::sync::PoisonError;

/// Guardian.GuardianMoveControl, shared by elder guardians.
#[derive(Default)]
pub struct GuardianMoveControl {
    inner: MoveControl,
    moving: bool,
}
impl Control for GuardianMoveControl {}
impl MoveControlTrait for GuardianMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let m = mob.get_mob_entity();
        let entity = &m.living_entity.entity;
        let moving = self.has_wanted()
            && !m
                .navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_done();
        self.moving = moving;
        entity.set_synced_data(tracked_data::guardian::DATA_ID_MOVING, moving);
        if !moving {
            m.set_speed(0.0);
            return;
        }
        let delta = Vector3::new(
            self.inner.wanted_x,
            self.inner.wanted_y,
            self.inner.wanted_z,
        ) - entity.pos.load();
        let length = delta.length();
        if length < f64::EPSILON {
            m.set_speed(0.0);
            return;
        }
        let direction = delta * (1.0 / length);
        let yaw = self.rotlerp(
            entity.yaw.load(),
            delta.z.atan2(delta.x).to_degrees() as f32 - 90.0,
            90.0,
        );
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);
        let target = (self.inner.speed_modifier
            * m.living_entity
                .get_attribute_value(&Attributes::MOVEMENT_SPEED)) as f32;
        let speed = m.movement_speed.load() + 0.125 * (target - m.movement_speed.load());
        m.set_speed(speed);
        let phase =
            f64::from(entity.age.load(std::sync::atomic::Ordering::Relaxed) + entity.entity_id);
        let push = (phase * 0.5).sin() * 0.05;
        let (sin, cos) = f64::from(yaw.to_radians()).sin_cos();
        let y_push = (phase * 0.75).sin() * 0.05;
        entity.velocity.store(
            entity.velocity.load()
                + Vector3::new(
                    push * cos,
                    y_push * (sin + cos) * 0.25 + f64::from(speed) * direction.y * 0.1,
                    push * sin,
                ),
        );
        let next = Vector3::new(
            entity.pos.load().x + direction.x * 2.0,
            entity.get_eye_y() + direction.y / length,
            entity.pos.load().z + direction.z * 2.0,
        );
        let mut look = m
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let base = look.base();
        let previous = if base.look_at_timer > 0 {
            base.position
        } else {
            next
        };
        let wanted = previous + (next - previous) * 0.125;
        look.look_at_with_range(wanted.x, wanted.y, wanted.z, 10.0, 40.0);
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.inner.set_wanted_position(x, y, z, speed);
    }
    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }

    fn is_moving(&self) -> bool {
        self.moving
    }
}
