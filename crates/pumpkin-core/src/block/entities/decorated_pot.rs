use super::BlockEntity;
use pumpkin_data::data_component_impl::{
    ContainerImpl, DataComponentImpl, PotDecoration, PotDecorationsImpl,
};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::{Clearable, Inventory, inventory::slot_transaction};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::world::World;

enum PotDecorationsLockClass {}

impl pumpkin_util::sync::LockClass for PotDecorationsLockClass {
    const NAME: &'static str = "decorated pot decorations";
    const LEVEL: u8 = 40;
}

type PotDecorationsLock<T> = pumpkin_util::sync::Mutex<T, PotDecorationsLockClass>;

/// Compatibility for fork-era block entities that predate stored face templates.
pub(crate) fn legacy_brick_decorations() -> PotDecorationsImpl {
    let brick = PotDecoration {
        item: &Item::BRICK,
        count: 1,
        patch: Vec::new(),
    };
    PotDecorationsImpl {
        decorations: std::array::from_fn(|_| Some(brick.clone())),
    }
}

pub struct DecoratedPotBlockEntity {
    pub position: BlockPos,
    pub sherds: Mutex<Option<NbtTag>>,
    pub item: Mutex<Option<ItemStack>>,
    decorations: PotDecorationsLock<Option<PotDecorationsImpl>>,
    transaction: Arc<slot_transaction::Transaction>,
    dirty: AtomicBool,
    comparator_dirty: AtomicBool,
}

impl BlockEntity for DecoratedPotBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn collect_item_components(&self, stack: &mut ItemStack) {
        let decorations = self
            .decorations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned()
            // Old fork saves had no face data and historically returned four bricks.
            // Explicitly stored empty decorations remain empty.
            .unwrap_or_else(legacy_brick_decorations);
        stack.set_data_component(decorations);
        if let Some(item) = self.get_item().filter(|item| !item.is_empty()) {
            stack.set_data_component(ContainerImpl {
                items: vec![(0, item)],
            });
        }
    }

    fn apply_item_components(&self, stack: &ItemStack) {
        let decorations = stack.get_data_component::<PotDecorationsImpl>().cloned();
        *self
            .decorations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = decorations;

        let item = stack
            .get_data_component::<ContainerImpl>()
            .and_then(|container| {
                container
                    .items
                    .iter()
                    .find(|(slot, item)| *slot == 0 && !item.is_empty())
                    .map(|(_, item)| item.clone())
            });
        *self
            .item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = item;
        self.on_slot_changed(0);
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn on_block_replaced(self: Arc<Self>, world: &Arc<World>, position: &BlockPos) {
        // DecoratedPotBlockEntity.preRemoveSideEffects / Containers.dropContents.
        world.scatter_inventory(position, &(self as Arc<dyn Inventory>));
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let sherds = nbt.get("sherds").cloned();
        let decorations = nbt
            .get("PotDecorations")
            .map(|data| PotDecorationsImpl::read_data(data).unwrap_or(PotDecorationsImpl::EMPTY));
        let decorations = decorations.or_else(|| {
            sherds.as_ref().map(|saved| {
                PotDecorationsImpl::read_data(saved)
                    .or_else(|| match saved {
                        NbtTag::List(legacy) => PotDecorationsImpl::from_legacy_sherds(legacy),
                        _ => None,
                    })
                    // A present but unreadable field must not be treated as missing legacy data.
                    .unwrap_or(PotDecorationsImpl::EMPTY)
            })
        });
        let item = nbt
            .get_compound("item")
            .and_then(ItemStack::read_item_stack);
        Self {
            position,
            sherds: Mutex::new(sherds),
            item: Mutex::new(item),
            decorations: PotDecorationsLock::new(decorations),
            transaction: Arc::default(),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Ok(sherds) = self.sherds.lock()
            && let Some(sh) = sherds.as_ref()
        {
            nbt.put("sherds", sh.clone());
        }
        if let Some(decorations) = self
            .decorations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            nbt.put("PotDecorations", decorations.write_data());
        }
        // DecoratedPotBlockEntity.saveAdditional keeps the single-item tag format.
        let item = slot_transaction::snapshot_slot(self, 0);
        if !item.is_empty() {
            let mut item_nbt = NbtCompound::new();
            item.write_item_stack(&mut item_nbt);
            nbt.put_compound("item", item_nbt);
        }
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_nbt(&mut nbt);
        Some(nbt)
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn is_comparator_dirty(&self) -> bool {
        self.comparator_dirty.load(Ordering::Relaxed)
    }

    fn clear_comparator_dirty(&self) {
        self.comparator_dirty.store(false, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl DecoratedPotBlockEntity {
    pub const ID: &'static str = "minecraft:decorated_pot";

    pub(crate) fn has_empty_decorations(&self) -> bool {
        self.decorations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|decorations| decorations.decorations.iter().all(Option::is_none))
    }

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            sherds: Mutex::new(None),
            item: Mutex::new(None),
            decorations: PotDecorationsLock::new(None),
            transaction: Arc::default(),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }

    pub fn get_item(&self) -> Option<ItemStack> {
        self.item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn take_item(&self) -> Option<ItemStack> {
        let stack = self.remove_stack(0);
        (!stack.is_empty()).then_some(stack)
    }

    /// Deposits only owned stock and keeps the unaccepted receipt in `stack`.
    pub fn try_insert_item(&self, stack: &mut ItemStack, count: u8) -> bool {
        let mut receipt = stack.split(count.min(stack.item_count));
        let before = receipt.item_count;
        let maximum = receipt.get_max_stack_size();
        slot_transaction::deposit(self, 0, &mut receipt, maximum);
        stack.increment(receipt.item_count);
        let moved = before > receipt.item_count;
        if moved {
            self.on_slot_changed(0);
        }
        moved
    }

    pub fn get_comparator_output(&self) -> u8 {
        self.item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map_or(0, |item| {
                if item.item_count == 0 {
                    0
                } else {
                    // Vanilla scales by the item's own stack size, not a fixed 64.
                    // `try_insert_item` may hold more than one stack, so cap the ratio if full.
                    let max_count = f32::from(item.get_max_stack_size());
                    let filled = (f32::from(item.item_count) / max_count).min(1.0);
                    1 + (filled * 14.0).floor() as u8
                }
            })
    }
}

// DecoratedPotBlockEntity.getTheItem/splitTheItem/setTheItem and ContainerSingleItem.
impl Inventory for DecoratedPotBlockEntity {
    fn slot_transaction(&self, _slot: usize) -> Arc<slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        if slot != 0 {
            return;
        }
        let mut item = self
            .item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut stack = item.take().unwrap_or_else(|| ItemStack::EMPTY.clone());
        update(&mut stack);
        *item = (!stack.is_empty()).then_some(stack);
    }

    fn size(&self) -> usize {
        1
    }

    fn is_empty(&self) -> bool {
        self.get_stack(0).is_empty()
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        if slot == 0 {
            self.get_item().unwrap_or_else(|| ItemStack::EMPTY.clone())
        } else {
            ItemStack::EMPTY.clone()
        }
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut taken = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            taken = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });
        if !taken.is_empty() {
            self.on_slot_changed(slot);
        }
        taken
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut taken = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            taken = stack.split(amount.min(stack.item_count));
        });
        if !taken.is_empty() {
            self.on_slot_changed(slot);
        }
        taken
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

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Clearable for DecoratedPotBlockEntity {
    fn clear(&self) {
        slot_transaction::clear(self);
    }
}
