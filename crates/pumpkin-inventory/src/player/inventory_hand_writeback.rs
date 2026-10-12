use super::player_inventory::PlayerInventory;
use pumpkin_data::item_stack::ItemStack;

impl PlayerInventory {
    /// Replaces a slot only if its stack identity and value still match the interaction snapshot.
    /// No callbacks or packet dispatch run while the inventory is locked.
    pub fn replace_stack_if_unchanged(
        &self,
        slot: usize,
        before: &ItemStack,
        after: ItemStack,
    ) -> bool {
        // Vanilla ServerPlayerGameMode.useItemOn operates on the original stack reference.
        // Pumpkin uses clones, so compare and replace under the same inventory guard.
        if slot < Self::MAIN_SIZE {
            let mut inventory = self
                .main_inventory
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = &mut inventory[slot];
            if current.uid != before.uid || !current.are_equal(before) {
                return false;
            }
            *current = after;
        } else if let Some(equipment_slot) = self.equipment_slots.get(&slot) {
            let mut equipment = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = equipment.get(equipment_slot);
            if current.uid != before.uid || !current.are_equal(before) {
                return false;
            }
            equipment.put(equipment_slot, after);
        } else {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Inventory, entity_equipment::EntityEquipment};
    use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item};
    use rustc_hash::FxHashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn stale_hand_snapshot_cannot_replace_new_identity_or_changed_count() {
        let inventory = PlayerInventory::new(
            Arc::new(Mutex::new(EntityEquipment::new())),
            Arc::new(FxHashMap::from_iter([(
                PlayerInventory::OFF_HAND_SLOT,
                EquipmentSlot::OFF_HAND,
            )])),
        );
        for slot in [4, PlayerInventory::OFF_HAND_SLOT] {
            let before = ItemStack::new(2, &Item::STONE);
            let after = before.copy_with_count(1);
            let replacement = ItemStack::new(2, &Item::STONE);
            inventory.set_stack(slot, replacement.clone());
            assert!(!inventory.replace_stack_if_unchanged(slot, &before, after.clone()));
            assert_eq!(inventory.get_stack(slot).uid, replacement.uid);

            // A second mutation of the same originating stack must also take precedence.
            let mut changed = before.clone();
            changed.set_count(3);
            inventory.set_stack(slot, changed);
            assert!(!inventory.replace_stack_if_unchanged(slot, &before, after.clone()));
            assert_eq!(inventory.get_stack(slot).item_count, 3);

            inventory.set_stack(slot, before.clone());
            assert!(inventory.replace_stack_if_unchanged(slot, &before, after));
            assert_eq!(inventory.get_stack(slot).item_count, 1);
        }
    }
}
