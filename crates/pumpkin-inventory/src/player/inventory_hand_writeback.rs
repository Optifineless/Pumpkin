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
