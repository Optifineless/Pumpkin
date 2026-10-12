use crate::inventory::{Clearable, Inventory};
use pumpkin_data::item_stack::ItemStack;
use std::any::Any;
use std::sync::RwLock;

pub struct SimpleInventory {
    pub(crate) transaction: std::sync::Arc<crate::inventory::slot_transaction::Transaction>,
    pub stacks: RwLock<Vec<ItemStack>>,
    size: usize,
}

impl SimpleInventory {
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            transaction: std::sync::Arc::default(),
            stacks: RwLock::new(vec![ItemStack::EMPTY.clone(); size]),
            size,
        }
    }
}

impl Clearable for SimpleInventory {
    fn clear(&self) {
        crate::inventory::slot_transaction::clear(self);
    }
}

impl Inventory for SimpleInventory {
    fn slot_transaction(
        &self,
        _slot: usize,
    ) -> std::sync::Arc<crate::inventory::slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        let mut items = self
            .stacks
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stack) = items.get_mut(slot) else {
            return;
        };
        update(stack);
    }

    fn size(&self) -> usize {
        self.size
    }

    fn is_empty(&self) -> bool {
        let stacks = self
            .stacks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        stacks.iter().all(ItemStack::is_empty)
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        let stacks = self
            .stacks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        stacks
            .get(slot)
            .cloned()
            .unwrap_or_else(|| ItemStack::EMPTY.clone())
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            removed = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });

        removed
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            if !stack.is_empty() && amount > 0 {
                removed = stack.split(amount);
            }
        });

        removed
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        let mut replacement = Some(stack);
        self.update_slot(slot, &mut |current| {
            if let Some(stack) = replacement.take() {
                *current = stack;
            }
        });
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
