use std::sync::{Arc, Weak};

use pumpkin_data::BlockStateId;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_util::sync::{LockClass, MutexGuard, SnapshotBoundary as SB, assert_not_in_snapshot};
use pumpkin_world::chunk::{ChunkData, io::Dirtiable};
use rustc_hash::{FxHashMap, FxHashSet};

use super::portal::frame_effects::RetiredBlockEntity;
use super::{World, remove_pending_block_entity};
use crate::block::entities::{
    BlockEntity, block_entity_from_nbt, capture::capture_legacy, jukebox::JukeboxBlockEntity,
};

#[path = "block_entity_admission.rs"]
mod admission;

enum SnapshotPublication {
    Published,
    Replaced(Arc<dyn BlockEntity>),
    Missing,
    Older,
}

pub(super) enum ObservedBlockEntityInstallation {
    Rejected,
    Installed(Option<Arc<dyn BlockEntity>>),
}

// Admission level 10 precedes block-entity registry and chunk-leaf access; no callbacks run under it.
pub(super) enum PendingBlockEntityLoadsLock {}
impl LockClass for PendingBlockEntityLoadsLock {
    const NAME: &'static str = "block-entity mutation admission";
    const LEVEL: u8 = 10;
}

#[cfg(test)]
pub(super) type PublicationHook = Arc<dyn Fn() + Send + Sync>;

#[cfg(test)]
pub(super) type TransformWriteHook = Arc<dyn Fn(BlockPos) + Send + Sync>;

// Test hooks copy/take their callbacks before invocation; none retains its slot mutex.
#[cfg(test)]
#[derive(Default)]
pub struct PersistenceTestHooks {
    pub(crate) bed_half_write_test: std::sync::atomic::AtomicU8,
    pub(crate) entity_recapture:
        std::sync::Mutex<Option<Arc<super::entity_unload::RecaptureTestHook>>>,
    pub(crate) lazy: std::sync::Mutex<Option<PublicationHook>>,
    pub(crate) activation: std::sync::Mutex<Option<PublicationHook>>,
    pub(crate) registration: std::sync::Mutex<Option<PublicationHook>>,
    pub(crate) registration_published: std::sync::Mutex<Option<PublicationHook>>,
    pub(crate) transform_write: std::sync::Mutex<Option<TransformWriteHook>>,
    pub(crate) state_write: std::sync::Mutex<Option<TransformWriteHook>>,
    pub(crate) publication: std::sync::Mutex<Option<PublicationHook>>,
    pub(crate) update_capture: std::sync::Mutex<Option<PublicationHook>>,
    pub(crate) update_packet_capture: std::sync::Mutex<Option<PublicationHook>>,
}

struct BlockEntityUnload {
    captures: usize,
    capture_thread: std::thread::ThreadId,
    completed: bool,
    terrain: Weak<ChunkData>,
}

#[derive(Default)]
pub(super) struct PendingBlockEntityLoads {
    pub(super) snapshots: FxHashMap<BlockPos, Weak<()>>,
    bed_breaks: FxHashSet<BlockPos>,
    bed_uses: FxHashSet<BlockPos>,
    // The watermark is reset when the registered Arc changes, so stamps order one live incarnation.
    next_capture_stamp: u64,
    published_capture_stamps: FxHashMap<BlockPos, u64>,
    // Retirement closes mutation and pending-promotion admission before the final capture.
    unloading: FxHashMap<Vector2<i32>, BlockEntityUnload>,
    active_mutations: FxHashMap<Vector2<i32>, usize>,
    stop_admission: bool,
    pub(super) notifications:
        std::collections::VecDeque<super::block_entity_notifications::BlockEntityNotification>,
    pub(super) notifying: bool,
}

/// Reserves a synchronous world mutation so chunk retirement can defer to its next tick.
pub struct BlockEntityMutationPermit<'a> {
    world: &'a World,
    chunk_pos: Vector2<i32>,
}

impl Drop for BlockEntityMutationPermit<'_> {
    fn drop(&mut self) {
        let mut loads = self
            .world
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(active) = loads.active_mutations.get_mut(&self.chunk_pos) {
            *active -= 1;
            if *active == 0 {
                loads.active_mutations.remove(&self.chunk_pos);
            }
        }
    }
}

