use super::Player;
use crate::{entity::EntityBase, world::guard_effects};
use pumpkin_data::{data_component_impl::EquipmentSlot, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use std::sync::Arc;

struct HeldDropEquipment {
    selected: u8,
    equipment: Vec<(EquipmentSlot, ItemStack)>,
}

impl Player {
    pub(super) fn drop_held_item_inner(&self, drop_stack: bool) {
        let _effects = guard_effects::GuardEffects::new();
        if self.client.closed() || self.get_entity().is_removed() || !self.menu_player_is_current()
        {
            return;
        }
        let source_slot = usize::from(self.inventory.get_selected_slot());
        let item_stack = self.inventory.get_stack(source_slot);
        if item_stack.is_empty() {
            return;
        }
        let mut slots: Vec<_> = self.inventory.equipment_slots.values().cloned().collect();
        slots.push(EquipmentSlot::MAIN_HAND);
        let before = HeldDropEquipment {
            selected: source_slot as u8,
            equipment: self.collect_equipment_changes(slots),
        };
        let drop_amount = if drop_stack { item_stack.item_count } else { 1 };
        let dropped_stack = item_stack.copy_with_count(drop_amount);
        if let Some(server) = self.world().server.upgrade()
            && let Some(player) = self.world().get_player_by_uuid(self.gameprofile.id)
            && std::ptr::eq(self, player.as_ref())
        {
            let mut event =
                crate::plugin::api::events::player::player_drop_item::PlayerDropItemEvent::new(
                    player,
                    dropped_stack.item.registry_key.to_string(),
                    dropped_stack.item_count,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                self.reconcile_held_drop_equipment(before);
                return;
            }
        }
        if self.client.closed() || self.get_entity().is_removed() || !self.menu_player_is_current()
        {
            return;
        }
        let dropped_stack = pumpkin_inventory::inventory::slot_transaction::withdraw(
            self.inventory.as_ref(),
            source_slot,
            &item_stack,
            drop_amount,
        );
        self.reconcile_held_drop_equipment(before);
        if dropped_stack.is_empty() {
            return;
        }
        let updated_stack = self.inventory.get_stack(source_slot);
        self.drop_item(dropped_stack);
        if self.client.closed() || self.get_entity().is_removed() || !self.menu_player_is_current()
        {
            return;
        }

        let inv: Arc<dyn Inventory> = self.inventory.clone();
        let screen_binding = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut screen_handler = screen_binding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot_index) = screen_handler.get_slot_index(&inv, source_slot) {
            screen_handler.set_received_stack(slot_index, updated_stack);
            screen_handler.send_content_updates();
        }
    }

    fn reconcile_held_drop_equipment(&self, before: HeldDropEquipment) {
        let Some(player) = self.world().get_player_by_uuid(self.gameprofile.id) else {
            return;
        };
        if !std::ptr::eq(self, player.as_ref()) {
            return;
        }
        // LivingEntity.detectEquipmentUpdates observes live equipment after every callback outcome.
        let effect = move || {
            if player.client.closed()
                || player.get_entity().is_removed()
                || !player.menu_player_is_current()
            {
                return;
            }
            let slots = before
                .equipment
                .iter()
                .map(|(slot, _)| slot.clone())
                .collect();
            let live = player.collect_equipment_changes(slots);
            let changed: Vec<_> = before
                .equipment
                .iter()
                .zip(&live)
                .filter(|((slot, old), (_, new))| {
                    !old.are_equal(new)
                        || (*slot == EquipmentSlot::MAIN_HAND
                            && before.selected != player.inventory.get_selected_slot())
                })
                .map(|((slot, _), _)| slot.clone())
                .collect();
            player.queue_equipment_updates(&changed);
        };
        if let Err(effect) = guard_effects::defer(effect) {
            effect();
        }
    }
}
