use super::VillagerEntity;
use crate::{
    entity::{EntityBase, ai::brain::memory::types},
    world::home_poi::is_home,
};
use pumpkin_data::{
    block_properties::WhiteBedLikeProperties, entity::EntityPose, environment_attribute::Activity,
};
use pumpkin_util::math::boundingbox::BoundingBox;
use std::sync::{Weak, atomic::Ordering::Relaxed};

impl VillagerEntity {
    pub(super) fn validate_home(
        &self,
        home: pumpkin_util::math::position::BlockPos,
        owner: Weak<dyn EntityBase>,
    ) {
        // ValidateNearbyPoi.create only validates HOME within 16 blocks in its own dimension.
        const MAX_DISTANCE: f64 = 16.0;
        let entity = self.get_entity();
        let world = entity.world.load_full();
        // VillagerGoalPackages.getRestPackage runs HOME validation only in REST, outside PANIC.
        if world.villager_activity(&entity.block_pos.load(), entity.age.load(Relaxed) < 0)
            != Activity::Rest
            || self.sensed_panic.load(Relaxed)
        {
            return;
        }
        let now = world.get_world_age();
        // Cost trade-off: check every sensor interval (20 ticks), rather than vanilla's every tick.
        if home
            .to_centered_f64()
            .squared_distance_to_vec(&entity.pos.load())
            >= MAX_DISTANCE.powi(2)
            || now - self.last_home_validation.load(Relaxed)
                < i64::from(crate::entity::ai::brain::sensing::DEFAULT_SCAN_RATE)
        {
            return;
        }
        self.last_home_validation.store(now, Relaxed);
        let Some(state) = world.get_block_state_id_if_loaded(&home) else {
            return;
        };
        let occupied = is_home(state)
            && WhiteBedLikeProperties::from_state_id(state).occupied
            && entity.pose.load() != EntityPose::Sleeping;
        if !occupied && world.claim_home(home, owner) {
            return;
        }
        self.wake_up_if_sleeping_at(home);
        // ValidateNearbyPoi.bedIsOccupiedByVillager preserves the sleeper's ticket.
        let sleeper = occupied
            && world
                .get_entities_at_box(&BoundingBox::full_block().at_pos(home))
                .iter()
                .any(|other| {
                    other.get_entity().entity_type == entity.entity_type
                        && other.get_entity().pose.load() == EntityPose::Sleeping
                });
        if !sleeper {
            world.release_home(home, entity.entity_uuid);
        }
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .erase(types::HOME.id());
    }
}
