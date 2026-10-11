use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use pumpkin_data::item_stack::ItemStack;

use super::Inventory;

type Owners = HashMap<usize, Weak<Mutex<()>>>;

/// Resolves transient transfer ownership by source identity, never by the viewer's menu.
///
/// The source must stay alive until the returned owner is released. No storage lock is held
/// while this owner is locked; callers must defer dirtying, events and packets until release.
#[must_use]
pub fn owner(source: &dyn Any) -> Arc<Mutex<()>> {
    static OWNERS: OnceLock<Mutex<Owners>> = OnceLock::new();
    let key = std::ptr::from_ref(source).cast::<()>() as usize;
    let mut owners = OWNERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    owners.retain(|_, owner| owner.strong_count() != 0);
    if let Some(owner) = owners.get(&key).and_then(Weak::upgrade) {
        return owner;
    }
    let owner = Arc::new(Mutex::new(()));
    owners.insert(key, Arc::downgrade(&owner));
    owner
}

/// Orders the owners of both sides so opposing transfers cannot deadlock.
/// Destination slot updates must participate, including deposits into a withdrawn source.
#[must_use]
pub fn ordered_owners<I: Inventory + ?Sized>(inventories: &[&I]) -> Vec<Arc<Mutex<()>>> {
    let mut owners = inventories
        .iter()
        .flat_map(|inventory| {
            (0..inventory.size()).map(move |slot| inventory.slot_transaction(slot))
        })
        .collect::<Vec<_>>();
    owners.sort_unstable_by_key(Arc::as_ptr);
    owners.dedup_by(|first, second| Arc::ptr_eq(first, second));
    owners
}

/// Clones one settled slot for serialization without retaining a storage or transfer guard.
#[must_use]
pub fn snapshot_slot(inventory: &dyn Inventory, slot: usize) -> ItemStack {
    let owner = inventory.slot_transaction(slot);
    let _transfer = owner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        if stack.uid == expected.uid && stack.are_equal(expected) && !stack.is_empty() {
            taken = if amount >= stack.item_count {
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
    inventory.update_slot(slot, &mut |current| {
        let max = current
            .get_max_stack_size()
            .max(if current.uid == expected.uid {
                expected.item_count
            } else {
                0
            });
        return_to_stack(current, &mut stack, max);
    });
    stack
}

/// Returns a withdrawal without replacing a stack another writer put in its place.
/// Any excess stays owned by the caller and must be returned or dropped after unlocking.
pub fn return_to_stack(current: &mut ItemStack, stack: &mut ItemStack, max: u8) {
    if current.is_empty() {
        std::mem::swap(current, stack);
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
