use pumpkin_inventory::inventory::slot_transaction;
use pumpkin_util::math::position::BlockPos;

use crate::{
    block::entities::BlockEntity,
    entity::{EntityBase, vehicle::minecart::MinecartEntity},
};

use super::World;

// BlockEntity.setRemoved / LevelChunk.clearAllBlockEntities retire old container references.
pub(super) fn retire_block_entity(entity: &std::sync::Arc<dyn BlockEntity>) {
    if let Some(inventory) = entity.clone().get_inventory() {
        slot_transaction::retire(inventory.as_ref());
    } else if let Some(campfire) = entity
        .as_any()
        .downcast_ref::<crate::block::entities::campfire::CampfireBlockEntity>(
    ) {
        // CampfireBlockEntity is Clearable food storage, not a vanilla hopper container.
        slot_transaction::retire(campfire);
    } else if let Some(pot) = entity
        .as_any()
        .downcast_ref::<crate::block::entities::decorated_pot::DecoratedPotBlockEntity>(
    ) {
        slot_transaction::retire(pot);
    }
}

pub(super) fn retire_entity_container(entity: &std::sync::Arc<dyn EntityBase>) {
    if let Some(cart) = entity.as_ref().cast_any().downcast_ref::<MinecartEntity>()
        && let Some(inventory) = cart.container()
    {
        slot_transaction::retire(inventory.as_ref());
    }
}

impl World {
    pub(super) fn retire_replaced_block_entity(
        previous: &std::sync::Arc<dyn BlockEntity>,
        replacement: &std::sync::Arc<dyn BlockEntity>,
    ) {
        if std::sync::Arc::ptr_eq(previous, replacement) {
            return;
        }
        let Some(previous_inventory) = std::sync::Arc::clone(previous).get_inventory() else {
            retire_block_entity(previous);
            return;
        };
        // LevelChunk.setBlockEntity retires the previous instance before replacing it.
        // A wrapper around the same backing container preserves its existing ownership.
        if let Some(next_inventory) = std::sync::Arc::clone(replacement).get_inventory()
            && std::sync::Arc::ptr_eq(
                &previous_inventory.slot_transaction(0),
                &next_inventory.slot_transaction(0),
            )
        {
            return;
        }
        slot_transaction::retire(previous_inventory.as_ref());
    }

    pub(super) fn retire_removed_container(&self, entity: &dyn EntityBase) {
        let entity_type = entity.get_entity().entity_type;
        if entity_type != &pumpkin_data::entity::EntityType::CHEST_MINECART
            && entity_type != &pumpkin_data::entity::EntityType::HOPPER_MINECART
        {
            return;
        }
        // AbstractMinecartContainer.destroy unpacks deferred contents before container removal.
        // Entity.remove supplies the base entity, so find the retained concrete instance.
        let entities = self.entities.load();
        let container = entities
            .iter()
            .find(|current| current.get_entity().entity_uuid == entity.get_entity().entity_uuid)
            .and_then(|current| current.cast_any().downcast_ref::<MinecartEntity>())
            .and_then(MinecartEntity::container)
            .cloned();
        drop(entities);
        if let Some(container) = container {
            let params = crate::world::loot::build_container_loot_context(
                &entity.get_entity().world.load(),
                entity.get_entity().pos.load(),
                None,
            );
            container.unpack_loot(&params);
            slot_transaction::retire(container.as_ref());
        }
    }

    pub(super) fn retire_container(&self, position: &BlockPos) {
        if let Some(entity) = self.get_block_entity(position) {
            #[cfg(test)]
            crate::inventory_transaction_followup_tests::pause_retirement();
            retire_block_entity(&entity);
        }
    }
}
