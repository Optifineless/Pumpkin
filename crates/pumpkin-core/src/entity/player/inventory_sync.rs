use super::Player;
use crate::net::bedrock::BedrockClient;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_protocol::{
    bedrock::{
        client::inventory_slot::CInventorySlot,
        network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
        server::inventory_transaction::{WINDOW_ID_INVENTORY, WINDOW_ID_OFF_HAND},
    },
    codec::var_uint::VarUInt,
    java::client::play::CSetPlayerInventory,
};

impl Player {
    pub(super) fn send_bedrock_inventory_slot(
        &self,
        bedrock: &BedrockClient,
        packet: &CSetPlayerInventory,
    ) {
        // PlayerInventory's Java off-hand index maps to Bedrock's separate off-hand container.
        let (container_id, slot, container_name) =
            if packet.slot.0 == PlayerInventory::OFF_HAND_SLOT as i32 {
                (WINDOW_ID_OFF_HAND, 0, ContainerName::Offhand)
            } else {
                (WINDOW_ID_INVENTORY, packet.slot.0, ContainerName::Inventory)
            };
        let bedrock_packet = CInventorySlot {
            container_id: VarUInt(container_id as u32),
            slot: VarUInt(slot as u32),
            full_container_name: Some(FullContainerName {
                container_name,
                dynamic_id: None,
            }),
            storage_item: None,
            item: NetworkItemStackDescriptor::from(&*packet.item.0),
        };
        if let Ok(data) = bedrock.serialize_packet(&bedrock_packet) {
            crate::world::guard_effects::send(self.client.clone(), data);
        }
    }
}
