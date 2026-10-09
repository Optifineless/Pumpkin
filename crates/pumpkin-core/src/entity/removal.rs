use std::sync::{Arc, Weak};

use pumpkin_data::tag::{self, Taggable};

use super::{Entity, EntityBase, RemovalReason, vehicle::boat::BoatEntity};

impl Entity {
    /// Retains the concrete removal callback without extending the entity's lifetime.
    pub(crate) fn register_removal_hook(&self, concrete: &Arc<dyn EntityBase>) {
        // Entity.remove is virtual in vanilla, including AbstractChestBoat.remove.
        let _ = self.removal_hook.set(Arc::downgrade(concrete));
    }

    pub(crate) fn dispatch_removal_hook(&self, fallback: &dyn EntityBase, reason: RemovalReason) {
        if let Some(concrete) = self.removal_hook.get().and_then(Weak::upgrade) {
            concrete.on_removed(reason);
        } else {
            fallback.on_removed(reason);
        }
    }

    pub(super) fn defer_boat_portal(&self) -> bool {
        // AbstractBoat.tick -> Entity.handlePortal: remove when astra-movement's
        // transfer_nonplayer_tree provides real dimension transfer for boats and riders.
        if self.entity_type.has_tag(&tag::EntityType::MINECRAFT_BOAT)
            || BoatEntity::has_chest_inventory(self.entity_type)
        {
            self.portal_manager
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            return true;
        }
        false
    }
}
