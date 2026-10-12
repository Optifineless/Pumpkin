use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{Inventory, inventory::slot_transaction};

use super::hopper::HopperBlockEntity;

impl HopperBlockEntity {
    /// Offers one conditional withdrawal; effects and any unreturnable receipt follow release.
    // HopperBlockEntity.tryTakeInItemFromSlot/ejectItems and DropperBlock.dispenseFrom.
    pub(crate) fn transfer_one(
        from: &dyn Inventory,
        to: &dyn Inventory,
        slot: usize,
        expected: &ItemStack,
        face: Option<pumpkin_data::BlockDirection>,
    ) -> (bool, ItemStack) {
        let owners = slot_transaction::ordered_owners(&[from, to]);
        let guards = pumpkin_inventory::inventory::slot_transaction::lock_owners(&owners);
        let one = slot_transaction::withdraw(from, slot, expected, 1);
        if one.is_empty() {
            return (false, one);
        }
        let (remaining, changed) = Self::add_item_from_face_deferred(Some(from), to, one, face);
        let moved = remaining.is_empty();
        let leftover = slot_transaction::refund(from, slot, expected, remaining);
        let source_changed = !from.get_stack(slot).are_equal(expected);
        drop(guards);
        if source_changed {
            from.on_slot_changed(slot);
        }
        for slot in changed {
            to.on_slot_changed(slot);
        }
        (moved, leftover)
    }
}
