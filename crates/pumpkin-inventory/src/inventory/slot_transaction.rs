//! Container ownership precedes storage locks. Multi-container operations acquire owners in
//! pointer order.
//!
//! Player saving may hold its storage-session lock before owners. Owners never acquire a
//! storage session; screen closure finishes before the leave path acquires that session.
//!
//! Screen handlers may acquire owners, but an owner must never acquire a screen
//! handler, call a plugin, send packets or notify the world. Reentrancy only permits local slot
//! primitives beneath an already admitted transfer; it does not permit callbacks under owners.

use super::Inventory;
use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use pumpkin_data::item_stack::ItemStack;
use smallvec::SmallVec;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Capacity for short-lived transaction metadata.
pub const INLINE_CAPACITY: usize = 4;

/// The common single/double-container cases retain owners without heap allocation.
pub type Owners = SmallVec<[Arc<Transaction>; INLINE_CAPACITY]>;
/// Guards have the same inline capacity as their backing owners.
pub type Guards<'a> = SmallVec<[ReentrantMutexGuard<'a, ()>; INLINE_CAPACITY]>;
/// Automation normally changes one destination slot.
pub type ChangedSlots = SmallVec<[usize; INLINE_CAPACITY]>;

/// Acquires the already ordered owners without allocating for ordinary transfers.
#[must_use]
pub fn lock_owners(owners: &[Arc<Transaction>]) -> Guards<'_> {
    owners.iter().map(|owner| owner.lock()).collect()
}

/// Owns the lifetime and mutation ordering of one backing container.
pub struct Transaction {
    lock: ReentrantMutex<()>,
    retired: AtomicBool,
}

impl Default for Transaction {
    fn default() -> Self {
        Self {
            lock: ReentrantMutex::new(()),
            retired: AtomicBool::new(false),
        }
    }
}

impl Transaction {
    pub fn lock(&self) -> ReentrantMutexGuard<'_, ()> {
        self.lock.lock()
    }

    pub fn try_lock(&self) -> Option<ReentrantMutexGuard<'_, ()>> {
        self.lock.try_lock()
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
    }

    // Retirement is committed under the owner, after any admitted transfer has settled.
    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
}

/// Orders the owners of both sides so opposing transfers cannot deadlock.
/// Destination slot updates must participate, including deposits into a withdrawn source.
#[must_use]
pub fn ordered_owners<I: Inventory + ?Sized>(inventories: &[&I]) -> Owners {
    let mut owners = Owners::new();
    for inventory in inventories {
        inventory.collect_transaction_owners(&mut owners);
    }
    owners.sort_unstable_by_key(Arc::as_ptr);
    owners.dedup_by(|first, second| Arc::ptr_eq(first, second));
    owners
}

/// Stops new mutations after all admitted transfers have settled.
pub fn retire(inventory: &dyn Inventory) {
    let owners = ordered_owners(&[inventory]);
    let _guards = lock_owners(&owners);
    for owner in &owners {
        owner.retire();
    }
}

/// Retires and drains a container; returns owned stock for scattering after release.
#[must_use]
pub fn retire_and_drain(inventory: &dyn Inventory) -> Vec<ItemStack> {
    let owners = ordered_owners(&[inventory]);
    let guards = lock_owners(&owners);
    for owner in &owners {
        owner.retire();
    }
    let mut stacks = Vec::new();
    for slot in 0..inventory.size() {
        inventory.update_slot_storage(slot, &mut |stack| {
            if !stack.is_empty() {
                stacks.push(std::mem::replace(stack, ItemStack::EMPTY.clone()));
            }
        });
    }
    drop(guards);
    stacks
}

/// Clears one settled inventory, then publishes only committed slot changes.
pub fn clear(inventory: &dyn Inventory) {
    let owners = ordered_owners(&[inventory]);
    let guards = lock_owners(&owners);
    let mut changed = ChangedSlots::new();
    for slot in 0..inventory.size() {
        inventory.update_slot(slot, &mut |stack| {
            if !stack.is_empty() {
                *stack = ItemStack::EMPTY.clone();
                changed.push(slot);
            }
        });
    }
    drop(guards);
    for slot in changed {
        inventory.on_slot_changed(slot);
    }
}

/// Replaces an observed stack atomically, returning false if another mover changed it.
#[must_use]
pub fn exchange(
    inventory: &dyn Inventory,
    slot: usize,
    expected: &ItemStack,
    replacement: ItemStack,
) -> bool {
    let mut replacement = Some(replacement);
    inventory.update_slot(slot, &mut |current| {
        if current.uid == expected.uid
            && current.are_equal(expected)
            && let Some(stack) = replacement.take()
        {
            *current = stack;
            inventory.on_slot_replaced(slot, expected, current);
        }
    });
    replacement.is_none()
}

