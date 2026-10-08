//! Vex charge and random flight goals steer the move control directly.
use crate::entity::{
    EntityBase,
    ai::goal::{Controls, Goal, to_goal_ticks},
    mob::{Mob, vex::VexEntity},
};
use pumpkin_util::math::position::BlockPos;
use std::sync::PoisonError;

pub struct VexChargeAttackGoal;
impl VexChargeAttackGoal {
    fn target_is_alive(target: &dyn EntityBase) -> bool {
        target.get_entity().is_alive()
            && target
                .get_living_entity()
                .is_some_and(|living| living.health.load() > 0.0)
    }

    fn move_to_eyes(mob: &dyn Mob, target: &dyn EntityBase) {
        let entity = target.get_entity();
        let eye = entity.pos.load()
            + pumpkin_util::math::vector3::Vector3::new(0.0, entity.get_eye_height(), 0.0);
        mob.get_mob_entity()
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_wanted_position(eye.x, eye.y, eye.z, 1.0);
    }
}

// Vex.VexChargeAttackGoal: canUse, canContinueToUse, start, stop and tick.
impl Goal for VexChargeAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.get_mob_entity().get_target().is_some_and(|target| {
            Self::target_is_alive(target.as_ref())
                && !mob
                    .get_mob_entity()
                    .move_control
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .has_wanted()
                && rand::random_range(0..to_goal_ticks(7)) == 0
                && mob
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&target.get_entity().pos.load())
                    > 4.0
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<VexEntity>()
            .is_some_and(VexEntity::is_charging)
            && mob
                .get_mob_entity()
                .move_control
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .has_wanted()
            && mob
                .get_mob_entity()
                .get_target()
                .is_some_and(|target| Self::target_is_alive(target.as_ref()))
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(target) = mob.get_mob_entity().get_target() {
            Self::move_to_eyes(mob, target.as_ref());
        }
        if let Some(vex) = mob.cast_any().downcast_ref::<VexEntity>() {
            vex.set_charging(true);
        }
        let entity = mob.get_entity();
        if !entity.is_silent() {
            entity.world.load().play_sound_fine(
                pumpkin_data::sound::Sound::EntityVexCharge,
                pumpkin_data::sound::SoundCategory::Hostile,
                &entity.pos.load(),
                1.0,
                1.0,
            );
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(vex) = mob.cast_any().downcast_ref::<VexEntity>() {
            vex.set_charging(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(target) = mob.get_mob_entity().get_target() {
            if mob
                .get_entity()
                .bounding_box
                .load()
                .intersects(&target.get_entity().bounding_box.load())
            {
                mob.get_mob_entity()
                    .try_attack(mob.get_entity(), target.as_ref());
                self.stop(mob);
            } else if mob
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&target.get_entity().pos.load())
                < 9.0
            {
                Self::move_to_eyes(mob, target.as_ref());
            }
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

pub struct VexRandomMoveGoal;
impl Goal for VexRandomMoveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !mob.get_mob_entity()
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .has_wanted()
            && rand::random_range(0..to_goal_ticks(7)) == 0
    }
    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }
    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let origin = mob.get_home().unwrap_or_else(|| entity.block_pos.load());
        let world = entity.world.load();
        for _ in 0..3 {
            let test = BlockPos::new(
                origin.0.x + rand::random_range(0..15) - 7,
                origin.0.y + rand::random_range(0..11) - 5,
                origin.0.z + rand::random_range(0..15) - 7,
            );
            if world.get_block_state(&test).is_air() {
                let target =
                    test.to_f64() + pumpkin_util::math::vector3::Vector3::new(0.5, 0.5, 0.5);
                mob.get_mob_entity()
                    .move_control
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set_wanted_position(target.x, target.y, target.z, 0.25);
                if mob.get_mob_entity().get_target().is_none() {
                    mob.get_mob_entity()
                        .look_control
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .look_at_with_range(target.x, target.y, target.z, 180.0, 20.0);
                }
                break;
            }
        }
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
