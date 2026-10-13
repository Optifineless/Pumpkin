#[allow(clippy::wildcard_imports)]
use super::*;

impl BedrockClient {
    pub fn handle_mob_equipment(
        &self,
        server: &Arc<Server>,
        player: &Arc<Player>,
        packet: &SMobEquipment,
    ) {
        if !self.select_hotbar_slot(server, player, packet.selected_slot) {
            // LivingEntity.detectEquipmentUpdates publishes the live selection after callbacks.
            self.send_live_hotbar_selection(player);
            let held = player.inventory().held_item();
            player
                .living_entity
                .send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, held)]);
        }
    }

    /// Reconciles selection through the held-item event and equipment publication.
    /// Call without guards; false means cancellation or a callback changed selection.
    pub(super) fn select_hotbar_slot(
        &self,
        server: &Arc<Server>,
        player: &Arc<Player>,
        slot: u8,
    ) -> bool {
        player.update_last_action_time();
        // ServerGamePacketListenerImpl.handleSetCarriedItem validates the hotbar index.
        if !PlayerInventory::is_valid_hotbar_index(usize::from(slot)) {
            return false;
        }
        let previous_slot = player.inventory.get_selected_slot();
        let mut event = PlayerItemHeldEvent::new(player.clone(), previous_slot, slot);
        server.plugin_manager.fire_blocking(server, &mut event);
        // A held-item hook may itself select a slot. Preserve that newer selection.
        if event.cancelled || player.inventory().get_selected_slot() != previous_slot {
            self.send_live_hotbar_selection(player);
            return false;
        }

        let inv = player.inventory();
        inv.set_selected_slot(slot);
        let stack = inv.held_item();
        player.enqueue_equipment_change(&EquipmentSlot::MAIN_HAND, &stack);
        player.inventory().get_selected_slot() == slot
    }
}
