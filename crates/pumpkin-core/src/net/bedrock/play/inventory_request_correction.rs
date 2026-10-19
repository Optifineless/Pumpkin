use super::{
    BedrockClient, CInventoryContent, ContainerName, FullContainerName, NetworkItemStackDescriptor,
    Player, PlayerInventory, VarUInt,
};
use pumpkin_protocol::bedrock::client::inventory_slot::CInventorySlot;

impl BedrockClient {
    pub(super) fn correct_stack_request_prediction(&self, player: &Player) {
        self.correct_inventory_prediction(player);
        let effects = crate::world::guard_effects::GuardEffects::new();
        let snapshot = {
            let current = player
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let handler = current
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            handler.window_type().map(|window_type| {
                let slots = &handler.get_behaviour().slots;
                let count = slots.len().saturating_sub(PlayerInventory::MAIN_SIZE);
                (
                    handler.sync_id(),
                    window_type,
                    slots[..count]
                        .iter()
                        .map(|slot| slot.get_cloned_stack())
                        .collect::<Vec<_>>(),
                )
            })
        };
        drop(effects);
        if let Some((window, window_type, slots)) = snapshot {
            // Use the same addresses as Player::enqueue_slot_packet, including crafting.
            if window_type == pumpkin_data::screen::WindowType::Crafting {
                for (index, stack) in slots.iter().enumerate() {
                    let (name, slot) = if index == 0 {
                        (ContainerName::CreatedOutput, 0)
                    } else {
                        (ContainerName::CraftingInput, 32 + index as u32 - 1)
                    };
                    self.try_enqueue_client_packet(&CInventorySlot {
                        container_id: VarUInt(u32::from(window)),
                        slot: VarUInt(slot),
                        full_container_name: Some(FullContainerName {
                            container_name: name,
                            dynamic_id: None,
                        }),
                        storage_item: None,
                        item: NetworkItemStackDescriptor::from(stack),
                    });
                }
            } else {
                self.try_enqueue_client_packet(&CInventoryContent {
                    container_id: VarUInt(u32::from(window)),
                    slots: slots.iter().map(NetworkItemStackDescriptor::from).collect(),
                    full_container_name: FullContainerName {
                        container_name: ContainerName::LevelEntity,
                        dynamic_id: None,
                    },
                    storage_item: NetworkItemStackDescriptor::default(),
                });
            }
        }
    }
}
