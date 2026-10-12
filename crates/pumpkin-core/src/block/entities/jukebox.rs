use pumpkin_data::{Block, block_properties::JukeboxLikeProperties, game_event::GameEvent};
use pumpkin_world::world::BlockFlags;
use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;

use crate::block::entities::BlockEntity;
use crate::world::World;
use pumpkin_inventory::{Clearable, Inventory};

/// Matches vanilla's `JukeboxBlockEntity`
pub struct JukeboxBlockEntity {
    position: BlockPos,
    world: Mutex<Weak<World>>,
    pub(crate) transaction:
        std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    /// The record item stored in the jukebox (`RecordItem` in NBT)
    record_stack: Arc<Mutex<ItemStack>>,
    /// Ticks since the current song started playing
    ticks_since_song_started: AtomicU64,
    /// Length of the current song in ticks (0 if not playing)
    song_length_ticks: AtomicU64,
    dirty: AtomicBool,
    comparator_dirty: AtomicBool,
}

const RECORD_ITEM_NBT_KEY: &str = "RecordItem";
const TICKS_SINCE_SONG_STARTED_NBT_KEY: &str = "ticks_since_song_started";

impl BlockEntity for JukeboxBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let record_stack = nbt
            .get_compound(RECORD_ITEM_NBT_KEY)
            .and_then(ItemStack::read_item_stack)
            .unwrap_or_else(|| ItemStack::EMPTY.clone());

        let ticks_since_song_started =
            nbt.get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY).unwrap_or(0) as u64;

        Self {
            position,
            world: Mutex::new(Weak::new()),
            transaction: std::sync::Arc::default(),
            record_stack: Arc::new(Mutex::new(record_stack)),
            ticks_since_song_started: AtomicU64::new(ticks_since_song_started),
            song_length_ticks: AtomicU64::new(0), // Will be set when playing starts
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        let record = pumpkin_inventory::inventory::slot_transaction::snapshot_slot(self, 0);
        if !record.is_empty() {
            let mut record_nbt = NbtCompound::new();
            record.write_item_stack(&mut record_nbt);
            nbt.put(RECORD_ITEM_NBT_KEY, record_nbt);
        }

        let ticks = self.ticks_since_song_started.load(Ordering::Relaxed);
        if ticks > 0 {
            nbt.put_long(TICKS_SINCE_SONG_STARTED_NBT_KEY, ticks as i64);
        }
    }

    fn tick(&self, world: &Arc<World>) {
        self.bind_world(world);
        let _guard = self.transaction.lock();
        if !self.transaction.is_live() {
            return;
        }
        // Increment ticks if we're playing
        let song_length = self.song_length_ticks.load(Ordering::Relaxed);
        if song_length > 0 {
            let ticks = self
                .ticks_since_song_started
                .fetch_add(1, Ordering::Relaxed);
            // Check if song has finished
            if ticks >= song_length {
                self.stop_playing();
                // JukeboxSongPlayer.tick leaves the record when playback ends.
            }
        }
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

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        let record = pumpkin_inventory::inventory::slot_transaction::snapshot_slot(self, 0);
        if !record.is_empty() {
            let mut record_nbt = NbtCompound::new();
            record.write_item_stack(&mut record_nbt);
            nbt.put("RecordItem", NbtTag::Compound(record_nbt));
        }
        nbt.put_long(
            "ticks_since_song_started",
            self.ticks_since_song_started.load(Ordering::Relaxed) as i64,
        );
        Some(nbt)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self)
    }
}

impl JukeboxBlockEntity {
    pub(crate) fn bind_world(&self, world: &Arc<World>) {
        *self
            .world
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::downgrade(world);
    }

    // JukeboxBlockEntity.setTheItem / notifyItemChangedInJukebox.
    fn replace_record(&self, stack: ItemStack) -> ItemStack {
        let mut previous = ItemStack::EMPTY.clone();
        let mut replacement = Some(stack);
        self.update_slot(0, &mut |record| {
            if let Some(stack) = replacement.take() {
                previous = std::mem::replace(record, stack);
            }
        });
        let current = self.get_record();
        let changed = !previous.are_equal(&current);
        let has_record = !current.is_empty();
        self.mark_dirty();
        if changed {
            self.notify_item_changed_in_jukebox(has_record);
        }
        previous
    }

