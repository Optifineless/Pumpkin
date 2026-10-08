use super::look_control::{LookControl, LookControlTrait};
use crate::entity::ai::control::Control;
use crate::entity::mob::Mob;
use pumpkin_util::math::wrap_degrees;

pub struct SmoothSwimmingLookControl {
    inner: LookControl,
    pub max_y_rot_from_center: i32,
}

impl SmoothSwimmingLookControl {
    #[must_use]
    pub fn new(max_y_rot_from_center: i32) -> Self {
        Self {
            inner: LookControl::default(),
            max_y_rot_from_center,
        }
    }

    pub fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;

        if self.inner.look_at_timer > 0 {
            self.inner.look_at_timer -= 1;
            if let Some(yaw) = self.inner.get_target_yaw(mob.get_mob_entity()) {
                entity.head_yaw.store(self.change_angle(
                    entity.head_yaw.load(),
                    yaw + 20.0,
                    self.inner.max_yaw_change,
                ));
            }
            if let Some(pitch) = self.inner.get_target_pitch(mob.get_mob_entity()) {
                entity.set_pitch(self.change_angle(
                    entity.pitch.load(),
                    pitch + 10.0,
                    self.inner.max_pitch_change,
                ));
            }
        } else {
            let is_idle = mob_entity
                .navigator
                .try_lock()
                .is_ok_and(|navigator| navigator.is_idle());
            if is_idle {
                entity.set_pitch(self.change_angle(entity.pitch.load(), 0.0, 5.0));
            }
            entity.head_yaw.store(self.change_angle(
                entity.head_yaw.load(),
                entity.body_yaw.load(),
                self.inner.max_yaw_change,
            ));
        }

        let head_diff_body = wrap_degrees(entity.head_yaw.load() - entity.body_yaw.load());
        let max_rot = self.max_y_rot_from_center as f32;
        if head_diff_body < -max_rot {
            let body_yaw = entity.body_yaw.load();
            entity.body_yaw.store(body_yaw - 4.0);
        } else if head_diff_body > max_rot {
            let body_yaw = entity.body_yaw.load();
            entity.body_yaw.store(body_yaw + 4.0);
        }
    }
}

impl Control for SmoothSwimmingLookControl {}

impl LookControlTrait for SmoothSwimmingLookControl {
    fn base(&mut self) -> &mut LookControl {
        &mut self.inner
    }
    fn tick(&mut self, mob: &dyn Mob) {
        Self::tick(self, mob);
    }
}
