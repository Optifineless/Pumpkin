use crate::block::entities::BlockEntity;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{Clearable, Inventory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::any::Any;
use std::array::from_fn;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct ShelfBlockEntity {
    pub position: BlockPos,
    pub(crate) transaction:
        std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    pub items: RwLock<[ItemStack; Self::INVENTORY_SIZE]>,
    pub align_items_to_bottom: AtomicBool,
    pub dirty: AtomicBool,
    pub comparator_dirty: AtomicBool,
}

impl BlockEntity for ShelfBlockEntity {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_inventory_nbt(nbt, true);
        if self.align_items_to_bottom.load(Ordering::Relaxed) {
            nbt.put_bool("align_items_to_bottom", true);
        }
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let align_items_to_bottom = nbt.get_bool("align_items_to_bottom").unwrap_or(false);
        let mut shelf = Self {
            position,
            transaction: std::sync::Arc::default(),
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            align_items_to_bottom: AtomicBool::new(align_items_to_bottom),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        };

        pumpkin_inventory::sync_read_items_from_nbt(
            nbt,
            shelf
                .items
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );

        shelf
    }

    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self)
    }

    fn is_comparator_dirty(&self) -> bool {
        self.comparator_dirty.load(Ordering::Relaxed)
    }

    fn clear_comparator_dirty(&self) {
        self.comparator_dirty.store(false, Ordering::Relaxed);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_inventory_nbt(&mut nbt, false);
        if self.align_items_to_bottom.load(Ordering::Relaxed) {
            nbt.put_bool("align_items_to_bottom", true);
        }
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ShelfBlockEntity {
    pub const INVENTORY_SIZE: usize = 3;
    pub const ID: &'static str = "minecraft:shelf";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            transaction: std::sync::Arc::default(),
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            align_items_to_bottom: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }
}

impl Inventory for ShelfBlockEntity {
    fn slot_transaction(
        &self,
        _slot: usize,
    ) -> std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        let mut items = self
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stack) = items.get_mut(slot) else {
            return;
        };
        update(stack);
    }

    fn size(&self) -> usize {
        Self::INVENTORY_SIZE
    }

    fn is_empty(&self) -> bool {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.iter().all(ItemStack::is_empty)
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items[slot].clone()
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            removed = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });
        if !removed.is_empty() {
            self.on_slot_changed(slot);
        }
        removed
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            if !stack.is_empty() && amount > 0 {
                removed = stack.split(amount);
            }
        });
        if !removed.is_empty() {
            self.on_slot_changed(slot);
        }
        removed
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        let mut replacement = Some(stack);
        self.update_slot(slot, &mut |current| {
            if let Some(stack) = replacement.take() {
                *current = stack;
            }
        });
        self.on_slot_changed(slot);
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Clearable for ShelfBlockEntity {
    fn clear(&self) {
        pumpkin_inventory::inventory::slot_transaction::clear(self);
    }
}
