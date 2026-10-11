use super::World;
use crate::entity::{EntityBase, RemovalReason};

impl World {
    pub(super) fn remove_entity_with_reason(&self, entity: &dyn EntityBase, reason: RemovalReason) {
        let base_entity = entity.get_entity();
        if !base_entity.set_removed(reason) {
            return;
        }
        self.clear_fishing_hook_owner(base_entity);
        self.spawn_state.load().remove_entity(self, entity);
        self.entity_tracker.remove_entity(entity, self);
        self.entities.rcu(|current_entities| {
            let mut new_entities = (**current_entities).clone();
            new_entities.retain(|e| !std::ptr::eq(e.get_entity(), base_entity));
            new_entities
        });
    }
}
