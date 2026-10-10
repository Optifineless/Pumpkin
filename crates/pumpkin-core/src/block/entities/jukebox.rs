use pumpkin_data::{
    Block, block_properties::JukeboxLikeProperties, data_component_impl::JukeboxPlayableImpl,
    game_event::GameEvent, jukebox_song::JukeboxSong, world::WorldEvent,
};
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
    /// The record item stored in the jukebox (`RecordItem` in NBT)
    record_stack: Arc<Mutex<ItemStack>>,
    /// Ticks since the current song started playing
    ticks_since_song_started: AtomicU64,
    /// Length of the current song in ticks (0 if not playing)
    song_length_ticks: AtomicU64,
    dirty: AtomicBool,
    comparator_dirty: AtomicBool,
    record_revision: AtomicU64,
    #[cfg(test)]
    notification_pause: Mutex<Option<NotificationPause>>,
}

#[cfg(test)]
type NotificationPause = Arc<dyn Fn() + Send + Sync>;

const RECORD_ITEM_NBT_KEY: &str = "RecordItem";
const TICKS_SINCE_SONG_STARTED_NBT_KEY: &str = "ticks_since_song_started";

/// Resolves a nonempty record's playable component through the generated song registry.
pub(crate) fn song_from_stack(stack: &ItemStack) -> Option<JukeboxSong> {
    // JukeboxSong.fromStack / ItemStack.getComponents hide components on empty stacks.
    if stack.is_empty() {
        return None;
    }
    let playable = stack.get_data_component::<JukeboxPlayableImpl>()?;
    JukeboxSong::from_name(playable.song.rsplit(':').next()?)
}

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
            record_stack: Arc::new(Mutex::new(record_stack)),
            ticks_since_song_started: AtomicU64::new(ticks_since_song_started),
            song_length_ticks: AtomicU64::new(0), // Will be set when playing starts
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
            record_revision: AtomicU64::new(0),
            #[cfg(test)]
            notification_pause: Mutex::new(None),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        let record = self
            .record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        if let Ok(record) = self.record_stack.try_lock()
            && !record.is_empty()
        {
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
    fn replace_record(&self, mut stack: ItemStack) -> ItemStack {
        self.update_record(&mut |record| std::mem::swap(record, &mut stack))
    }

    fn update_record(&self, update: &mut dyn FnMut(&mut ItemStack)) -> ItemStack {
        let (previous, changed, has_record, revision, song) = {
            let mut record = self
                .record_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = record.clone();
            update(&mut record);
            let changed = !previous.are_equal(&record);
            let revision = if changed {
                self.record_revision.fetch_add(1, Ordering::Relaxed) + 1
            } else {
                self.record_revision.load(Ordering::Relaxed)
            };
            let song = if changed {
                song_from_stack(&record)
            } else {
                None
            };
            (previous, changed, !record.is_empty(), revision, song)
        };
        if changed {
            self.mark_dirty();
            let world = self.notify_item_changed_in_jukebox(has_record, revision);
            // JukeboxBlockEntity.setTheItem notifies the item change before play/stop.
            self.update_playback(song, revision, world.as_ref());
        }
        previous
    }

    fn update_playback(
        &self,
        song: Option<JukeboxSong>,
        revision: u64,
        world: Option<&Arc<World>>,
    ) {
        let song_changed = {
            let _record = self
                .record_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.record_revision.load(Ordering::Relaxed) != revision {
                return;
            }
            song.map_or_else(
                || {
                    let was_playing = self.song_length_ticks.load(Ordering::Relaxed) > 0;
                    self.stop_playing();
                    was_playing
                },
                |song| {
                    self.start_playing(song.length_in_ticks());
                    true
                },
            )
        };
        let Some(world) = world else {
            return;
        };
        if !song_changed || self.record_revision.load(Ordering::Relaxed) != revision {
            return;
        }
        // JukeboxSongPlayer.play/stop -> JukeboxBlockEntity.onSongChanged.
        if let Some(song) = song {
            world.sync_world_event(
                WorldEvent::SoundPlayJukeboxSong,
                self.position,
                song.get_id() as i32,
            );
        } else {
            world.emit_game_event(
                GameEvent::JukeboxStopPlay.name(),
                self.position.to_centered_f64(),
            );
            if self.record_revision.load(Ordering::Relaxed) != revision {
                return;
            }
            world.sync_world_event(WorldEvent::SoundStopJukeboxSong, self.position, 0);
        }
        world.update_neighbors_at(&self.position, &Block::JUKEBOX, None);
    }

    fn notify_item_changed_in_jukebox(
        &self,
        has_record: bool,
        revision: u64,
    ) -> Option<Arc<World>> {
        let world = self
            .world
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .upgrade();
        let world = world?;
        let expected = world.get_block_state_id(&self.position);
        if Block::from_state_id(expected) != &Block::JUKEBOX {
            return None;
        }
        #[cfg(test)]
        {
            let pause = self.notification_pause.lock().unwrap().clone();
            if let Some(pause) = pause {
                pause();
            }
        }
        // JukeboxBlockEntity.notifyItemChangedInJukebox only updates a still-placed entity.
        // Check the live state and record generation
        // inside the conditional chunk write so removal and newer records take precedence.
        let applied = world.set_block_state_if(
            &self.position,
            JukeboxLikeProperties { has_record }.to_state_id(&Block::JUKEBOX),
            BlockFlags::NOTIFY_LISTENERS,
            |state| {
                state == expected
                    && self.record_revision.load(Ordering::Relaxed) == revision
                    && world
                        .block_entities
                        .get(&self.position.chunk_position())
                        .is_some_and(|entities| {
                            entities.get(&self.position).is_some_and(|entity| {
                                entity
                                    .as_any()
                                    .downcast_ref::<Self>()
                                    .is_some_and(|placed| std::ptr::eq(placed, self))
                            })
                        })
            },
        );
        if applied.is_some() && self.record_revision.load(Ordering::Relaxed) == revision {
            world.emit_game_event(
                GameEvent::BlockChange.name(),
                self.position.to_centered_f64(),
            );
            return Some(world);
        }
        None
    }

    pub const ID: &'static str = "minecraft:jukebox";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            world: Mutex::new(Weak::new()),
            record_stack: Arc::new(Mutex::new(ItemStack::EMPTY.clone())),
            ticks_since_song_started: AtomicU64::new(0),
            song_length_ticks: AtomicU64::new(0),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
            record_revision: AtomicU64::new(0),
            #[cfg(test)]
            notification_pause: Mutex::new(None),
        }
    }

    /// Get the current record stack
    pub fn get_record(&self) -> ItemStack {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the record, updates playback and notifies its world when the stored item changes.
    pub fn set_record(&self, stack: ItemStack) {
        self.replace_record(stack);
    }

    /// Clear the stack and return what was there - used for dropping
    pub fn clear_record(&self) -> ItemStack {
        let previous = self.replace_record(ItemStack::EMPTY.clone());
        if previous.is_empty() {
            let record = self
                .record_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // JukeboxBlockEntity.setTheItem(EMPTY) also stops an already-empty jukebox.
            if record.is_empty() {
                self.stop_playing();
            }
        }
        previous
    }

    /// Start playing a song with the given length in ticks
    pub fn start_playing(&self, length_in_ticks: u64) {
        self.ticks_since_song_started.store(0, Ordering::Relaxed);
        self.song_length_ticks
            .store(length_in_ticks, Ordering::Relaxed);
        self.mark_dirty();
    }

    /// Stop playing the current song
    pub fn stop_playing(&self) {
        self.ticks_since_song_started.store(0, Ordering::Relaxed);
        self.song_length_ticks.store(0, Ordering::Relaxed);
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
    fn update_slot(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        if slot == 0 {
            self.update_record(update);
        }
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

    fn remove_stack(&self, _slot: usize) -> ItemStack {
        self.clear_record()
    }

    fn remove_stack_specific(&self, _slot: usize, _amount: u8) -> ItemStack {
        // Jukebox only holds one item, so remove the whole stack
        self.remove_stack(0)
    }

    fn set_stack(&self, _slot: usize, stack: ItemStack) {
        self.set_record(stack);
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
        self.clear_record();
    }
}

#[cfg(test)]
#[path = "jukebox_event_tests.rs"]
mod tests;
