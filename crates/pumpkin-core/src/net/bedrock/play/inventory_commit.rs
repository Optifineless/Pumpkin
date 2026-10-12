use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{inventory::slot_transaction, screen_handler::ScreenHandler};
use pumpkin_protocol::bedrock::server::item_stack_request::ItemStackRequestSlotInfo;

use super::item_stack_request::map_bedrock_container_slot;
use pumpkin_protocol::bedrock::network_item::ContainerName;

/// Stages an exact insertion into a real slot or the cursor, retaining stock on rejection.
// AbstractContainerMenu.doClick -> Slot.safeInsert respects eligibility and capacity.
pub(super) fn virtual_destination(
    handler: &dyn ScreenHandler,
    destination: &ItemStackRequestSlotInfo,
    stock: &ItemStack,
    expected: &ItemStack,
    count: u8,
) -> Option<ItemStack> {
    if !expected.is_empty() && !expected.are_items_and_components_equal(stock) {
        return None;
    }
    if count == 0 || count > stock.item_count {
        return None;
    }
    let mut replacement = if expected.is_empty() {
        stock.copy_with_count(count)
    } else {
        expected.clone()
    };
    // ItemStack.getCount reports zero for every empty stack, including AIR.
    let current_count = if expected.is_empty() {
        0
    } else {
        expected.item_count
    };
    replacement.item_count = current_count.checked_add(count)?;
    let maximum = if destination.container_name.container_name == ContainerName::Cursor {
        stock.get_max_stack_size()
    } else {
        let index = map_bedrock_container_slot(
            handler,
            destination.container_name.container_name,
            destination.slot_id,
        )?;
        let slot = handler.get_behaviour().slots.get(index)?;
        if !slot.can_insert(&replacement) {
            return None;
        }
        slot.get_max_item_count_for_stack(&replacement)
    };
    (replacement.item_count <= maximum).then_some(replacement)
}

/// Commits packet observations together; effects are published by the handler after release.
// AbstractContainerMenu.doClick: one action consumes precisely the stock it observed.
pub(super) fn commit(
    handler: &dyn ScreenHandler,
    changes: &[(&ItemStackRequestSlotInfo, &ItemStack, &ItemStack)],
) -> bool {
    let slots = changes
        .iter()
        .map(|(info, _, _)| {
            map_bedrock_container_slot(handler, info.container_name.container_name, info.slot_id)
                .and_then(|index| handler.get_behaviour().slots.get(index).cloned())
        })
        .collect::<Vec<_>>();
    // An unmapped container is never a cursor; reject before changing any physical slot.
    if changes.iter().zip(&slots).any(|((info, _, _), slot)| {
        slot.is_none() && info.container_name.container_name != ContainerName::Cursor
    }) {
        return false;
    }
    let inventories = slots
        .iter()
        .flatten()
        .map(|slot| slot.get_inventory())
        .collect::<Vec<_>>();
    let refs = inventories.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    let owners = slot_transaction::ordered_owners(&refs);
    for (index, slot) in slots.iter().enumerate() {
        if let Some(slot) = slot
            && slots[..index].iter().flatten().any(|other| {
                other.get_index() == slot.get_index()
                    && std::sync::Arc::ptr_eq(&other.get_inventory(), &slot.get_inventory())
            })
        {
            return false;
        }
    }
    let guards = pumpkin_inventory::inventory::slot_transaction::lock_owners(&owners);
    if !owners.iter().all(|owner| owner.is_live()) {
        return false;
    }
    for ((_, expected, replacement), slot) in changes.iter().zip(&slots) {
        if let Some(slot) = slot {
            let current = slot.get_stack();
            if current.uid != expected.uid || !current.are_equal(expected) {
                return false;
            }
            if !replacement.is_empty()
                && (expected.is_empty()
                    || !expected.are_items_and_components_equal(replacement)
                    || replacement.item_count > expected.item_count)
                && (!slot.can_insert(replacement)
                    || replacement.item_count > slot.get_max_item_count_for_stack(replacement))
            {
                return false;
            }
        } else {
            let current = handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if current.uid != expected.uid
                || !current.are_equal(expected)
                || (replacement.item_count > expected.item_count
                    && replacement.item_count > replacement.get_max_stack_size())
            {
                return false;
            }
        }
    }
    for ((_, expected, replacement), slot) in changes.iter().zip(&slots) {
        if let Some(slot) = slot {
            let committed = slot_transaction::exchange(
                slot.get_inventory().as_ref(),
                slot.get_index(),
                expected,
                (*replacement).clone(),
            );
            debug_assert!(committed);
        }
    }
    drop(guards);
    for slot in slots.into_iter().flatten() {
        slot.get_inventory().on_slot_changed(slot.get_index());
    }
    true
}
