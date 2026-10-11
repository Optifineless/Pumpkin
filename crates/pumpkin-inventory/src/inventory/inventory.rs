use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use std::any::Any;
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

pub trait Inventory: Send + Sync + Clearable {
    /// Mutates a valid slot once under its storage lock, without dispatching side effects.
    /// The callback only mutates the stack; it must not call inventories, events or networking.
    /// Call `on_slot_changed` after releasing transfer ownership when the stack changed.
    fn update_slot(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack));

    /// Returns ownership of the backing inventory; double chests route slots to their halves.
    fn slot_transaction(&self, _slot: usize) -> Arc<std::sync::Mutex<()>> {
        super::slot_transaction::owner(self.as_any())
    }

    /// Dispatches effects of a committed slot update after storage and transfer locks are gone.
    fn on_slot_changed(&self, _slot: usize) {
        self.mark_dirty();
    }

    fn size(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn get_stack(&self, slot: usize) -> ItemStack;

    fn remove_stack(&self, slot: usize) -> ItemStack;

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack;

    fn set_stack(&self, slot: usize, stack: ItemStack);

    /// Computes insertion capacity while transfer ownership is held, without mutating storage.
    fn space_for_stack(&self, stack: &ItemStack) -> u8 {
        let max = self
            .get_max_count_per_stack()
            .min(stack.get_max_stack_size());
        let mut space = 0u8;
        for slot in 0..self.size() {
            if !self.is_valid_slot_for(slot, stack) {
                continue;
            }
            let current = self.get_stack(slot);
            let available = if current.is_empty() {
                max
            } else if current.are_items_and_components_equal(stack) {
                max.saturating_sub(current.item_count)
            } else {
                0
            };
            space = space.saturating_add(available).min(stack.item_count);
            if space == stack.item_count {
                break;
            }
        }
        space
    }

    fn on_open(&self) {}
    fn on_close(&self) {}

    fn count(&self, item: &Item) -> u8 {
        let mut count = 0;

        for i in 0..self.size() {
            let stack = self.get_stack(i);
            if stack.get_item().id == item.id {
                count += stack.item_count;
            }
        }

        count
    }

    fn contains_any_predicate(&self, predicate: &(dyn Fn(&ItemStack) -> bool + Sync)) -> bool {
        for i in 0..self.size() {
            let stack = self.get_stack(i);
            if predicate(&stack) {
                return true;
            }
        }

        false
    }

    fn contains_any(&self, items: &[Item]) -> bool {
        self.contains_any_predicate(&|stack| !stack.is_empty() && items.contains(stack.get_item()))
    }

    fn write_inventory_nbt(&self, nbt: &mut NbtCompound, include_empty: bool) {
        let mut slots = Vec::new();
        let size = self.size();
        let owners = super::slot_transaction::ordered_owners(&[self]);
        let transfers = owners
            .iter()
            .map(|owner| {
                owner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            })
            .collect::<Vec<_>>();
        let snapshot = (0..size).map(|i| self.get_stack(i)).collect::<Vec<_>>();
        drop(transfers);
        for (i, stack) in snapshot.into_iter().enumerate() {
            if !stack.is_empty() {
                let mut item_compound = NbtCompound::new();
                item_compound.put_byte("Slot", i as i8);
                stack.write_item_stack(&mut item_compound);
                slots.push(NbtTag::Compound(item_compound));
            }
        }

        if include_empty || !slots.is_empty() {
            nbt.put("Items", NbtTag::List(slots));
        }
    }

    fn get_max_count_per_stack(&self) -> u8 {
        99
    }

    fn mark_dirty(&self) {}

    fn read_data(&self, nbt: &NbtCompound, stacks: &mut [ItemStack]) {
        sync_read_items_from_nbt(nbt, stacks);
    }

    /// Checks item eligibility without slot occupancy, for menu insertion and swapping.
    fn can_place_item(&self, slot: usize, stack: &ItemStack) -> bool {
        self.is_valid_slot_for(slot, stack)
    }

    fn is_valid_slot_for(&self, _slot: usize, _stack: &ItemStack) -> bool {
        true
    }

    fn can_transfer_to(
        &self,
        _hopper_inventory: &dyn Inventory,
        _slot: usize,
        _stack: &ItemStack,
    ) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any;
}

pub trait Clearable {
    fn clear(&self);
}

pub fn sync_read_items_from_nbt(nbt: &NbtCompound, stacks: &mut [ItemStack]) {
    if let Some(inventory_list) = nbt.get_list("Items") {
        for tag in inventory_list {
            if let Some(item_compound) = tag.extract_compound()
                && let Some(slot_byte) = item_compound.get_byte("Slot")
            {
                let slot = slot_byte as usize;
                if slot < stacks.len()
                    && let Some(item_stack) = ItemStack::read_item_stack(item_compound)
                {
                    stacks[slot] = item_stack;
                }
            }
        }
    }
}

pub fn sync_write_items_to_nbt(items: &[ItemStack], nbt: &mut NbtCompound) {
    let mut slots = Vec::new();
    for (i, stack) in items.iter().enumerate() {
        if !stack.is_empty() {
            let mut item_nbt = NbtCompound::new();
            item_nbt.put_byte("Slot", i as i8);
            stack.write_item_stack(&mut item_nbt);
            slots.push(NbtTag::Compound(item_nbt));
        }
    }
    if !slots.is_empty() {
        nbt.put_list("Items", slots);
    }
}

pub struct ComparableInventory(pub Arc<dyn Inventory>);

impl PartialEq for ComparableInventory {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ComparableInventory {}

impl Hash for ComparableInventory {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // AbstractContainerMenu.transferState keys by container identity, not dyn vtable.
        Arc::as_ptr(&self.0).cast::<()>().hash(state);
    }
}
