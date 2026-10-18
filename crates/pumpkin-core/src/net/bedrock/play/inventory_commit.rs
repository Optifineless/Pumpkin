use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{
    inventory::slot_transaction,
    screen_handler::{InventoryPlayer, ScreenHandler},
};
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
    player: &dyn InventoryPlayer,
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
            // A swap also takes stock when only the physical stack identity changes.
            if !expected.is_empty()
                && (replacement.uid != expected.uid
                    || !expected.are_items_and_components_equal(replacement)
                    || replacement.item_count < expected.item_count)
                && !slot.can_take_items(player)
            {
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
            let inventory = slot.get_inventory();
            // The owners already exclude writers; do not reacquire them through exchange.
            inventory.update_slot_storage(slot.get_index(), &mut |current| {
                *current = (*replacement).clone();
                inventory.on_slot_replaced(slot.get_index(), expected, current);
            });
        }
    }
    drop(guards);
    for slot in slots.into_iter().flatten() {
        slot.get_inventory().on_slot_changed(slot.get_index());
    }
    true
}

// LivingEntity.collectEquipmentChanges reads live stacks on the tick; admission queues identities.
pub(super) fn queue_equipment_change(
    player: &crate::entity::player::Player,
    handler: &dyn ScreenHandler,
    index: usize,
    replacement: &ItemStack,
) {
    use pumpkin_data::data_component_impl::EquipmentSlot;
    use pumpkin_inventory::{Inventory, screen_handler::InventoryPlayer};
    let slot = &handler.get_behaviour().slots[index];
    let inventory = player.inventory();
    if !std::sync::Arc::ptr_eq(
        &slot.get_inventory().slot_transaction(slot.get_index()),
        &inventory.slot_transaction(0),
    ) {
        return;
    }
    if slot.get_index() == usize::from(inventory.get_selected_slot()) {
        player.send_menu_equipment(&[(EquipmentSlot::MAIN_HAND, replacement.clone())]);
    } else if let Some(equipment) = inventory.equipment_slots.get(&slot.get_index()) {
        player.enqueue_equipment_change(equipment, replacement);
    }
}