// A logical operation identity, with no lock or blocking permit retained across callbacks.
pub(super) struct BedBreakSnapshot<'a> {
    world: &'a World,
    position: BlockPos,
    expected: BlockStateId,
    chunk: Arc<ChunkData>,
}

impl BedBreakSnapshot<'_> {
    pub(super) fn chunk_is_current(&self) -> bool {
        self.world
            .level
            .loaded_chunks
            .get(&self.position.chunk_position())
            .is_some_and(|current| Arc::ptr_eq(&self.chunk, &current))
    }
}

impl Drop for BedBreakSnapshot<'_> {
    fn drop(&mut self) {
        self.world
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .bed_breaks
            .remove(&self.position);
    }
}

// Reserves both halves across unsafe-use break events without retaining the gate.
pub(super) struct BedUseReservation<'a> {
    world: &'a World,
    positions: [BlockPos; 2],
}

impl Drop for BedUseReservation<'_> {
    fn drop(&mut self) {
        let mut loads = self
            .world
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for position in self.positions {
            loads.bed_uses.remove(&position);
        }
    }
}

struct PendingBlockEntitySnapshot<'a> {
    world: &'a World,
    pos: BlockPos,
    chunk: Arc<ChunkData>,
    nbt: NbtCompound,
    token: Option<Arc<()>>,
}

impl Drop for PendingBlockEntitySnapshot<'_> {
    fn drop(&mut self) {
        let mut loads = self
            .world
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(token) = self.token.take() else {
            return;
        };
        if Arc::strong_count(&token) == 1
            && loads
                .snapshots
                .get(&self.pos)
                .is_some_and(|registered| Weak::ptr_eq(registered, &Arc::downgrade(&token)))
        {
            loads.snapshots.remove(&self.pos);
        }
        // Drop the strong token before releasing the gate, so simultaneous failed loads
        // cannot both miss last-owner cleanup.
        drop(token);
    }
}

impl World {
    #[cfg(test)]
    pub(super) fn before_state_write_test(&self, position: BlockPos) {
        let hook = self
            .persistence_test_hooks
            .state_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(hook) = hook {
            hook(position);
        }
    }

    /// Mirrors LevelChunk.getBlockEntityNbtForSaving: serialize the live instance and publish it only if current.
    pub(super) fn publish_block_entity_snapshot_if_current(
        &self,
        pos: BlockPos,
        expected: &Arc<dyn BlockEntity>,
        retiring: bool,
    ) {
        let mut expected = expected.clone();
        loop {
            let stamp = self.next_block_entity_capture_stamp();
            let nbt = self.capture_block_entity_snapshot(pos, &expected);
            if retiring {
                // Retirement excludes writers; release registry guards before terrain leaves.
                let chunk = self
                    .level
                    .loaded_chunks
                    .get(&pos.chunk_position())
                    .map(|entry| Arc::clone(entry.value()));
                if let Some(chunk) = chunk {
                    super::lectern_update::reconcile_retiring_lectern(&chunk, pos, &nbt);
                }
            }
            match self.try_publish_block_entity_snapshot(pos, &expected, &nbt, stamp, retiring) {
                SnapshotPublication::Published
                | SnapshotPublication::Missing
                | SnapshotPublication::Older => return,
                SnapshotPublication::Replaced(current) => {
                    expected = current;
                }
            }
        }
    }

    pub(super) fn capture_block_entity_snapshot(
        &self,
        pos: BlockPos,
        block_entity: &Arc<dyn BlockEntity>,
    ) -> NbtCompound {
        let mut nbt = capture_legacy(block_entity.as_ref()).into_nbt();
        let custom_data = self
            .custom_block_entity_data
            .get(&pos)
            .map(|custom_data| custom_data.clone());
        if let Some(custom_data) = custom_data.filter(|custom_data| !custom_data.is_empty()) {
            nbt.put_compound("PumpkinCustomData", custom_data);
        }
        nbt
    }

