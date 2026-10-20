use super::{
    Control,
    look_control::{LookControl, LookControlTrait},
};
use crate::entity::{mob::Mob, passive::happy_ghast::HappyGhastEntity};
use pumpkin_util::math::wrap_degrees_90;

/// HappyGhast.HappyGhastLookControl.
#[derive(Default)]
pub struct HappyGhastLookControl(pub LookControl);
impl Control for HappyGhastLookControl {}
impl LookControlTrait for HappyGhastLookControl {
    fn base(&mut self) -> &mut LookControl {
        &mut self.0
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let ghast = mob.cast_any().downcast_ref::<HappyGhastEntity>();
        if ghast.is_some_and(HappyGhastEntity::is_on_still_timeout) {
            let yaw = entity.yaw.load();
            entity.yaw.store(yaw - wrap_degrees_90(yaw));
            entity.head_yaw.store(entity.yaw.load());
        } else if self.0.look_at_timer > 0 {
            self.0.look_at_timer -= 1;
            let delta = self.0.position - entity.pos.load();
            let yaw = -(delta.x.atan2(delta.z) as f32) * (180.0 / std::f32::consts::PI);
            entity.yaw.store(yaw);
            entity.body_yaw.store(yaw);
            entity.head_yaw.store(yaw);
        } else {
            let delta = if let Some(target) = ghast.and_then(HappyGhastEntity::facing_target) {
                let delta = target - entity.pos.load();
                if !(0.0..4096.0).contains(&delta.length_squared()) {
                    return;
                }
                delta
            } else {
                entity.velocity.load()
            };
            let yaw = -(delta.x.atan2(delta.z) as f32) * (180.0 / std::f32::consts::PI);
            entity.yaw.store(yaw);
            entity.body_yaw.store(yaw);
        }
    }
}
