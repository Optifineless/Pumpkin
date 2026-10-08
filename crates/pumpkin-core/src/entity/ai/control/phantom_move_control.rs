use super::{Control, MoveControlTrait};
use crate::entity::mob::{Mob, phantom::PhantomEntity};
use pumpkin_util::math::{approach, vector3::Vector3, wrap_degrees};
use std::sync::atomic::Ordering;

pub struct PhantomMoveControl {
    speed: f32,
}
impl Default for PhantomMoveControl {
    fn default() -> Self {
        Self { speed: 0.1 }
    }
}
impl Control for PhantomMoveControl {}
impl MoveControlTrait for PhantomMoveControl {
    // Phantom.PhantomMoveControl.tick.
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(phantom) = mob.cast_any().downcast_ref::<PhantomEntity>() else {
            return;
        };
        let entity = mob.get_entity();
        if entity.horizontal_collision.load(Ordering::Relaxed) {
            entity.yaw.store(entity.yaw.load() + 180.0);
            self.speed = 0.1;
        }
        let mut delta = phantom.move_target_point.load() - entity.pos.load();
        let horizontal = delta.x.hypot(delta.z);
        if horizontal.abs() <= f64::from(1.0e-5f32) {
            return;
        }
        let scale = 1.0 - (delta.y * f64::from(0.7f32)).abs() / horizontal;
        delta.x *= scale;
        delta.z *= scale;
        let horizontal = delta.x.hypot(delta.z);
        let length = delta.length();
        let previous_yaw = entity.yaw.load();
        let yaw = self.change_angle(
            wrap_degrees(previous_yaw + 90.0),
            wrap_degrees(delta.z.atan2(delta.x).to_degrees() as f32),
            4.0,
        ) - 90.0;
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);
        self.speed = if wrap_degrees(previous_yaw - yaw).abs() < 3.0 {
            approach(self.speed, 1.8, 0.005 * (1.8 / self.speed))
        } else {
            approach(self.speed, 0.2, 0.025)
        };
        let pitch = -(-delta.y).atan2(horizontal).to_degrees() as f32;
        entity.pitch.store(pitch);
        let angle = (yaw + 90.0).to_radians();
        let target = Vector3::new(
            f64::from(self.speed * angle.cos()) * (delta.x / length).abs(),
            f64::from(self.speed * pitch.to_radians().sin()) * (delta.y / length).abs(),
            f64::from(self.speed * angle.sin()) * (delta.z / length).abs(),
        );
        let movement = entity.velocity.load();
        entity.velocity.store(movement + (target - movement) * 0.2);
    }
}
