#[allow(clippy::wildcard_imports)]
use super::*;

impl BedrockClient {
    pub fn handle_mob_equipment(
        &self,
        server: &Arc<Server>,
        player: &Arc<Player>,
        packet: &SMobEquipment,
    ) {
        self.select_hotbar_slot(server, player, packet.selected_slot);
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
            self.try_enqueue_client_packet(&CPlayerHotbar {
                selected_slot: VarUInt(u32::from(player.inventory().get_selected_slot())),
                container_id: 0,
                should_select_slot: true,
            });
            return false;
        }

        let inv = player.inventory();
        inv.set_selected_slot(slot);
        let stack = inv.held_item();
        let equipment = &[(EquipmentSlot::MAIN_HAND, stack)];
        player.living_entity.send_equipment_changes(equipment);
        player.inventory().get_selected_slot() == slot
    }
}
