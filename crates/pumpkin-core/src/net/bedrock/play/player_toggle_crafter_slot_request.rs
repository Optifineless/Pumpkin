#[allow(clippy::wildcard_imports)]
use super::*;

use crate::{
    block::entities::crafter::CrafterBlockEntity, entity::player::Player, net::ClientPlatform,
};
use pumpkin_inventory::generic_container_screen_handler::GenericContainerScreenHandler;
use pumpkin_protocol::bedrock::{
    client::block_actor_data::CBlockActorData, server::SPlayerToggleCrafterSlotRequest,
};

impl BedrockClient {
    pub fn handle_player_toggle_crafter_slot_request(
        &self,
        player: &Arc<Player>,
        packet: &SPlayerToggleCrafterSlotRequest,
    ) {
        let position = BlockPos::new(packet.position_x, packet.position_y, packet.position_z);
        if player.is_spectator() || player.open_container_pos.load() != Some(position) {
            return;
        }

        let world = player.world();
        let Some(block_entity) = world.get_block_entity(&position) else {
            return;
        };
        let Some(live_crafter) = block_entity.as_any().downcast_ref::<CrafterBlockEntity>() else {
            return;
        };

        let screen = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let inventory = {
            let handler = screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if handler.window_type() != Some(pumpkin_data::screen::WindowType::Crafter3x3) {
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
        let Some(menu_crafter) = inventory.as_any().downcast_ref::<CrafterBlockEntity>() else {
            return;
        };
        if !std::ptr::eq(live_crafter, menu_crafter) {
            return;
        }

        let slot = usize::from(packet.slot_index);
        if slot >= CrafterBlockEntity::INVENTORY_SIZE || !live_crafter.slot_can_be_disabled(slot) {
            return;
        }

        // Bedrock's packet says whether the slot is disabled; the entity setter accepts enabled.
        live_crafter.set_slot_state(slot, !packet.is_disabled);

        let state_id = world.get_block_state(&position).id;
        let Some(actor_data) = block_entity.bedrock_block_actor_data(state_id) else {
            return;
        };

        // Match vanilla's menu property broadcast for Java viewers and publish block-actor state
        // to Bedrock viewers whose currently open menu is this live crafter.
        let viewers = world.players.load_full();
        for viewer in viewers.iter() {
            if viewer.open_container_pos.load() != Some(position) {
                continue;
            }
            let viewer_screen = viewer
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            Player::send_menu_updates(&viewer_screen);
            if let ClientPlatform::Bedrock(client) = viewer.client.as_ref() {
                client
                    .try_enqueue_client_packet(&CBlockActorData::new(position, actor_data.clone()));
            }
        }
    }
}
