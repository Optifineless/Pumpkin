use std::sync::Arc;

use pumpkin_util::math::vector2::Vector2;
use rustc_hash::FxHashSet;

use super::{World, entity_persistence};
use crate::entity::{EntityBase, RemovalReason};

impl World {
    pub(super) fn claim_chunk_unloads(
        &self,
        chunks: &FxHashSet<Vector2<i32>>,
    ) -> Vec<Arc<dyn EntityBase>> {
        // PersistentEntitySectionManager.unloadEntity -> Entity.setRemoved retains the
        // first reason. Claim before saving because Pumpkin damage callbacks can race unloading.
        let claimed: Vec<_> = self
            .entities
            .load()
            .iter()
            .filter(|entity| chunks.contains(&entity_persistence::root_chunk(entity)))
            .filter(|entity| {
                entity
                    .get_entity()
                    .removal_reason
                    .compare_exchange(None, Some(RemovalReason::UnloadedToChunk))
                    .is_ok()
            })
            .cloned()
            .collect();
        let ids: FxHashSet<_> = claimed
            .iter()
            .map(|entity| entity.get_entity().entity_uuid)
            .collect();
        // Keep ownership outside RCU retries, which must not repeat removal transitions.
        self.entities.rcu(|current| {
            current
                .iter()
                .filter(|entity| !ids.contains(&entity.get_entity().entity_uuid))
                .cloned()
                .collect::<Vec<_>>()
        });
        claimed
    }
}