/// Inserts owned items under the destination's lock; the caller keeps all excess.
pub fn deposit(inventory: &dyn Inventory, slot: usize, stack: &mut ItemStack, max: u8) {
    inventory.update_slot(slot, &mut |current| {
        let empty = current.is_empty();
        return_to_stack(current, stack, max);
        if empty && !current.is_empty() {
            inventory.on_slot_replaced(slot, ItemStack::EMPTY, current);
        }
    });
}

/// Clones one settled slot for serialization without retaining a storage or transfer guard.
#[must_use]
pub fn snapshot_slot(inventory: &dyn Inventory, slot: usize) -> ItemStack {
    let owner = inventory.slot_transaction(slot);
    let _transfer = owner.lock();
    inventory.get_stack(slot)
}

/// Withdraws only from the observed stack identity and count, in one storage mutation.
#[must_use]
pub fn withdraw(
    inventory: &dyn Inventory,
    slot: usize,
    expected: &ItemStack,
    amount: u8,
) -> ItemStack {
    let mut taken = ItemStack::EMPTY.clone();
    inventory.update_slot(slot, &mut |stack| {
        if stack.uid == expected.uid
            && stack.are_equal(expected)
            && amount > 0
            && amount <= stack.item_count
        {
            taken = if amount == stack.item_count {
                std::mem::replace(stack, ItemStack::EMPTY.clone())
            } else {
                stack.split(amount)
            };
        }
    });
    taken
}

/// Returns only an unaccepted withdrawal; a concurrent write is never replaced by a snapshot.
#[must_use]
pub fn refund(
    inventory: &dyn Inventory,
    slot: usize,
    expected: &ItemStack,
    mut stack: ItemStack,
) -> ItemStack {
    if stack.is_empty() {
        return stack;
    }
    inventory.update_slot(slot, &mut |current| {
        let max = current.get_max_stack_size().max(
            if current.is_empty() || current.uid == expected.uid {
                expected.item_count
            } else {
                0
            },
        );
        return_to_stack(current, &mut stack, max);
    });
    stack
}

/// Returns a withdrawal without replacing a stack another writer put in its place.
/// Any excess stays owned by the caller and must be returned or dropped after unlocking.
pub fn return_to_stack(current: &mut ItemStack, stack: &mut ItemStack, max: u8) {
    if stack.is_empty() {
        return;
    }
    if current.is_empty() {
        *current = stack.split(stack.item_count.min(max));
    } else if current.are_items_and_components_equal(stack) {
        let count = stack.item_count.min(max.saturating_sub(current.item_count));
        current.increment(count);
        stack.decrement(count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimpleInventory;
    use pumpkin_data::item::Item;

    #[test]
    fn withdrawal_rejects_a_replacement_with_the_same_item_and_count() {
        let inventory = SimpleInventory::new(1);
        inventory.set_stack(0, ItemStack::new(8, &Item::DIAMOND));
        let expected = inventory.get_stack(0);
        inventory.set_stack(0, ItemStack::new(8, &Item::DIAMOND));
        assert!(withdraw(&inventory, 0, &expected, 8).is_empty());
        assert_eq!(inventory.get_stack(0).item_count, 8);
    }

    #[test]
    fn withdrawal_rejects_more_than_the_observed_stock() {
        let inventory = SimpleInventory::new(1);
        inventory.set_stack(0, ItemStack::new(8, &Item::DIAMOND));
        let expected = inventory.get_stack(0);
        assert!(withdraw(&inventory, 0, &expected, 9).is_empty());
        assert_eq!(inventory.get_stack(0).item_count, 8);
    }

    #[test]
    fn retirement_drains_stock_and_rejects_late_refunds() {
        let inventory = SimpleInventory::new(1);
        inventory.set_stack(0, ItemStack::new(8, &Item::DIAMOND));
        let expected = inventory.get_stack(0);
        let receipt = withdraw(&inventory, 0, &expected, 1);
        let scattered = retire_and_drain(&inventory);
        let leftover = refund(&inventory, 0, &expected, receipt);
        assert_eq!(
            scattered
                .iter()
                .map(|stack| u32::from(stack.item_count))
                .sum::<u32>(),
            7
        );
        assert_eq!(leftover.item_count, 1);
        assert!(inventory.is_empty());
    }

    #[test]
    fn failed_transfer_returns_only_its_withdrawal_after_another_removal() {
        let inventory = SimpleInventory::new(1);
        inventory.set_stack(0, ItemStack::new(8, &Item::DIAMOND));
        let expected = inventory.get_stack(0);
        let one = withdraw(&inventory, 0, &expected, 1);
        let other = inventory.remove_stack_specific(0, 2);
        assert!(refund(&inventory, 0, &expected, one).is_empty());
        assert_eq!(inventory.get_stack(0).item_count, 6);
        assert_eq!(other.item_count, 2);
    }
}
