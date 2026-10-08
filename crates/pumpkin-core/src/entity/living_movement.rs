use super::{EntityBase, LivingEntity};
use pumpkin_data::attributes::Attributes;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};

// LivingEntity.computeModifiedFriction.
fn modified_friction(friction: f32, modifier: f32) -> f32 {
    (1.0 - (1.0 - friction) * modifier).clamp(0.0, 1.0)
}

// LivingEntity.getFrictionInfluencedSpeed. Flight controls do not change getFlyingSpeed.
fn friction_influenced_speed(
    speed: f32,
    block_friction: f32,
    on_ground: bool,
    flying_speed: f32,
) -> f32 {
    if !on_ground {
        flying_speed
    } else if f64::from(block_friction) > 0.6 {
        speed * (0.216_000_02 / (block_friction * block_friction * block_friction))
    } else {
        speed
    }
}

impl LivingEntity {
    // LivingEntity.aiStep: applyInput precedes serverAiStep, which precedes jumps/travel.
    pub(super) fn tick_mob_ai(&self, caller: &dyn EntityBase) {
        if self.health.load() <= 0.0 {
            self.jumping.store(false, SeqCst);
            self.movement_input
                .store(pumpkin_util::math::vector3::Vector3::default());
        } else if let Some(mob) = caller.get_mob()
            && !mob.get_mob_entity().is_no_ai()
        {
            // Species Brain/timer work still in mob_tick is an existing stub for customServerAiStep.
            mob.get_mob_entity().server_ai_step(mob, caller);
        }
    }

    // LivingEntity.shouldTravelInFluid tests the fluid at the feet, not the species unconditionally.
    pub(super) fn can_stand_on_current_fluid(&self, caller: &dyn EntityBase) -> bool {
        caller.get_mob().is_some_and(|mob| {
            mob.can_stand_on_fluid(
                self.entity
                    .world
                    .load()
                    .get_fluid(&self.entity.block_pos.load()),
            )
        })
    }

    pub(super) fn movement_speed(&self, caller: &dyn EntityBase) -> f64 {
        caller.get_mob().map_or_else(
            || self.get_attribute_value(&Attributes::MOVEMENT_SPEED),
            |mob| f64::from(mob.get_mob_entity().movement_speed.load()),
        )
    }

    // LivingEntity.travelInAir: block friction affects X/Z, omnidirectional only Y drag.
    pub(super) fn air_travel_factors(&self, caller: &dyn EntityBase) -> (f64, f64, f64) {
        let on_ground = self.entity.on_ground.load(Relaxed);
        let block_friction = if on_ground {
            modified_friction(
                self.entity
                    .get_block_with_y_offset(0.500_001)
                    .1
                    .slipperiness,
                self.get_attribute_value(&Attributes::FRICTION_MODIFIER) as f32,
            )
        } else {
            1.0
        };
        let drag_modifier = self.get_attribute_value(&Attributes::AIR_DRAG_MODIFIER) as f32;
        let air_drag = modified_friction(0.91, drag_modifier);
        let flying_speed = caller
            .get_player()
            .map_or(0.02, super::super::player::Player::get_off_ground_speed);
        let acceleration = friction_influenced_speed(
            self.movement_speed(caller) as f32,
            block_friction,
            on_ground,
            flying_speed as f32,
        );
        let omnidirectional = caller.is_flutterer()
            || caller
                .get_mob()
                .is_some_and(super::super::mob::Mob::omnidirectional_air_mover);
        let vertical = caller.get_y_velocity_drag().unwrap_or_else(|| {
            f64::from(modified_friction(
                if omnidirectional { 0.91 } else { 0.98 },
                drag_modifier,
            ))
        });
        (
            f64::from(acceleration),
            f64::from(block_friction * air_drag),
            vertical,
        )
    }

    // Spider.onClimbable feeds LivingEntity.handleOnClimbable before movement.
    pub(super) fn update_mob_climbable(&self, caller: &dyn EntityBase) {
        self.climbing.store(
            caller
                .get_mob()
                .is_some_and(super::super::mob::Mob::mob_on_climbable),
            Relaxed,
        );
    }

    // LivingEntity.jumpFromGround, with Rabbit's power and post-jump overrides.
    pub(super) fn jump(&self, caller: &dyn EntityBase) {
        let jump = self.get_jump_velocity(
            caller
                .get_mob()
                .map_or(1.0, super::super::mob::Mob::jump_power_scale),
        );

        if jump <= 1.0e-5 {
            // Rabbit.jumpFromGround continues even when the superclass applies no impulse.
            if let Some(mob) = caller.get_mob() {
                mob.after_jump();
            }
            return;
        }

        let mut velo = self.entity.velocity.load();

        velo.y = jump.max(velo.y);

        if self.entity.sprinting.load(Relaxed) {
            let yaw = f64::from(self.entity.yaw.load()).to_radians();

            velo.x -= yaw.sin() * 0.2;
            velo.z += yaw.cos() * 0.2;
        }

        self.entity.velocity.store(velo);

        self.entity.velocity_dirty.store(true, SeqCst);
        if let Some(mob) = caller.get_mob() {
            mob.after_jump();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::friction_influenced_speed;

    #[test]
    fn airborne_acceleration_is_independent_of_controller_speed() {
        // Bee/parrot controller speeds 0.6/0.4 must both accelerate by 0.02 off ground.
        for speed in [0.6, 0.4] {
            assert_eq!(friction_influenced_speed(speed, 1.0, false, 0.02), 0.02);
        }
        assert_eq!(friction_influenced_speed(0.4, 0.4, true, 0.02), 0.4);
        // Ice friction 0.9: 0.4 * 0.216 / 0.729 = 0.11851852.
        assert!((friction_influenced_speed(0.4, 0.9, true, 0.02) - 0.118_518_52).abs() < 1e-7);
    }
}
