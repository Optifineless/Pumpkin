use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{inventory::slot_transaction, screen_handler::ScreenHandler};
use pumpkin_protocol::bedrock::server::item_stack_request::ItemStackRequestSlotInfo;

use super::item_stack_request::map_bedrock_container_slot;

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
                .map(|index| handler.get_behaviour().slots[index].clone())
        })
        .collect::<Vec<_>>();
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
    for ((_, expected, _), slot) in changes.iter().zip(&slots) {
        if let Some(slot) = slot {
            let current = slot.get_stack();
            if current.uid != expected.uid || !current.are_equal(expected) {
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
