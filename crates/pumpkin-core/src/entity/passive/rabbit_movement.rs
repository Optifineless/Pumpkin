//! Rabbit's hopping lifecycle, from Rabbit.customServerAiStep and aiStep.
use super::rabbit::{FLEE_SPEED_MOD, RabbitEntity, RabbitVariant, STROLL_SPEED_MOD};
use crate::entity::{EntityBase, ageable::AgeableMob};
use pumpkin_data::{
    entity::EntityStatus,
    sound::{Sound, SoundCategory},
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{
    PoisonError,
    atomic::{AtomicBool, AtomicI32, Ordering::Relaxed},
};

// Rabbit constants controlling the pause and animation after each hop.
const JUMP_DELAY_TICKS: i32 = 10;
const PANIC_JUMP_DELAY_TICKS: i32 = 3;
const JUMP_DURATION_IN_TICKS: i32 = 15;

#[derive(Default)]
pub struct RabbitJumpState {
    pub delay: AtomicI32,
    pub can_jump: AtomicBool,
    was_on_ground: AtomicBool,
    duration: AtomicI32,
    ticks: AtomicI32,
}

// Rabbit.setLandingDelay and getJumpPower: Java hardcodes these thresholds.
fn landing_delay(speed: f64) -> i32 {
    if speed < FLEE_SPEED_MOD {
        JUMP_DELAY_TICKS
    } else {
        PANIC_JUMP_DELAY_TICKS
    }
}
fn jump_scale(speed: f64, needs_height: bool) -> f64 {
    let base: f32 = if needs_height {
        0.5
    } else if speed <= STROLL_SPEED_MOD {
        0.2
    } else {
        0.3
    };
    f64::from(base / 0.42f32)
}

impl RabbitEntity {
    pub(super) fn start_jumping(&self) {
        self.mob_entity.living_entity.jumping.store(true, Relaxed);
        self.jump_state
            .duration
            .store(JUMP_DURATION_IN_TICKS, Relaxed);
        self.jump_state.ticks.store(0, Relaxed);
        let entity = self.get_entity();
        if !entity.is_silent() {
            let pitch = ((rand::random::<f32>() - rand::random::<f32>()) * 0.2 + 1.0) * 0.8;
            let category = if self.get_variant() == RabbitVariant::Evil {
                SoundCategory::Hostile
            } else {
                SoundCategory::Neutral
            };
            entity.world.load().play_sound_fine(
                Sound::EntityRabbitJump,
                category,
                &entity.pos.load(),
                1.0,
                pitch,
            );
        }
    }

    pub(super) fn tick_hopping(&self) {
        let state = &self.jump_state;
        if state.delay.load(Relaxed) > 0 {
            state.delay.fetch_sub(1, Relaxed);
        }
        let carrot_ticks = self.more_carrot_ticks.load(Relaxed);
        if carrot_ticks > 0 {
            self.more_carrot_ticks
                .store((carrot_ticks - rand::random_range(0..3)).max(0), Relaxed);
        }
        let entity = self.get_entity();
        let on_ground = entity.on_ground.load(Relaxed);
        if on_ground {
            let (wanted, speed, has_wanted) = {
                let control = self
                    .mob_entity
                    .move_control
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let (wanted, speed) = control.wanted_position();
                (wanted, speed, control.has_wanted())
            };
            if !state.was_on_ground.load(Relaxed) {
                self.mob_entity.living_entity.jumping.store(false, Relaxed);
                state.delay.store(landing_delay(speed), Relaxed);
                state.can_jump.store(false, Relaxed);
            }
            if self.get_variant() == RabbitVariant::Evil
                && state.delay.load(Relaxed) == 0
                && let Some(target) = self.mob_entity.get_target()
                && (target.get_entity().pos.load() - entity.pos.load()).length_squared() < 16.0
            {
                let target = target.get_entity().pos.load();
                self.face_point(target);
                self.mob_entity
                    .move_control
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set_wanted_position(target.x, target.y, target.z, speed);
                self.start_jumping();
            }
            let requested = self
                .mob_entity
                .jump_control
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .has_request();
            if !requested && has_wanted && state.delay.load(Relaxed) == 0 {
                let next = self
                    .mob_entity
                    .navigator
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .next_move_target()
                    .map_or(wanted, |(position, _)| position);
                self.face_point(next);
                self.start_jumping();
            } else if requested && !state.can_jump.load(Relaxed) {
                state.can_jump.store(true, Relaxed);
            }
        }
        state.was_on_ground.store(on_ground, Relaxed);
    }

    fn face_point(&self, point: Vector3<f64>) {
        let offset = point - self.get_entity().pos.load();
        self.get_entity()
            .yaw
            .store(offset.z.atan2(offset.x).to_degrees() as f32 - 90.0);
    }

    pub(super) fn rabbit_jump_power_scale(&self) -> f64 {
        let entity = self.get_entity();
        let (wanted, speed) = self
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wanted_position();
        let path_above = self
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_move_target()
            .is_some_and(|(next, _)| next.y > entity.pos.load().y + 0.5);
        let higher = path_above
            || entity.horizontal_collision.load(Relaxed)
            || self.mob_entity.living_entity.jumping.load(Relaxed)
                && wanted.y > entity.pos.load().y + 0.5;
        jump_scale(speed, higher)
    }

    pub(super) fn tick_jump_animation(&self) {
        let state = &self.jump_state;
        if state.ticks.load(Relaxed) != state.duration.load(Relaxed) {
            state.ticks.fetch_add(1, Relaxed);
        } else if state.duration.load(Relaxed) != 0 {
            state.duration.store(0, Relaxed);
            state.ticks.store(0, Relaxed);
            self.mob_entity.living_entity.jumping.store(false, Relaxed);
        }
    }

    pub(super) fn rabbit_after_jump(&self) {
        let entity = self.get_entity();
        let (_, speed) = self
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wanted_position();
        let velocity = entity.velocity.load();
        if speed > 0.0 && velocity.x * velocity.x + velocity.z * velocity.z < 0.01 {
            entity.update_velocity_from_input(
                Vector3::new(0.0, if self.is_baby() { 0.5 } else { 1.5 }, 1.0),
                f64::from(0.1f32),
            );
        }
        // Rabbit.jumpFromGround broadcasts event 1 after the actual impulse, not startJumping.
        entity
            .world
            .load()
            .send_entity_status(entity, EntityStatus::Jump, None);
    }
}

#[cfg(test)]
mod tests {
    use super::{jump_scale, landing_delay};
    #[test]
    fn rabbit_jump_scaling_and_landing_delay() {
        assert_eq!(landing_delay(2.1999), 10);
        assert_eq!(landing_delay(2.2), 3);
        // Expected multipliers; this does not establish hop displacement through travel.
        assert!((jump_scale(0.6, false) - 0.476_190_5).abs() < 1e-7);
        assert!((jump_scale(1.0, false) - 0.714_285_73).abs() < 1e-7);
        assert!((jump_scale(0.6, true) - 1.190_476_2).abs() < 1e-7);
    }
}