    fn try_publish_block_entity_snapshot(
        &self,
        pos: BlockPos,
        expected: &Arc<dyn BlockEntity>,
        nbt: &NbtCompound,
        stamp: u64,
        retiring: bool,
    ) -> SnapshotPublication {
        assert_not_in_snapshot(
            SB::WorldRead,
            "World::publish_block_entity_snapshot_if_current",
        );
        let chunk_pos = pos.chunk_position();
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !retiring && loads.unloading.contains_key(&chunk_pos) {
            return SnapshotPublication::Missing;
        }
        loads.snapshots.remove(&pos);

        let current = self
            .block_entities
            .get(&chunk_pos)
            .and_then(|entities| entities.get(&pos).cloned());
        match current {
            Some(current) if Arc::ptr_eq(&current, expected) => {}
            Some(current) => return SnapshotPublication::Replaced(current),
            None => return SnapshotPublication::Missing,
        }
        if loads
            .published_capture_stamps
            .get(&pos)
            .is_some_and(|published| stamp < *published)
        {
            return SnapshotPublication::Older;
        }

        let Some(chunk_entry) = self.level.loaded_chunks.get(&chunk_pos) else {
            return SnapshotPublication::Missing;
        };
        let chunk = chunk_entry.value().clone();
        chunk
            .pending_block_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(pos, nbt.clone());
        loads.published_capture_stamps.insert(pos, stamp);
        // Keep the chunk entry as the eviction fence until this image is dirty. The matching
        // vanilla eviction point is ChunkMap.scheduleUnload.
        chunk.mark_dirty(true);
        drop(chunk_entry);
        drop(loads);

        #[cfg(test)]
        {
            let hook = self
                .persistence_test_hooks
                .publication
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }

        self.pending_block_entity_migrations.push(chunk_pos);
        SnapshotPublication::Published
    }

    pub(super) fn next_block_entity_capture_stamp(&self) -> u64 {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loads.next_capture_stamp = loads.next_capture_stamp.saturating_add(1);
        loads.next_capture_stamp
    }

