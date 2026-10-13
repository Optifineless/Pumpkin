use crate::{Inventory, inventory::slot_transaction, screen_handler::InventoryPlayer, slot::Slot};
use pumpkin_data::item_stack::ItemStack;

// AbstractContainerMenu.doClick SWAP: claim both stacks in one admitted transaction.
pub fn click(slot: &dyn Slot, button: usize, player: &dyn InventoryPlayer) {
    let source = slot.get_inventory();
    let destination = player.get_inventory();
    let expected = slot.get_stack();
    let button_stack = destination.get_stack(button);
    let can_take = slot.can_take_items(player);
    let can_insert = slot.can_insert(&button_stack);
    let max = slot.get_max_item_count_for_stack(&button_stack);
    if (!expected.is_empty() && !can_take) || (!button_stack.is_empty() && !can_insert) {
        return;
    }
    if expected.is_empty() && button_stack.is_empty() {
        return;
    }
    let owners = slot_transaction::ordered_owners(&[source.as_ref(), destination.as_ref()]);
    let guards = crate::inventory::slot_transaction::lock_owners(&owners);
    let current = source.get_stack(slot.get_index());
    let hotbar = destination.get_stack(button);
    if current.uid != expected.uid
        || !current.are_equal(&expected)
        || hotbar.uid != button_stack.uid
        || !hotbar.are_equal(&button_stack)
        || !owners.iter().all(|owner| owner.is_live())
    {
        return;
    }
    // Swapping a slot with itself must leave the stack alone.
    if std::sync::Arc::ptr_eq(
        &source,
        &(destination.clone() as std::sync::Arc<dyn Inventory>),
    ) && slot.get_index() == button
    {
        return;
    }
    let mut button_remainder = slot_transaction::withdraw(
        destination.as_ref(),
        button,
        &button_stack,
        button_stack.item_count,
    );
    let replacement = button_remainder.split(max.min(button_remainder.item_count));
    let mut taken = slot_transaction::withdraw(
        source.as_ref(),
        slot.get_index(),
        &expected,
        expected.item_count,
    );
    source.update_slot(slot.get_index(), &mut |current| {
        source.on_slot_replaced(slot.get_index(), current, &replacement);
        *current = replacement.clone();
    });
    let overflow = !button_remainder.is_empty();
    destination.update_slot(button, &mut |current| {
        *current = button_remainder.clone();
    });
    drop(guards);
    // CartographyTableMenu.onTake post-processes only admitted, owned stock.
    // Keep the receipt local across callbacks; later insertion preserves competing edits.
    if !taken.is_empty() {
        slot.on_crafted_by(player, &mut taken);
    }
    let taken_for_callback = taken.clone();
    let owner = destination.slot_transaction(button);
    let guard = owner.lock();
    if !overflow && !taken.is_empty() {
        insert_into_hotbar(destination.as_ref(), button, &mut taken);
    }
    if !taken.is_empty() {
        destination.insert_stack_anywhere(&mut taken);
    }
    drop(guard);
    if !taken_for_callback.is_empty() {
        slot.on_take_item(player, &taken_for_callback);
    }
    source.on_slot_changed(slot.get_index());
    destination.on_slot_changed(button);
    if !taken.is_empty() {
        player.drop_item(taken, true);
    }
}

// AbstractContainerMenu.doClick SWAP keeps the moved stack instance for a whole hotbar replacement.
// The caller owns the destination; retain the receipt if it retired while callbacks ran.
fn insert_into_hotbar(destination: &dyn Inventory, button: usize, taken: &mut ItemStack) {
    let current = destination.get_stack(button);
    let max = taken.get_max_stack_size();
    if current.is_empty()
        && taken.item_count <= max
        && slot_transaction::exchange(destination, button, &current, taken.clone())
    {
        *taken = ItemStack::EMPTY.clone();
    } else {
        slot_transaction::deposit(destination, button, taken, max);
    }
}
