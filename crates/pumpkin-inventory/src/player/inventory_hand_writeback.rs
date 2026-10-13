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
        // ServerPlayerGameMode.useItemOn retains the source reference; share container ownership.
        crate::inventory::slot_transaction::exchange(self, slot, before, after)
    }
}
