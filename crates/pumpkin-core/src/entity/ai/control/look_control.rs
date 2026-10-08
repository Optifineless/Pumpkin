use crate::entity::EntityBase;
use crate::entity::ai::control::Control;
use crate::entity::mob::{Mob, MobEntity};
use pumpkin_util::math::clamp_angle;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

// Please keep the atomic values out of here!!!
#[derive(Default)]
pub struct LookControl {
    pub(super) max_yaw_change: f32,
    pub(super) max_pitch_change: f32,
    pub(super) look_at_timer: i32,
    pub(super) position: Vector3<f64>,
}

impl Control for LookControl {}

impl LookControl {
    pub fn look_at_position(&mut self, mob: &dyn Mob, position: Vector3<f64>) {
        self.look_at(mob, position.x, position.y, position.z);
    }

    pub fn look_at_entity(&mut self, mob: &dyn Mob, entity: &Arc<dyn EntityBase>) {
        let entity = entity.get_entity();
        let pos = entity.get_eye_pos();
        self.look_at(mob, pos.x, pos.y, pos.z);
    }

    pub fn look_at_entity_with_range(
        &mut self,
        entity: &Arc<dyn EntityBase>,
        max_yaw_change: f32,
        max_pitch_change: f32,
    ) {
        let entity = entity.get_entity();
        let pos = entity.pos.load();
        self.look_at_with_range(
            pos.x,
            entity.get_eye_y(),
            pos.z,
            max_yaw_change,
            max_pitch_change,
        );
    }

    pub fn look_at(&mut self, mob: &dyn Mob, x: f64, y: f64, z: f64) {
        self.look_at_with_range(
            x,
            y,
            z,
            mob.get_max_look_yaw_change(),
            mob.get_max_look_pitch_change(),
        );
    }

    pub const fn look_at_with_range(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        max_yaw_change: f32,
        max_pitch_change: f32,
    ) {
        self.position = Vector3::new(x, y, z);
        self.max_yaw_change = max_yaw_change;
        self.max_pitch_change = max_pitch_change;
        self.look_at_timer = 2;
    }

    pub fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        if mob.reset_look_pitch() {
            entity.set_pitch(0.0);
        }

        if self.look_at_timer > 0 {
            self.look_at_timer -= 1;
            if let Some(yaw) = self.get_target_yaw(mob.get_mob_entity()) {
                entity.head_yaw.store(self.change_angle(
                    entity.head_yaw.load(),
                    yaw,
                    self.max_yaw_change,
                ));
            }
            if let Some(pitch) = self.get_target_pitch(mob.get_mob_entity()) {
                entity.set_pitch(self.change_angle(
                    entity.pitch.load(),
                    pitch,
                    self.max_pitch_change,
                ));
            }
        } else {
            entity.head_yaw.store(self.change_angle(
                entity.head_yaw.load(),
                entity.body_yaw.load(),
                10.0,
            ));
        }

        Self::clamp_head_yaw(mob);
    }

    fn clamp_head_yaw(mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        if mob.clamp_look_yaw_when_idle()
            || mob_entity
                .navigator
                .try_lock()
                .is_ok_and(|navigator| navigator.is_in_progress())
        {
            let entity = &mob_entity.living_entity.entity;
            let max_head_rotation = mob.get_max_head_rotation();
            entity.head_yaw.store(clamp_angle(
                entity.head_yaw.load(),
                entity.body_yaw.load(),
                max_head_rotation,
            ));
        }
    }

    pub(super) fn get_target_pitch(&self, mob: &MobEntity) -> Option<f32> {
        let position = self.position;
        let mob_position = mob.living_entity.entity.pos.load();
        let d = position.x - mob_position.x;
        let e = position.y - mob.living_entity.entity.get_eye_y();
        let f = position.z - mob_position.z;
        let g = d.hypot(f);
        if e.abs() <= 1.0E-5 && g.abs() <= 1.0E-5 {
            None
        } else {
            Some(-(e.atan2(g) as f32).to_degrees())
        }
    }

    pub(super) fn get_target_yaw(&self, mob: &MobEntity) -> Option<f32> {
        let position = self.position;
        let mob_position = mob.living_entity.entity.pos.load();
        let d = position.x - mob_position.x;
        let e = position.z - mob_position.z;
        if e.abs() <= 1.0E-5 && d.abs() <= 1.0E-5 {
            None
        } else {
            Some((e.atan2(d) as f32).to_degrees() - 90.0)
        }
    }
}

/// Common look requests, dispatched to the species' vanilla `LookControl` subclass.
pub trait LookControlTrait: Control {
    fn base(&mut self) -> &mut LookControl;
    fn tick(&mut self, mob: &dyn Mob);
    fn look_at(&mut self, mob: &dyn Mob, x: f64, y: f64, z: f64) {
        self.base().look_at(mob, x, y, z);
    }
    fn look_at_position(&mut self, mob: &dyn Mob, position: Vector3<f64>) {
        self.base().look_at_position(mob, position);
    }
    fn look_at_entity(&mut self, mob: &dyn Mob, entity: &Arc<dyn EntityBase>) {
        self.base().look_at_entity(mob, entity);
    }
    fn look_at_entity_with_range(&mut self, entity: &Arc<dyn EntityBase>, yaw: f32, pitch: f32) {
        self.base().look_at_entity_with_range(entity, yaw, pitch);
    }
    fn look_at_with_range(&mut self, x: f64, y: f64, z: f64, yaw: f32, pitch: f32) {
        self.base().look_at_with_range(x, y, z, yaw, pitch);
    }
}

impl LookControlTrait for LookControl {
    fn base(&mut self) -> &mut LookControl {
        self
    }
    fn tick(&mut self, mob: &dyn Mob) {
        Self::tick(self, mob);
    }
}
