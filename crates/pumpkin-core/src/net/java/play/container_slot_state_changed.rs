#[allow(clippy::wildcard_imports)]
use super::*;
use crate::block::entities::crafter::CrafterBlockEntity;
use pumpkin_data::screen::WindowType;
use pumpkin_inventory::{
    Inventory, generic_container_screen_handler::GenericContainerScreenHandler,
};
use pumpkin_protocol::java::server::play::SContainerSlotStateChanged;

impl JavaClient {
    pub fn handle_container_slot_state_changed(
        &self,
        player: &Player,
        packet: &SContainerSlotStateChanged,
    ) {
        debug!(
            "Player {} container {} slot {} state changed to {}",
            player.gameprofile.name, packet.container_id.0, packet.slot_id.0, packet.new_state
        );

        if player.is_spectator() || u8::try_from(packet.container_id.0).is_err() {
            return;
        }

        // ServerGamePacketListenerImpl.handleContainerSlotStateChanged only applies this packet
        // to the player's current CrafterMenu and its CrafterBlockEntity.
        let screen = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let inventory: Arc<dyn Inventory> = {
            let handler = screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if i32::from(handler.sync_id()) != packet.container_id.0
                || handler.window_type() != Some(WindowType::Crafter3x3)
            {
                return;
            }
            let Some(crafter_menu) = handler
                .as_any()
                .downcast_ref::<GenericContainerScreenHandler>()
            else {
                return;
            };
            crafter_menu.inventory.clone()
        };

        let Ok(slot) = usize::try_from(packet.slot_id.0) else {
            return;
        };
        let Some(crafter) = inventory.as_any().downcast_ref::<CrafterBlockEntity>() else {
            return;
        };
        if !crafter.slot_can_be_disabled(slot) {
            return;
        }

        crafter.set_slot_state(slot, packet.new_state);

        // CrafterBlockEntity releases its slot and transaction locks before dirtying. Publish the
        // corresponding ten container properties after the mutation, matching CrafterMenu's
        // broadcastChanges call.
        let current_screen = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if Arc::ptr_eq(&screen, &current_screen) {
            Player::send_menu_updates(&screen);
        }
    }
}
