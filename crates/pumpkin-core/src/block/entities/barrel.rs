use pumpkin_data::block_properties::BarrelLikeProperties;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, FacingExt, item_stack::ItemStack};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::xoroshiro128::Xoroshiro;
use pumpkin_util::random::{RandomImpl, get_seed};
use std::any::Any;
use std::sync::RwLock;
use std::{
    array::from_fn,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::block::viewer::{ViewerCountListener, ViewerCountTracker, ViewerCountTrackerExt};
use crate::world::{BlockFlags, World};
use pumpkin_inventory::{Clearable, Inventory};

use super::BlockEntity;

pub struct BarrelBlockEntity {
    pub position: BlockPos,
    pub lock: super::container_lock::ContainerLock,
    pub(crate) transaction:
        std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    pub items: RwLock<[ItemStack; Self::INVENTORY_SIZE]>,
    pub dirty: AtomicBool,
    pub comparator_dirty: AtomicBool,

    // Viewer
    viewers: ViewerCountTracker,
}

impl BlockEntity for BarrelBlockEntity {
    fn collect_item_components(&self, stack: &mut ItemStack) {
        self.lock.collect(stack);
    }
    fn apply_item_components(&self, stack: &ItemStack) {
        self.lock.apply(stack);
    }
    fn container_lock(&self) -> Option<pumpkin_data::data_component_impl::LockImpl> {
        self.lock.get()
    }
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let mut barrel = Self {
            position,
            lock: super::container_lock::ContainerLock::from_nbt(nbt),
            transaction: std::sync::Arc::default(),
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
            viewers: ViewerCountTracker::new(),
        };

        pumpkin_inventory::sync_read_items_from_nbt(
            nbt,
            barrel
                .items
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );

        barrel
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.lock.write_nbt(nbt);
        self.write_inventory_nbt(nbt, true);
    }

    fn tick(&self, world: &Arc<World>) {
        self.viewers
            .update_viewer_count::<Self>(self, world, &self.position);
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
        Some(nbt)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ViewerCountListener for BarrelBlockEntity {
    fn on_container_open(&self, world: &Arc<World>, position: &BlockPos) {
        self.play_sound(world, Sound::BlockBarrelOpen);
        self.set_open(world, true);
        world.emit_game_event(GameEvent::ContainerOpen.name(), position.to_centered_f64());
    }

    fn on_container_close(&self, world: &Arc<World>, position: &BlockPos) {
        self.play_sound(world, Sound::BlockBarrelClose);
        self.set_open(world, false);
        world.emit_game_event(GameEvent::ContainerClose.name(), position.to_centered_f64());
    }
}

impl BarrelBlockEntity {
    pub const INVENTORY_SIZE: usize = 27;
    pub const ID: &'static str = "minecraft:barrel";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            lock: super::container_lock::ContainerLock::default(),
            transaction: std::sync::Arc::default(),
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
            viewers: ViewerCountTracker::new(),
        }
    }

    fn set_open(&self, world: &Arc<World>, open: bool) {
        let state = world.get_block_state(&self.position);
        let mut properties = BarrelLikeProperties::from_state_id(state.id);

        properties.open = open;

        world.set_block_state(
            &self.position,
            properties.to_state_id(&Block::BARREL),
            BlockFlags::NOTIFY_ALL,
        );
    }

    fn play_sound(&self, world: &Arc<World>, sound: Sound) {
        let mut rng = Xoroshiro::from_seed(get_seed());

        let state = world.get_block_state(&self.position);
        let properties = BarrelLikeProperties::from_state_id(state.id);
        let direction = properties.facing.to_block_direction().to_offset();
        let position = Vector3::new(
            self.position.0.x as f64 + 0.5 + direction.x as f64 / 2.0,
            self.position.0.y as f64 + 0.5 + direction.y as f64 / 2.0,
            self.position.0.z as f64 + 0.5 + direction.z as f64 / 2.0,
        );
        world.play_sound_fine(
            sound,
            SoundCategory::Blocks,
            &position,
            0.5,
            rng.next_f32() * 0.1 + 0.9,
        );
    }
}

impl Inventory for BarrelBlockEntity {
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

    fn on_open(&self) {
        self.viewers.open_container();
    }

    fn on_close(&self) {
        self.viewers.close_container();
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Clearable for BarrelBlockEntity {
    fn clear(&self) {
        pumpkin_inventory::inventory::slot_transaction::clear(self);
    }
}
