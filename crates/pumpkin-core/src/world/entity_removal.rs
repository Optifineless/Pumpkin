use std::sync::atomic::Ordering;

use super::World;
use crate::entity::{Entity, EntityBase};

impl World {
    /// Returns whether this call owns the first removal and its subsequent effects.
    pub(crate) fn try_remove_entity(&self, entity: &dyn EntityBase) -> bool {
        if !self.try_mark_entity_removed(entity.get_entity()) {
            return false;
        }
        self.finish_entity_removal(entity);
        true
    }

    /// Claims first removal without callbacks; the winner must finish cleanup after unlocking.
    pub(crate) fn try_mark_entity_removed(&self, entity: &Entity) -> bool {
        self.begin_entity_removal(entity)
    }

    pub(crate) fn finish_entity_removal(&self, entity: &dyn EntityBase) {
        let base_entity = entity.get_entity();
        self.clear_fishing_hook_owner(base_entity);
        self.retire_removed_container(entity);
        base_entity.removed.store(true, Ordering::Release);
        // Entity.setRemoved ejects passengers after marking the mount unavailable.
        base_entity.detach_riding_links_after_removal(crate::entity::RemovalReason::Discarded);

        self.spawn_state.load().remove_entity(self, entity);
        self.entity_tracker.remove_entity(entity, self);
        self.entities.rcu(|current_entities| {
            let mut new_entities = (**current_entities).clone();
            new_entities.retain(|e| e.get_entity().entity_uuid != base_entity.entity_uuid);
            new_entities
        });
        self.retire_game_event_source(base_entity);
    }
}
