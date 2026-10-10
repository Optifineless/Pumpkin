use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_util::math::vector2::Vector2;
use rustc_hash::FxHashSet;

use super::{World, entity_persistence};
use crate::entity::{EntityBase, RemovalReason};

impl World {
    /// Removes an entity only if this call owns its first removal transition.
    pub fn remove_entity(&self, entity: &dyn EntityBase) -> bool {
        let base_entity = entity.get_entity();
        // Entity.setRemoved retains the first reason, including a non-destructive unload.
        if base_entity
            .removal_reason
            .compare_exchange(None, Some(RemovalReason::Discarded))
            .is_err()
        {
            return false;
        }
        self.clear_fishing_hook_owner(base_entity);
        base_entity.removed.store(true, Ordering::Release);
        base_entity.dispatch_removal_hook(entity, RemovalReason::Discarded);

        self.spawn_state.load().remove_entity(self, entity);
        self.entity_tracker.remove_entity(entity, self);
        self.entities.rcu(|current_entities| {
            let mut new_entities = (**current_entities).clone();
            new_entities.retain(|e| e.get_entity().entity_uuid != base_entity.entity_uuid);
            new_entities
        });
        true
    }

    pub(super) fn finish_chunk_unloads(&self, entities: &[Arc<dyn EntityBase>]) {
        // PersistentEntitySectionManager.processChunkUnload runs removal after saving.
        for entity in entities {
            entity.get_entity().removed.store(true, Ordering::Release);
            entity.on_removed(RemovalReason::UnloadedToChunk);
            self.entity_tracker.remove_entity(entity.as_ref(), self);
            self.spawn_state.load().remove_entity(self, entity.as_ref());
        }
        // Serialized trees own the unload result; release the old strong riding links.
        entity_persistence::detach_unloaded_trees(entities);
    }

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
