use pumpkin_data::{
    Advancement,
    entity::{EntityStatus, EntityType},
    sound::Sound,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use rand::RngExt;
use std::sync::atomic::Ordering::Relaxed;

use crate::block::{
    BlockBehaviour, OnEntityCollisionArgs, OnLandedUponArgs, PathComputationType,
    UpdateEntityMovementAfterFallOnArgs, stop_vertical_movement_after_fall,
};

#[pumpkin_block("minecraft:honey_block")]
pub struct HoneyBlock;

// HoneyBlock's constants and gravity/drag conversion from vanilla 26.3.
const SLIDE_STARTS_WHEN_VERTICAL_SPEED_IS_AT_LEAST: f64 = 0.13;
const MIN_FALL_SPEED_TO_BE_CONSIDERED_SLIDING: f64 = 0.08;
const THROTTLE_SLIDE_SPEED_TO: f64 = 0.05;
const SLIDE_ADVANCEMENT_CHECK_INTERVAL: i64 = 20;

impl HoneyBlock {
    fn get_old_delta_y(delta_y: f64) -> f64 {
        delta_y / f64::from(0.98f32) + 0.08
    }

    fn get_new_delta_y(delta_y: f64) -> f64 {
        (delta_y - 0.08) * f64::from(0.98f32)
    }

    // HoneyBlock.isSlidingDown. The inside volume is a full cube, wider than the inset collider.
    fn is_sliding_down(pos: &BlockPos, caller: &dyn crate::entity::EntityBase) -> bool {
        Self::is_sliding_down_with_movement(pos, caller, Self::movement(caller))
    }

    fn is_sliding_down_with_movement(
        pos: &BlockPos,
        caller: &dyn crate::entity::EntityBase,
        movement: pumpkin_util::math::vector3::Vector3<f64>,
    ) -> bool {
        let entity = caller.get_entity();
        let location = entity.pos.load();
        if entity.on_ground.load(Relaxed)
            || location.y > f64::from(pos.0.y) + 0.9375 - 1.0E-7
            || Self::get_old_delta_y(movement.y) >= -MIN_FALL_SPEED_TO_BE_CONSIDERED_SLIDING
        {
            return false;
        }
        let dx = (f64::from(pos.0.x) + 0.5 - location.x).abs();
        let dz = (f64::from(pos.0.z) + 0.5 - location.z).abs();
        let overlap_distance = 0.4375 + f64::from(entity.width() / 2.0);
        dx + 1.0E-7 > overlap_distance || dz + 1.0E-7 > overlap_distance
    }

    fn movement(
        caller: &dyn crate::entity::EntityBase,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        // Client-reported displacement precedes travel's gravity/drag, unlike server deltaMovement.
        caller.get_player().map_or_else(
            || caller.get_entity().velocity.load(),
            |player| {
                let mut movement = player.get_known_movement();
                movement.y = Self::get_new_delta_y(movement.y);
                movement
            },
        )
    }

    /// Resets a player's fall after accepted motion, before another packet can report a landing.
    pub(crate) fn reset_player_fall_distance(caller: &dyn crate::entity::EntityBase, delta_y: f64) {
        let Some(player) = caller.get_player() else {
            return;
        };
        let entity = caller.get_entity();
        let mut movement = Self::movement(caller);
        movement.y = Self::get_new_delta_y(delta_y);
        let world = entity.world.load();
        let bounds = entity.bounding_box.load().contract_all(1.0E-7);
        // HoneyBlock.entityInside after Entity.checkFallDamage; only honey is dispatched here.
        for pos in BlockPos::iterate(bounds.min_block_pos(), bounds.max_block_pos()) {
            if world.get_block(&pos) == &pumpkin_data::Block::HONEY_BLOCK
                && Self::is_sliding_down_with_movement(&pos, caller, movement)
            {
                player.living_entity.fall_distance.store(0.0);
                return;
            }
        }
    }

    // HoneyBlock.doSlideMovement; player's own client predicts this motion.
    fn do_slide_movement(caller: &dyn crate::entity::EntityBase) {
        let entity = caller.get_entity();
        let mut movement = Self::movement(caller);
        let old_y = Self::get_old_delta_y(movement.y);
        if old_y < -SLIDE_STARTS_WHEN_VERTICAL_SPEED_IS_AT_LEAST {
            let reduction = -THROTTLE_SLIDE_SPEED_TO / old_y;
            movement.x *= reduction;
            movement.z *= reduction;
        }
        movement.y = Self::get_new_delta_y(-THROTTLE_SLIDE_SPEED_TO);
        entity.velocity.store(movement);
        if let Some(living) = caller.get_living_entity() {
            living.fall_distance.store(0.0);
        }
        if let Some(falling) = caller
            .cast_any()
            .downcast_ref::<crate::entity::falling::FallingEntity>()
        {
            falling.reset_fall_distance();
        }
    }

    fn maybe_do_slide_effects(args: &OnEntityCollisionArgs<'_>) {
        let entity = args.entity.get_entity();
        if args.entity.get_living_entity().is_some()
            || entity.entity_type == &EntityType::TNT
            || args
                .entity
                .cast_any()
                .is::<crate::entity::vehicle::minecart::MinecartEntity>()
            || args
                .entity
                .cast_any()
                .is::<crate::entity::vehicle::boat::BoatEntity>()
        {
            let mut random = rand::rng();
            if random.random_range(0..5) == 0 {
                entity.play_sound(Sound::BlockHoneyBlockSlide);
            }
            if random.random_range(0..5) == 0 {
                args.world
                    .send_entity_status(entity, EntityStatus::HoneySlide, None);
            }
        }
    }
}

impl BlockBehaviour for HoneyBlock {
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        if Self::is_sliding_down(args.position, args.entity) {
            // HoneyBlock.maybeDoSlideAchievement uses the world's game time.
            if let Some(player) = args.entity.get_player()
                && args
                    .world
                    .level_time
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .world_age
                    % SLIDE_ADVANCEMENT_CHECK_INTERVAL
                    == 0
            {
                player.trigger_advancement_criterion(
                    Advancement::ADVENTURE_HONEY_BLOCK_SLIDE,
                    "honey_block_slide",
                );
            }
            Self::do_slide_movement(args.entity);
            Self::maybe_do_slide_effects(&args);
        }
    }

    fn on_landed_upon(&self, args: OnLandedUponArgs<'_>) {
        if let Some(living) = args.entity.get_living_entity() {
            living.handle_fall_damage(args.entity, args.fall_distance, 0.2);
        }
    }

    fn update_entity_movement_after_fall_on(&self, args: UpdateEntityMovementAfterFallOnArgs<'_>) {
        stop_vertical_movement_after_fall(args.entity);
    }

    fn is_pathfindable(
        &self,
        _state: &pumpkin_data::BlockState,
        _computation_type: PathComputationType,
    ) -> bool {
        false
    }
}
