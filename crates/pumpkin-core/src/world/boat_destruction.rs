//! Commit `VehicleEntity.destroy` only after cancellable item publication is prepared.

use std::sync::Arc;

use pumpkin_data::{entity::EntityType, item_stack::ItemStack};
use pumpkin_util::math::vector3::Vector3;

use super::{World, spawn_insertion::SpawnReservation};
use crate::{
    entity::{Entity, EntityBase, item::ItemEntity},
    plugin::api::events::entity::{entity_spawn::EntitySpawnEvent, item_spawn::ItemSpawnEvent},
};

struct PreparedBoatItem<'a> {
    entity: Option<Arc<dyn EntityBase>>,
    _reservation: Option<SpawnReservation<'a>>,
}

impl World {
    fn prepare_boat_item(
        self: &Arc<Self>,
        spawn_pos: Vector3<f64>,
        stack: Option<ItemStack>,
    ) -> Option<PreparedBoatItem<'_>> {
        let cancelled = || PreparedBoatItem {
            entity: None,
            _reservation: None,
        };
        let Some(stack) = stack.filter(|stack| !stack.is_empty()) else {
            return Some(cancelled());
        };
        let entity = Entity::new(self.clone(), spawn_pos, &EntityType::ITEM);
        let mut item_event = ItemSpawnEvent::new(
            entity.entity_id,
            spawn_pos,
            stack.item.registry_key.to_string(),
        );
        if let Some(server) = self.server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut item_event);
        }
        if item_event.cancelled {
            return Some(cancelled());
        }
        if self.level.cancel_token.is_cancelled() {
            return None;
        }
        let item: Arc<dyn EntityBase> = Arc::new(ItemEntity::new(entity, stack));
        let reservation = self.reserve_spawn_uuids(std::slice::from_ref(&item))?;
        let mut event = EntitySpawnEvent::new(
            item.get_entity().entity_id,
            EntityType::ITEM.id.to_string(),
            spawn_pos,
            self.clone(),
        );
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return Some(cancelled());
        }
        item.init_data_tracker();
        Some(PreparedBoatItem {
            entity: Some(item),
            _reservation: Some(reservation),
        })
    }

    pub(crate) fn destroy_boat_with_drop(
        self: &Arc<Self>,
        boat: &dyn EntityBase,
        stack: Option<ItemStack>,
    ) -> bool {
        let base = boat.get_entity();
        let Some(expected) = self.get_entity_by_id(base.entity_id) else {
            return false;
        };
        if !std::ptr::eq(expected.get_entity(), base) {
            return false;
        }
        let Some(prepared) = self.prepare_boat_item(base.pos.load(), stack) else {
            return false;
        };
        // A callback may close the world. Refusal retains the saved source; cancellation
        // in a current world deliberately destroys it without an item, as before.
        let Some(mutation) = self.reserve_block_entity_mutation(base.block_pos.load()) else {
            return false;
        };
        let activation = self.begin_entity_activation();
        let current = base.is_alive()
            && Arc::ptr_eq(&base.world.load_full(), self)
            && self
                .entities
                .load()
                .iter()
                .any(|entity| Arc::ptr_eq(entity, &expected));
        let claimed = current && self.try_mark_entity_removed(base);
        if claimed && let Some(item) = &prepared.entity {
            // UUID reservation excludes all other insertion paths until this leaf commits.
            self.entities.rcu(|entities| {
                let mut next = (**entities).clone();
                next.push(item.clone());
                next
            });
        }
        drop(activation);
        drop(mutation);
        if !claimed {
            return false;
        }
        self.finish_entity_removal(boat);
        if let Some(item) = &prepared.entity
            && !self.level.cancel_token.is_cancelled()
            && item.get_entity().is_alive()
            && Arc::ptr_eq(&item.get_entity().world.load_full(), self)
            && self
                .entities
                .load()
                .iter()
                .any(|entity| Arc::ptr_eq(entity, item))
        {
            self.spawn_state.load().add_entity(self, item.as_ref());
            self.entity_tracker.add_entity(item, self);
        }
        true
    }
}
