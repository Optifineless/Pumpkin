use std::sync::Arc;

use pumpkin_inventory::Inventory;
use pumpkin_util::math::vector3::Vector3;

use super::World;
use crate::{entity::item::ItemEntity, plugin::api::events::entity::item_spawn::ItemSpawnEvent};

impl World {
    /// Scatters a container's contents at its entity position with zero pickup delay.
    pub(crate) fn scatter_container_inventory(
        self: &Arc<Self>,
        position: Vector3<f64>,
        inventory: &Arc<dyn Inventory>,
    ) {
        // Containers.dropContents(Level, Entity, Container) retains the entity's position.
        for slot in 0..inventory.size() {
            self.scatter_stack(
                position.x,
                position.y,
                position.z,
                inventory.remove_stack(slot),
            );
        }
    }

    /// Fires the cancellable item-spawn event before publishing an item entity.
    pub(crate) fn spawn_item_with_event(self: &Arc<Self>, item: ItemEntity) {
        let entity = item.get_entity();
        let name = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .item
            .registry_key
            .to_owned();
        let mut event = ItemSpawnEvent::new(entity.entity_id, entity.pos.load(), name);
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if !event.cancelled {
            self.spawn_entity(Arc::new(item));
        }
    }
}