    fn notify_item_changed_in_jukebox(&self, mut has_record: bool) {
        if !self.transaction.is_live() {
            return;
        }
        let world = self
            .world
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .upgrade();
        let Some(world) = world else {
            return;
        };
        if world.get_block(&self.position) != &Block::JUKEBOX
            || world.get_block_entity(&self.position).is_none_or(|entity| {
                entity
                    .as_any()
                    .downcast_ref::<Self>()
                    .is_none_or(|current| !std::ptr::eq(current, self))
            })
        {
            return;
        }
        loop {
            if world
                .set_block_state_if(
                    &self.position,
                    JukeboxLikeProperties { has_record }.to_state_id(&Block::JUKEBOX),
                    BlockFlags::NOTIFY_LISTENERS,
                    |state| {
                        self.transaction.is_live() && Block::from_state_id(state) == &Block::JUKEBOX
                    },
                )
                .is_none()
            {
                return;
            }
            // Deferred notifications may arrive in a different order than their slot commits.
            let current = !self.get_record().is_empty();
            if current == has_record {
                break;
            }
            has_record = current;
        }
        if !self.transaction.is_live() {
            return;
        }
        world.emit_game_event(
            GameEvent::BlockChange.name(),
            self.position.to_centered_f64(),
        );
    }

    pub const ID: &'static str = "minecraft:jukebox";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            world: Mutex::new(Weak::new()),
            transaction: std::sync::Arc::default(),
            record_stack: Arc::new(Mutex::new(ItemStack::EMPTY.clone())),
            ticks_since_song_started: AtomicU64::new(0),
            song_length_ticks: AtomicU64::new(0),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }

    /// Get the current record stack
    pub fn get_record(&self) -> ItemStack {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the record and notifies its world when the stored item changes.
    pub fn set_record(&self, stack: ItemStack) {
        self.replace_record(stack);
    }

    /// Clear the stack and return what was there - used for dropping
    pub fn clear_record(&self) -> ItemStack {
        self.replace_record(ItemStack::EMPTY.clone())
    }

    /// Start playing a song with the given length in ticks
    pub fn start_playing(&self, length_in_ticks: u64) {
        let guard = self.transaction.lock();
        if !self.transaction.is_live() || self.get_record().is_empty() {
            return;
        }
        self.ticks_since_song_started.store(0, Ordering::Relaxed);
        self.song_length_ticks
            .store(length_in_ticks, Ordering::Relaxed);
        drop(guard);
        self.mark_dirty();
    }

    /// Stop playing the current song
    pub fn stop_playing(&self) {
        let guard = self.transaction.lock();
        self.ticks_since_song_started.store(0, Ordering::Relaxed);
        self.song_length_ticks.store(0, Ordering::Relaxed);
        drop(guard);
        self.mark_dirty();
    }

    /// Check if a song is currently playing
    pub fn is_playing(&self) -> bool {
        let song_length = self.song_length_ticks.load(Ordering::Relaxed);
        if song_length == 0 {
            return false;
        }
        let ticks = self.ticks_since_song_started.load(Ordering::Relaxed);
        ticks < song_length
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }
}

/// Implements single-slot inventory for jukebox (matches vanilla's `SingleStackInventory`)
impl Inventory for JukeboxBlockEntity {
    fn slot_transaction(
        &self,
        _slot: usize,
    ) -> std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        if slot == 0 {
            let mut record = self
                .record_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            update(&mut record);
            if record.is_empty() {
                // JukeboxSongPlayer.onItemChanged stops playback with the committed removal.
                self.ticks_since_song_started.store(0, Ordering::Relaxed);
                self.song_length_ticks.store(0, Ordering::Relaxed);
            }
        }
    }

    fn on_slot_changed(&self, _slot: usize) {
        // JukeboxBlockEntity.notifyItemChangedInJukebox runs after the committed mutation.
        self.mark_dirty();
        let has_record = !self.get_record().is_empty();
        self.notify_item_changed_in_jukebox(has_record);
    }

    fn size(&self) -> usize {
        1
    }

    fn is_empty(&self) -> bool {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    fn get_stack(&self, _slot: usize) -> ItemStack {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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

    fn remove_stack_specific(&self, _slot: usize, _amount: u8) -> ItemStack {
        // JukeboxBlockEntity.splitTheItem ignores the requested amount.
        self.remove_stack(0)
    }

    fn set_stack(&self, _slot: usize, stack: ItemStack) {
        self.replace_record(stack);
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Clearable for JukeboxBlockEntity {
    fn clear(&self) {
        pumpkin_inventory::inventory::slot_transaction::clear(self);
    }
}

#[cfg(test)]
#[path = "jukebox_event_tests.rs"]
mod tests;