    pub(super) fn publish_block_entity_nbt(
        &self,
        pos: BlockPos,
        nbt: &NbtCompound,
        stamp: u64,
        expected: Option<&Arc<dyn BlockEntity>>,
        notification: Option<super::block_entity_notifications::BlockEntityNotification>,
    ) -> bool {
        let chunk_pos = pos.chunk_position();
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.unloading.contains_key(&chunk_pos)
            || loads
                .published_capture_stamps
                .get(&pos)
                .is_some_and(|published| stamp < *published)
        {
            return false;
        }
        if let Some(expected) = expected {
            let current = self
                .block_entities
                .get(&chunk_pos)
                .and_then(|entities| entities.get(&pos).cloned());
            if !current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, expected))
            {
                return false;
            }
        }
        loads.snapshots.remove(&pos);
        let published = self.level.read_chunk_sync(&chunk_pos, |chunk| {
            chunk
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(pos, nbt.clone());
            chunk.mark_dirty(true);
        });
        if published.is_some() {
            loads.published_capture_stamps.insert(pos, stamp);
            if let Some(notification) = notification {
                loads.notifications.push_back(notification);
            }
        }
        drop(loads);
        if published.is_some() {
            self.pending_block_entity_migrations.push(chunk_pos);
        }
        published.is_some()
    }

    pub(super) fn load_pending_block_entity(&self, pos: &BlockPos) -> Option<Arc<dyn BlockEntity>> {
        // Most lookups miss, and menu validity checks look up while holding their
        // screen handler: only take admission (level 10) when an image exists.
        if !self.has_pending_block_entity_image(pos) {
            return None;
        }
        let mutation = self.reserve_block_entity_mutation(*pos)?;
        let snapshot = self.pending_block_entity_snapshot(pos)?;
        // A read keeps the legacy invalidation/retry contract during decoding.
        // A writer restoring its predecessor already owns an outer permit.
        drop(mutation);
        #[cfg(test)]
        {
            let hook = self.persistence_test_hooks.lazy.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook();
            }
        }
        // Deserialization and binding can call native implementations, so neither owns the gate.
        let candidate = block_entity_from_nbt(&snapshot.nbt)?;
        let entity = self.promote_pending_block_entity(&snapshot, candidate)?;
        self.bind_block_entity_context(entity.as_ref());
        if let Some(jukebox) = entity.as_any().downcast_ref::<JukeboxBlockEntity>() {
            jukebox.notify_playback_power_if_changed();
        }
        Some(entity)
    }

    /// `Container.stillValidBlockEntity` identity against the live registry only. Menu
    /// validity runs under screen guards, so it never promotes a pending image (admission).
    pub(crate) fn is_live_block_entity(&self, entity: &Arc<dyn BlockEntity>) -> bool {
        let pos = entity.get_position();
        self.block_entities
            .get(&pos.chunk_position())
            .and_then(|entities| entities.get(&pos).cloned())
            .is_some_and(|live| Arc::ptr_eq(&live, entity))
    }

    fn has_pending_block_entity_image(&self, pos: &BlockPos) -> bool {
        let chunk = self
            .level
            .loaded_chunks
            .get(&pos.chunk_position())
            .map(|chunk| chunk.clone());
        chunk.is_some_and(|chunk| {
            chunk
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(pos)
        })
    }

    fn pending_block_entity_snapshot(
        &self,
        pos: &BlockPos,
    ) -> Option<PendingBlockEntitySnapshot<'_>> {
        let chunk_pos = pos.chunk_position();
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.stop_admission || loads.unloading.contains_key(&chunk_pos) {
            return None;
        }
        let chunk = self.level.loaded_chunks.get(&chunk_pos)?.clone();
        let nbt = chunk
            .pending_block_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(pos)?
            .clone();
        let token = loads
            .snapshots
            .get(pos)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let token = Arc::new(());
                loads.snapshots.insert(*pos, Arc::downgrade(&token));
                token
            });
        Some(PendingBlockEntitySnapshot {
            world: self,
            pos: *pos,
            chunk,
            nbt,
            token: Some(token),
        })
    }

    fn promote_pending_block_entity(
        &self,
        snapshot: &PendingBlockEntitySnapshot<'_>,
        candidate: Arc<dyn BlockEntity>,
    ) -> Option<Arc<dyn BlockEntity>> {
        let entity = {
            let chunk_pos = snapshot.pos.chunk_position();
            let mut loads = self
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = self.level.loaded_chunks.get(&chunk_pos)?;
            if !Arc::ptr_eq(&snapshot.chunk, &current)
                || loads.stop_admission
                || loads.unloading.contains_key(&chunk_pos)
            {
                return None;
            }
            if let Some(entity) = self
                .block_entities
                .get(&chunk_pos)
                .and_then(|entities| entities.get(&snapshot.pos).cloned())
            {
                return Some(entity);
            }
            let valid = loads.snapshots.get(&snapshot.pos).is_some_and(|token| {
                snapshot
                    .token
                    .as_ref()
                    .is_some_and(|strong| Weak::ptr_eq(token, &Arc::downgrade(strong)))
            });
            let pending = current
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !valid || !pending.contains_key(&snapshot.pos) {
                return None;
            }
            // LevelChunk.getBlockEntity/promotePendingBlockEntity has one canonical object per valid
            // pending record. Removal, NBT replacement and unload invalidate its outstanding copies.
            let mut entities = self.block_entities.entry(chunk_pos).or_default();
            let entity = entities
                .entry(snapshot.pos)
                .or_insert_with(|| {
                    if let Some(custom_data) = snapshot
                        .nbt
                        .get_compound("PumpkinCustomData")
                        .or_else(|| snapshot.nbt.get_compound("BukkitValues"))
                    {
                        self.custom_block_entity_data
                            .entry(snapshot.pos)
                            .or_insert_with(|| custom_data.clone());
                    }
                    candidate.clone()
                })
                .clone();
            loads.snapshots.remove(&snapshot.pos);
            entity
        };
        // A native losing block entity can run code on drop; release registry guards first.
        drop(candidate);
        Some(entity)
    }

    pub(super) fn invalidate_pending_block_entity_load(
        &self,
        pos: &BlockPos,
    ) -> MutexGuard<'_, PendingBlockEntityLoads> {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loads.snapshots.remove(pos);
        loads
    }

    /// Retires and removes a chunk's live block entities after its unload capture.
    #[cfg(test)]
    pub(crate) fn unload_block_entities(&self, chunk_pos: Vector2<i32>) -> bool {
        if !self.begin_block_entity_unload(chunk_pos) {
            return false;
        }
        self.finish_block_entity_unload_capture(chunk_pos);
        true
    }

    pub(crate) fn finish_block_entity_unload_capture(&self, chunk_pos: Vector2<i32>) {
        // LevelChunk.clearAllBlockEntities retires publication with the chunk. Serialization
        // stays outside the gate because native BlockEntity implementations can call back.
        self.save_block_entities(chunk_pos, true);
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retired = self.block_entities.remove(&chunk_pos);
        loads
            .published_capture_stamps
            .retain(|pos, _| pos.chunk_position() != chunk_pos);
        if let Some(unloading) = loads.unloading.get_mut(&chunk_pos) {
            unloading.captures -= 1;
            unloading.completed = true;
        }
        drop(loads);
        drop(retired);
    }

    pub(super) fn install_block_entity_from_observed(
        &self,
        position: BlockPos,
        expected: BlockStateId,
        entity: &Arc<dyn BlockEntity>,
    ) -> ObservedBlockEntityInstallation {
        let chunk_pos = position.chunk_position();
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.unloading.contains_key(&chunk_pos)
            || self.get_block_state_id_if_loaded(&position) != Some(expected)
        {
            return ObservedBlockEntityInstallation::Rejected;
        }
        // LevelChunk.setBlockEntity: admission and the registry swap share the terrain gate.
        // Retain the exact predecessor so native retirement runs after all ownership releases.
        let previous = self
            .block_entities
            .entry(chunk_pos)
            .or_default()
            .insert(position, entity.clone());
        loads.snapshots.remove(&position);
        if previous
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, entity))
        {
            loads.published_capture_stamps.remove(&position);
        }
        drop(loads);
        ObservedBlockEntityInstallation::Installed(previous)
    }

    pub(super) fn replace_live_block_entity(
        &self,
        pos: BlockPos,
        entity: Arc<dyn BlockEntity>,
    ) -> bool {
        let chunk_pos = pos.chunk_position();
        loop {
            let previous = self
                .block_entities
                .get(&chunk_pos)
                .and_then(|entities| entities.get(&pos).cloned());
            if let Some(previous) = &previous {
                // LevelChunk.setBlockEntity retires the exact predecessor before swapping it.
                // This object work stays outside the registry/admission guards.
                Self::retire_replaced_block_entity(previous, &entity);
            }

            let mut loads = self
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if loads.unloading.contains_key(&chunk_pos) {
                return false;
            }
            let current = self
                .block_entities
                .get(&chunk_pos)
                .and_then(|entities| entities.get(&pos).cloned());
            let same_predecessor = match (&previous, &current) {
                (Some(previous), Some(current)) => Arc::ptr_eq(previous, current),
                (None, None) => true,
                _ => false,
            };
            if !same_predecessor {
                drop(loads);
                continue;
            }
            let is_new_incarnation = previous
                .as_ref()
                .is_none_or(|previous| !Arc::ptr_eq(previous, &entity));
            self.block_entities
                .entry(chunk_pos)
                .or_default()
                .insert(pos, entity);
            loads.snapshots.remove(&pos);
            if is_new_incarnation {
                loads.published_capture_stamps.remove(&pos);
            }
            drop(loads);
            drop(previous);
            return true;
        }
    }

    pub(super) fn remove_observed_block(
        &self,
        position: &BlockPos,
        expected: BlockStateId,
        replacement: BlockStateId,
    ) -> Option<RetiredBlockEntity> {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.write_block_state_if(position, replacement, |current| current == expected)?;
        Some(self.remove_block_entity_under_publication_gate(position, &mut loads))
    }

    pub(super) fn remove_loaded_block_entity(&self, pos: &BlockPos) -> RetiredBlockEntity {
        // LevelChunk.removeBlockEntity retires live and pending state before another promotion.
        let mut loads = self.invalidate_pending_block_entity_load(pos);
        self.remove_block_entity_under_publication_gate(pos, &mut loads)
    }

    pub(super) fn reserve_bed_break(
        &self,
        position: BlockPos,
        expected: BlockStateId,
    ) -> Option<BedBreakSnapshot<'_>> {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads
            .unloading
            .get(&position.chunk_position())
            .is_some_and(|unloading| !unloading.completed || unloading.captures != 0)
            || self.get_block_state_id_if_loaded(&position) != Some(expected)
            || loads.bed_breaks.contains(&position)
            || loads.bed_uses.contains(&position)
        {
            return None;
        }
        let chunk = self
            .level
            .loaded_chunks
            .get(&position.chunk_position())?
            .clone();
        loads.bed_breaks.insert(position);
        Some(BedBreakSnapshot {
            world: self,
            position,
            expected,
            chunk,
        })
    }

    pub(super) fn reserve_bed_use(
        &self,
        halves: &[(BlockPos, BlockStateId)],
    ) -> Option<BedUseReservation<'_>> {
        let [first, second] = halves else {
            return None;
        };
        let positions = [first.0, second.0];
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if halves.iter().any(|(position, expected)| {
            loads
                .unloading
                .get(&position.chunk_position())
                .is_some_and(|unloading| !unloading.completed || unloading.captures != 0)
                || self.get_block_state_id_if_loaded(position) != Some(*expected)
                || loads.bed_breaks.contains(position)
                || loads.bed_uses.contains(position)
        }) {
            return None;
        }
        loads.bed_uses.extend(positions);
        Some(BedUseReservation {
            world: self,
            positions,
        })
    }

    pub(super) fn remove_reserved_bed(
        &self,
        snapshot: &BedBreakSnapshot<'_>,
    ) -> Option<RetiredBlockEntity> {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !snapshot.chunk_is_current() {
            return None;
        }
        // TODO(#230 B4): Check a per-cell mutation identity when one exists;
        // state equality cannot distinguish a removed and reinstalled identical bed.
        self.remove_bed_states_under_publication_gate(
            &[(snapshot.position, snapshot.expected)],
            &mut loads,
        )
        .pop()
        .map(|(_, _, entity)| entity)
    }

    pub(super) fn remove_bed_states_if(
        &self,
        halves: &[(BlockPos, BlockStateId)],
    ) -> Vec<(BlockPos, BlockStateId, RetiredBlockEntity)> {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if halves
            .iter()
            .any(|(position, _)| loads.bed_breaks.contains(position))
        {
            return Vec::new();
        }
        self.remove_bed_states_under_publication_gate(halves, &mut loads)
    }

    fn remove_bed_states_under_publication_gate(
        &self,
        halves: &[(BlockPos, BlockStateId)],
        loads: &mut PendingBlockEntityLoads,
    ) -> Vec<(BlockPos, BlockStateId, RetiredBlockEntity)> {
        if halves.iter().any(|(position, expected)| {
            loads.unloading.contains_key(&position.chunk_position())
                || self.get_block_state_id_if_loaded(position) != Some(*expected)
        }) {
            return Vec::new();
        }
        let mut chunks = Vec::new();
        for (position, _) in halves {
            let Some(chunk) = self
                .level
                .read_chunk_sync(&position.chunk_position(), Arc::clone)
            else {
                return Vec::new();
            };
            if !chunks.iter().any(|other| Arc::ptr_eq(other, &chunk)) {
                chunks.push(chunk);
            }
        }
        let updates = halves
            .iter()
            .map(|&(position, expected)| (position, expected, BlockStateId::AIR))
            .collect::<Vec<_>>();
        if !ChunkData::set_observed_blocks(&chunks, &updates) {
            return Vec::new();
        }
        let mut committed = Vec::with_capacity(halves.len());
        for &(position, expected) in halves {
            // Keep add_block_entity/pending promotion outside the raw-write/retirement gap.
            // All native retirement callbacks run only after this gate is released.
            let retired = self.remove_block_entity_under_publication_gate(&position, loads);
            committed.push((position, expected, retired));
        }
        committed
    }

    pub(super) fn remove_block_entity_under_publication_gate(
        &self,
        pos: &BlockPos,
        loads: &mut PendingBlockEntityLoads,
    ) -> RetiredBlockEntity {
        loads.snapshots.remove(pos);
        loads.published_capture_stamps.remove(pos);
        let chunk_pos = pos.chunk_position();
        let retired = self
            .block_entities
            .get_mut(&chunk_pos)
            .and_then(|mut entities| entities.remove(pos));
        let removed_pending = self
            .level
            .read_chunk_sync(&chunk_pos, |chunk| {
                let removed = remove_pending_block_entity(chunk, pos);
                if retired.is_some() {
                    chunk.mark_dirty(true);
                }
                removed
            })
            .flatten();
        self.custom_block_entity_data.remove(pos);
        if retired.is_some() || removed_pending.is_some() {
            self.block_entities
                .remove_if(&chunk_pos, |_, entities| entities.is_empty());
        }
        RetiredBlockEntity {
            live: retired,
            pending: removed_pending,
        }
    }
}
