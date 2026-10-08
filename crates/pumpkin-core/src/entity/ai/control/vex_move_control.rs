use super::{
    Control, MoveControlTrait,
    move_control::{MoveControl, Operation},
};
use crate::entity::mob::Mob;
use pumpkin_util::math::vector3::Vector3;

/// Vex.VexMoveControl accelerates toward a point independently of navigation.
#[derive(Default)]
pub struct VexMoveControl {
    inner: MoveControl,
}
impl Control for VexMoveControl {}
impl MoveControlTrait for VexMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        if !self.has_wanted() {
            return;
        }
        let entity = mob.get_entity();
        let delta = Vector3::new(
            self.inner.wanted_x,
            self.inner.wanted_y,
            self.inner.wanted_z,
        ) - entity.pos.load();
        let length = delta.length();
        if length < entity.bounding_box.load().get_average_side_length() {
            self.inner.operation = Operation::Wait;
            entity.velocity.store(entity.velocity.load() * 0.5);
        } else {
            let velocity =
                entity.velocity.load() + delta * (self.inner.speed_modifier * 0.05 / length);
            entity.velocity.store(velocity);
            let facing = mob
                .get_mob_entity()
                .get_target()
                .map_or(velocity, |t| t.get_entity().pos.load() - entity.pos.load());
            let yaw = -facing.x.atan2(facing.z).to_degrees() as f32;
            entity.yaw.store(yaw);
            entity.body_yaw.store(yaw);
        }
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.inner.set_wanted_position(x, y, z, speed);
    }
    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }
}
