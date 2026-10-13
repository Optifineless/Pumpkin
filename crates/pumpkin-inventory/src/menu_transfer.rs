use smallvec::SmallVec;
use std::sync::Arc;

use pumpkin_data::item_stack::ItemStack;

use crate::{
    inventory::slot_transaction::{self, INLINE_CAPACITY},
    slot::Slot,
};

struct Move {
    slot: Arc<dyn Slot>,
    expected: ItemStack,
    count: u8,
}

// AbstractContainerMenu.moveItemStackTo merges first, then fills one empty slot.
fn plan(
    slots: &[Arc<dyn Slot>],
    stack: &ItemStack,
    start: i32,
    end: i32,
    reverse: bool,
) -> SmallVec<[Move; INLINE_CAPACITY]> {
    let mut moves = SmallVec::new();
    let mut remaining = stack.item_count;
    for empty_only in [false, true] {
        if !empty_only && !stack.is_stackable() {
            continue;
        }
        for offset in 0..end - start {
            if remaining == 0 {
                break;
            }
            let index = if reverse {
                end - 1 - offset
            } else {
                start + offset
            };
            let slot = slots[index as usize].clone();
            let target = slot.get_stack();
            let capacity = if empty_only {
                if target.is_empty() && slot.can_insert(stack) {
                    slot.get_max_item_count_for_stack(stack)
                } else {
                    0
                }
            } else if !target.is_empty() && target.are_items_and_components_equal(stack) {
                slot.get_max_item_count_for_stack(stack)
                    .saturating_sub(target.item_count)
            } else {
                0
            };
            let count = capacity.min(remaining);
            if count > 0 {
                moves.push(Move {
                    slot,
                    expected: target,
                    count,
                });
                remaining -= count;
                if empty_only {
                    break;
                }
            }
        }
    }
    moves
}

/// Moves only stock withdrawn from the source; all notifications follow the paired commit.
/// Updates `before` for callback accounting, including earlier offers in this click.
pub fn transfer(
    slots: &[Arc<dyn Slot>],
    source_slot: &dyn Slot,
    observed: &mut ItemStack,
    range: (i32, i32, bool),
    before: &mut ItemStack,
) -> (bool, ItemStack) {
    let source = source_slot.get_inventory();
    let index = source_slot.get_index();
    let (start, end, reverse) = range;
    let mut inventories = SmallVec::<[_; INLINE_CAPACITY]>::new();
    inventories.push(source.clone());
    for slot in &slots[start as usize..end as usize] {
        let inventory = slot.get_inventory();
        if !inventories
            .iter()
            .any(|other| Arc::ptr_eq(other, &inventory))
        {
            inventories.push(inventory);
        }
    }
    let inventory_refs = inventories
        .iter()
        .map(AsRef::as_ref)
        .collect::<SmallVec<[_; INLINE_CAPACITY]>>();
    let owners = slot_transaction::ordered_owners(&inventory_refs);
    let guards = slot_transaction::lock_owners(&owners);
    let current = source.get_stack(index);
    if current.is_empty()
        || current.uid != observed.uid
        || !current.are_items_and_components_equal(observed)
    {
        return (false, ItemStack::EMPTY.clone());
    }
    let already_moved = before.item_count.saturating_sub(observed.item_count);
    // AbstractFurnaceMenu.quickMoveStack snapshots the source for onTake's item count.
    *before = current.clone();
    before.item_count = before.item_count.saturating_add(already_moved);
    *observed = current;
    let moves = plan(slots, observed, start, end, reverse);
    let amount = moves.iter().map(|planned| planned.count).sum();
    let mut receipt = slot_transaction::withdraw(source.as_ref(), index, observed, amount);
    if receipt.is_empty() {
        return (false, ItemStack::EMPTY.clone());
    }
    let offered = receipt.item_count;
    let mut changed = SmallVec::<[_; INLINE_CAPACITY]>::new();
    for planned in moves {
        let inventory = planned.slot.get_inventory();
        let mut committed = false;
        inventory.update_slot(planned.slot.get_index(), &mut |target| {
            if target.uid != planned.expected.uid || !target.are_equal(&planned.expected) {
                return;
            }
            let count = planned.count.min(receipt.item_count);
            if target.is_empty() {
                *target = receipt.split(count);
                inventory.on_slot_replaced(planned.slot.get_index(), &planned.expected, target);
            } else {
                target.increment(count);
                receipt.decrement(count);
            }
            committed = count > 0;
        });
        if committed {
            changed.push(planned.slot);
        }
    }
    let moved = offered - receipt.item_count;
    let leftover = slot_transaction::refund(source.as_ref(), index, observed, receipt);
    let source_changed = !source.get_stack(index).are_equal(observed);
    observed.decrement(moved);
    drop(guards);
    if source_changed {
        source.on_slot_changed(index);
    }
    for slot in changed {
        slot.get_inventory().on_slot_changed(slot.get_index());
    }
    (moved > 0, leftover)
}

/// Pays a generated result and moves its owned stock under the same ordered owners.
/// `pay` must only mutate owned inventory/local menu state, without callbacks.
pub fn transfer_result<T>(
    slots: &[Arc<dyn Slot>],
    source: (&dyn crate::Inventory, usize, Option<&dyn crate::Inventory>),
    expected: &ItemStack,
    range: (i32, i32, bool),
    require_full: bool,
    pay: impl FnOnce() -> Option<T>,
) -> Option<(T, ItemStack)> {
    let (inventory, index, payment) = source;
    let (start, end, reverse) = range;
    let targets = slots[start as usize..end as usize]
        .iter()
        .map(|slot| slot.get_inventory())
        .collect::<SmallVec<[_; INLINE_CAPACITY]>>();
    let mut inventory_refs = targets
        .iter()
        .map(AsRef::as_ref)
        .collect::<SmallVec<[_; INLINE_CAPACITY]>>();
    inventory_refs.push(inventory);
    if let Some(payment) = payment {
        inventory_refs.push(payment);
    }
    // Result cache, payment container/grid and destinations precede storage, ascending Arc address.
    let owners = slot_transaction::ordered_owners(&inventory_refs);
    let guards = slot_transaction::lock_owners(&owners);
    let current = inventory.get_stack(index);
    if !owners.iter().all(|owner| owner.is_live())
        || current.is_empty()
        || current.uid != expected.uid
        || !current.are_equal(expected)
    {
        return None;
    }
    let moves = plan(slots, &current, start, end, reverse);
    let amount: u8 = moves.iter().map(|planned| planned.count).sum();
    if amount == 0 || (require_full && amount != current.item_count) {
        return None;
    }
    let paid = pay()?;
    // ResultContainer.removeItem takes the whole batch; crafting drops undelivered excess.
    let mut receipt = slot_transaction::withdraw(inventory, index, &current, current.item_count);
    debug_assert_eq!(receipt.item_count, current.item_count);
    let mut changed = SmallVec::<[_; INLINE_CAPACITY]>::new();
    for planned in moves {
        let target = planned.slot.get_inventory();
        slot_transaction::deposit(
            target.as_ref(),
            planned.slot.get_index(),
            &mut receipt,
            if planned.expected.is_empty() {
                planned.count
            } else {
                planned.expected.item_count.saturating_add(planned.count)
            },
        );
        changed.push(planned.slot);
    }
    debug_assert_eq!(receipt.item_count, current.item_count - amount);
    drop(guards);
    inventory.on_slot_changed(index);
    for slot in changed {
        slot.get_inventory().on_slot_changed(slot.get_index());
    }
    Some((paid, receipt))
}
