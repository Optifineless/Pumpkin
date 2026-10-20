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

// Menu-local UI addresses follow Geyser's edition-specific container translators.
pub(super) fn bedrock_menu_slot(
    window_type: Option<pumpkin_data::screen::WindowType>,
    total_slots: usize,
    slot: usize,
) -> Option<(ContainerName, u8)> {
    use pumpkin_data::screen::WindowType;

    let container_slots = total_slots.checked_sub(36)?;
    if slot >= total_slots {
        return None;
    }
    if slot >= container_slots {
        let inventory_slot = slot - container_slots;
        return Some((
            ContainerName::Inventory,
            if inventory_slot < 27 {
                (inventory_slot + 9) as u8
            } else {
                (inventory_slot - 27) as u8
            },
        ));
    }
    Some(match window_type {
        Some(WindowType::Crafting) if slot == 0 => (ContainerName::CreatedOutput, 0),
        Some(WindowType::Crafting) => (ContainerName::CraftingInput, (32 + slot - 1) as u8),
        Some(WindowType::Loom) => match slot {
            0 => (ContainerName::LoomInput, 9),
            1 => (ContainerName::LoomDye, 10),
            2 => (ContainerName::LoomMaterial, 11),
            3 => (ContainerName::LoomResultPreview, 50),
            _ => return None,
        },
        Some(WindowType::Beacon) if slot == 0 => (ContainerName::BeaconPayment, 27),
        _ => (ContainerName::LevelEntity, slot as u8),
    })
}
