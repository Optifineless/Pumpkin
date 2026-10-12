use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::inventory::slot_transaction;

use super::ItemEntity;

#[cfg(test)]
pub(super) type TransferPause = Box<dyn FnOnce() + Send>;

impl ItemEntity {
    #[cfg(test)]
    pub(crate) fn set_transfer_pause(&self, pause: TransferPause) {
        *self.transfer_pause.lock().unwrap() = Some(pause);
    }

    #[cfg(test)]
    pub(crate) fn pause_transfer(&self) {
        let pause = self.transfer_pause.lock().unwrap().take();
        if let Some(pause) = pause {
            pause();
        }
    }

    /// Transfers an owned withdrawal, then returns only the unaccepted amount.
    ///
    /// The insertion must only mutate inventory storage. Dispatch events, dirtying and packets
    /// after ownership is released; no item-stack guard is held across insertion.
    pub(crate) fn transfer_stack(
        &self,
        destination: &dyn pumpkin_inventory::Inventory,
        insert: impl FnOnce(&mut ItemStack),
    ) -> (u16, u8, u8, bool) {
        #[cfg(test)]
        self.pause_transfer();
        let mut owners = slot_transaction::ordered_owners(&[destination]);
        owners.push(self.transaction.clone());
        owners.sort_unstable_by_key(std::sync::Arc::as_ptr);
        let transfers = pumpkin_inventory::inventory::slot_transaction::lock_owners(&owners);
        let (original, mut withdrawn) = self.withdraw_for_insertion(destination);
        if withdrawn.is_empty() {
            return (original.item.id, original.item_count, 0, false);
        }
        let offered = withdrawn.item_count;
        insert(&mut withdrawn);
        let moved = offered - withdrawn.item_count;
        let mut stack = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Only this withdrawal is returned, never a snapshot predating another pickup.
        let max = stack
            .get_max_stack_size()
            .max(if stack.uid == original.uid {
                original.item_count
            } else {
                0
            });
        // A partial claim leaves stock: an independently emptied source may be discarded.
        if !self
            .entity
            .removed
            .load(std::sync::atomic::Ordering::Acquire)
            && (offered == original.item_count || !stack.is_empty())
        {
            slot_transaction::return_to_stack(&mut stack, &mut withdrawn, max);
        }
        let empty = stack.is_empty();
        drop(stack);
        drop(transfers);
        if !withdrawn.is_empty() {
            let pos = self.entity.pos.load();
            self.entity
                .world
                .load()
                .scatter_stack(pos.x, pos.y, pos.z, withdrawn);
        }
        (original.item.id, original.item_count, moved, empty)
    }

    // ItemEntity.playerTouch / HopperBlockEntity.addItem claim only available capacity.
    fn withdraw_for_insertion(
        &self,
        destination: &dyn pumpkin_inventory::Inventory,
    ) -> (ItemStack, ItemStack) {
        loop {
            if self
                .entity
                .removed
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return (ItemStack::EMPTY.clone(), ItemStack::EMPTY.clone());
            }
            let original = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if original.is_empty() {
                return (original, ItemStack::EMPTY.clone());
            }
            let amount = destination.space_for_stack(&original);
            if amount == 0 {
                return (original, ItemStack::EMPTY.clone());
            }
            let mut stack = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self
                .entity
                .removed
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return (ItemStack::EMPTY.clone(), ItemStack::EMPTY.clone());
            }
            if stack.uid != original.uid || !stack.are_equal(&original) {
                continue;
            }
            let taken = if amount >= stack.item_count {
                std::mem::replace(&mut *stack, ItemStack::EMPTY.clone())
            } else {
                stack.split(amount)
            };
            return (original, taken);
        }
    }

    pub(super) fn stack_snapshot(&self) -> ItemStack {
        let owner = self.transaction.clone();
        let _transfer = owner.lock();
        self.item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn stack_is_empty_between_transfers(&self) -> bool {
        let owner = self.transaction.clone();
        let _transfer = owner.lock();
        self.item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    /// Publishes the committed count after transfer ownership and storage guards are released.
    pub(crate) fn finish_transfer(&self, empty: bool) {
        if empty {
            self.entity.remove();
        } else {
            self.on_count_changed();
        }
    }
}
