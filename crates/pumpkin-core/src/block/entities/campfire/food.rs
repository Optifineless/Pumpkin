use super::CampfireBlockEntity;
use crate::{
    entity::{EntityBase, player::Player},
    plugin::api::events::block::campfire_start::CampfireStartEvent,
    world::World,
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{Clearable, Inventory, inventory::slot_transaction};
use std::sync::{Arc, atomic::Ordering};

impl CampfireBlockEntity {
    // CampfireBlockEntity.placeFood admits cold storage too; the tick decides when to cook.
    pub(crate) fn place_food(
        &self,
        input: &mut ItemStack,
        player: &Player,
        world: &Arc<World>,
        slot: usize,
        cooking_time: i32,
    ) -> bool {
        if !slot_transaction::snapshot_slot(self, slot).is_empty() {
            return false;
        }
        let mut event = CampfireStartEvent::new(
            self.position,
            world.clone(),
            input.copy_with_count(1),
            slot as u8,
            cooking_time,
        );
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled
            || player.get_entity().is_removed()
            || !Arc::ptr_eq(&player.world(), world)
            || !world
                .players
                .load()
                .iter()
                .any(|current| std::ptr::eq(current.as_ref(), player))
            || !world
                .get_block_entity(&self.position)
                .is_some_and(|current| {
                    current
                        .as_any()
                        .downcast_ref::<Self>()
                        .is_some_and(|current| std::ptr::eq(current, self))
                })
        {
            return false;
        }
        // The adapter commits the accepted timer with the existing source/destination transfer.
        crate::item::item_utils::place_held_item(
            input,
            player,
            &AdmittedFood {
                campfire: self,
                cooking_time: event.cooking_time,
            },
            slot,
        )
    }
}

struct AdmittedFood<'a> {
    campfire: &'a CampfireBlockEntity,
    cooking_time: i32,
}

impl Inventory for AdmittedFood<'_> {
    fn slot_transaction(&self, slot: usize) -> Arc<slot_transaction::Transaction> {
        self.campfire.slot_transaction(slot)
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        self.campfire.update_slot_storage(slot, update);
    }

    fn on_slot_replaced(&self, slot: usize, previous: &ItemStack, current: &ItemStack) {
        self.campfire.on_slot_replaced(slot, previous, current);
        *self.campfire.cooking_total_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.cooking_time;
        // Preserve even zero/negative overrides without firing the legacy fallback again.
        self.campfire.cooking_started[slot].store(true, Ordering::Relaxed);
    }

    fn size(&self) -> usize {
        self.campfire.size()
    }

    fn is_empty(&self) -> bool {
        self.campfire.is_empty()
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        self.campfire.get_stack(slot)
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        self.campfire.remove_stack(slot)
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        self.campfire.remove_stack_specific(slot, amount)
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        self.campfire.set_stack(slot, stack);
    }

    fn mark_dirty(&self) {
        self.campfire.mark_dirty();
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self.campfire
    }
}

impl Clearable for AdmittedFood<'_> {
    fn clear(&self) {
        self.campfire.clear();
    }
}
