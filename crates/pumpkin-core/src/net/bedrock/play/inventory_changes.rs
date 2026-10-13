use super::{
    BedrockClient, ContainerName, EquipmentSlot, FullContainerName, ItemStack,
    NetworkItemStackDescriptor, Player, ScreenHandler, VarUInt, map_bedrock_slot_to_screen_handler,
};
use pumpkin_inventory::{
    player::player_inventory::PlayerInventory, screen_handler::InventoryPlayer,
};
use pumpkin_protocol::bedrock::{
    client::inventory_slot::CInventorySlot,
    server::inventory_transaction::{WINDOW_ID_ARMOUR, WINDOW_ID_INVENTORY, WINDOW_ID_OFF_HAND},
};

pub(super) struct InventorySnapshot {
    stacks: [ItemStack; PlayerInventory::OFF_HAND_SLOT + 1],
    main_hand: ItemStack,
    selected: u8,
}

impl InventorySnapshot {
    pub(super) fn capture(player: &Player) -> Self {
        let inventory = player.inventory();
        let main = inventory
            .main_inventory
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut stacks = std::array::from_fn(|_| ItemStack::EMPTY.clone());
        stacks[..PlayerInventory::MAIN_SIZE].clone_from_slice(&main);
        {
            let equipment = inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (slot, equipment_slot) in inventory.equipment_slots.iter() {
                stacks[*slot] = equipment.get(equipment_slot);
            }
        }
        let selected = inventory.get_selected_slot();
        let main_hand = stacks[usize::from(selected)].clone();
        Self {
            stacks,
            main_hand,
            selected,
        }
    }
}

impl BedrockClient {
    pub(super) fn publish_inventory_changes(
        &self,
        player: &Player,
        before: &InventorySnapshot,
        correct_prediction: bool,
    ) {
        let after = InventorySnapshot::capture(player);
        // Retain the received-slot cache update from consuming block uses, for either hand.
        if correct_prediction
            || before
                .stacks
                .iter()
                .zip(&after.stacks)
                .any(|(a, b)| a.uid != b.uid || !a.are_equal(b))
        {
            let mut handler = player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (slot, (old, new)) in before.stacks.iter().zip(&after.stacks).enumerate() {
                if (correct_prediction || old.uid != new.uid || !old.are_equal(new))
                    && let Some((window, _, index)) = Self::inventory_slot_address(player, slot)
                    && let Some(screen_slot) =
                        map_bedrock_slot_to_screen_handler(window, index as u32)
                {
                    handler.set_received_stack(screen_slot, new.clone());
                }
            }
        }
        // AbstractContainerMenu.broadcastChanges synchronizes only changed slots.
        // Unsupported client writes still need the complete authoritative correction.
        if correct_prediction {
            self.correct_inventory_prediction(player);
        } else {
            for (slot, (old, new)) in before.stacks.iter().zip(&after.stacks).enumerate() {
                if old.uid != new.uid || !old.are_equal(new) {
                    self.send_changed_inventory_slot(player, slot, new);
                }
            }
        }
        // LivingEntity.detectEquipmentUpdates refreshes modifiers and publishes on the tick.
        if before.selected != after.selected
            || before.main_hand.uid != after.main_hand.uid
            || !before.main_hand.are_equal(&after.main_hand)
        {
            player.enqueue_equipment_change(&EquipmentSlot::MAIN_HAND, &after.main_hand);
        }
        let offhand = PlayerInventory::OFF_HAND_SLOT;
        if before.stacks[offhand].uid != after.stacks[offhand].uid
            || !before.stacks[offhand].are_equal(&after.stacks[offhand])
        {
            player.enqueue_equipment_change(&EquipmentSlot::OFF_HAND, &after.stacks[offhand]);
        }
    }

    fn send_changed_inventory_slot(&self, player: &Player, slot: usize, stack: &ItemStack) {
        let Some((window, name, index)) = Self::inventory_slot_address(player, slot) else {
            return;
        };
        self.try_enqueue_client_packet(&CInventorySlot {
            container_id: VarUInt(window as u32),
            slot: VarUInt(index as u32),
            full_container_name: Some(FullContainerName {
                container_name: name,
                dynamic_id: None,
            }),
            storage_item: None,
            item: NetworkItemStackDescriptor::from(stack),
        });
    }
    fn inventory_slot_address(player: &Player, slot: usize) -> Option<(i32, ContainerName, usize)> {
        if slot < PlayerInventory::MAIN_SIZE {
            Some((WINDOW_ID_INVENTORY, ContainerName::Inventory, slot))
        } else if slot == PlayerInventory::OFF_HAND_SLOT {
            Some((WINDOW_ID_OFF_HAND, ContainerName::Offhand, 0))
        } else {
            let equipment_slot = player.inventory().equipment_slots.get(&slot)?;
            let index = match *equipment_slot {
                EquipmentSlot::Head(_) => 0,
                EquipmentSlot::Chest(_) => 1,
                EquipmentSlot::Legs(_) => 2,
                EquipmentSlot::Feet(_) => 3,
                _ => return None,
            };
            Some((WINDOW_ID_ARMOUR, ContainerName::Armor, index))
        }
    }
}
