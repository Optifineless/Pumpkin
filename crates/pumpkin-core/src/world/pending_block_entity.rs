use std::sync::{Arc, MutexGuard, Weak};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_util::sync::{SnapshotBoundary as SB, assert_not_in_snapshot};
use pumpkin_world::chunk::{ChunkData, io::Dirtiable};
use rustc_hash::{FxHashMap, FxHashSet};

use super::portal::frame_effects::RetiredBlockEntity;
use super::{World, remove_pending_block_entity};
use crate::block::entities::{BlockEntity, block_entity_from_nbt, capture::capture_legacy};

const MAX_REPLACEMENT_SNAPSHOT_RETRIES: usize = 2;

enum SnapshotPublication {
    Published,
    Replaced(Arc<dyn BlockEntity>),
    Missing,
    Older,
}

#[cfg(test)]
pub(super) type PublicationHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub(super) struct PendingBlockEntityLoads {
    pub(super) snapshots: FxHashMap<BlockPos, Weak<()>>,
    // Stamps are allocated under this gate before serialization and tracked per published position.
    next_capture_stamp: u64,
    published_capture_stamps: FxHashMap<BlockPos, u64>,
    // Admission first stops pending promotion; sealing also stops live replacement and publication.
    unloading: FxHashMap<Vector2<i32>, usize>,
    sealed_unloading: FxHashSet<Vector2<i32>>,
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
    /// Mirrors LevelChunk.getBlockEntityNbtForSaving: serialize the live instance and publish it only if current.
    pub(super) fn publish_block_entity_snapshot_if_current(
        &self,
        pos: BlockPos,
        expected: &Arc<dyn BlockEntity>,
        retiring: bool,
    ) {
        let mut expected = expected.clone();
        for retry in 0..=MAX_REPLACEMENT_SNAPSHOT_RETRIES {
            let stamp = self.next_block_entity_capture_stamp();
            let nbt = self.capture_block_entity_snapshot(pos, &expected);
            match self.try_publish_block_entity_snapshot(pos, &expected, &nbt, stamp, retiring) {
                SnapshotPublication::Older
                    if retiring && retry < MAX_REPLACEMENT_SNAPSHOT_RETRIES => {}
                SnapshotPublication::Published
                | SnapshotPublication::Missing
                | SnapshotPublication::Older => return,
                SnapshotPublication::Replaced(current) => {
                    if retiring {
                        // LevelChunk.clearAllBlockEntities retires each live container on unload.
                        // A replacement joining this final capture needs the same retirement first.
                        super::container_lifecycle::retire_block_entity(&current);
                    }
                    if retry == MAX_REPLACEMENT_SNAPSHOT_RETRIES {
                        return;
                    }
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
        if retiring {
            // LevelChunk.clearAllBlockEntities runs after this final snapshot; no later additions may join it.
            loads.sealed_unloading.insert(chunk_pos);
        } else if loads.unloading.contains_key(&chunk_pos) {
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
    ) -> bool {
        let chunk_pos = pos.chunk_position();
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.sealed_unloading.contains(&chunk_pos)
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
        }
        drop(loads);
        if published.is_some() {
            self.pending_block_entity_migrations.push(chunk_pos);
        }
        published.is_some()
    }

    pub(super) fn block_entity_addition_allowed(&self, pos: BlockPos) -> bool {
        let chunk_pos = pos.chunk_position();
        let loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Before sealing, only replacements at an already captured position may join the retiring image.
        !loads.sealed_unloading.contains(&chunk_pos)
            && (!loads.unloading.contains_key(&chunk_pos)
                || self
                    .block_entities
                    .get(&chunk_pos)
                    .is_some_and(|entities| entities.contains_key(&pos)))
    }

    pub(super) fn begin_block_entity_unload(&self, chunk_pos: Vector2<i32>) {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *loads.unloading.entry(chunk_pos).or_default() += 1;
        loads
            .snapshots
            .retain(|pos, _| pos.chunk_position() != chunk_pos);
    }

    pub(super) fn seal_block_entity_unload(&self, chunk_pos: Vector2<i32>) {
        self.pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sealed_unloading
            .insert(chunk_pos);
    }

    pub(super) fn finish_block_entity_unload(&self, chunk_pos: Vector2<i32>) {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = loads.unloading.get_mut(&chunk_pos) {
            *count -= 1;
            if *count == 0 {
                loads.unloading.remove(&chunk_pos);
                loads.sealed_unloading.remove(&chunk_pos);
            }
        }
    }

    pub(super) fn load_pending_block_entity(&self, pos: &BlockPos) -> Option<Arc<dyn BlockEntity>> {
        let snapshot = self.pending_block_entity_snapshot(pos)?;
        #[cfg(test)]
        {
            let hook = self.lazy_block_entity_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook();
            }
        }
        // Deserialization and binding can call native implementations, so neither owns the gate.
        let candidate = block_entity_from_nbt(&snapshot.nbt)?;
        let entity = self.promote_pending_block_entity(&snapshot, candidate)?;
        self.bind_block_entity_context(entity.as_ref());
        Some(entity)
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
        if loads.unloading.contains_key(&chunk_pos) {
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
            if !Arc::ptr_eq(&snapshot.chunk, &current) || loads.unloading.contains_key(&chunk_pos) {
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
    pub(crate) fn unload_block_entities(&self, chunk_pos: Vector2<i32>) {
        self.begin_block_entity_unload(chunk_pos);
        // LevelChunk.clearAllBlockEntities retires publication with the chunk. Serialization
        // stays outside the gate because native BlockEntity implementations can call back.
        self.save_block_entities(chunk_pos, true);
        self.seal_block_entity_unload(chunk_pos);
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retired = self.block_entities.remove(&chunk_pos);
        loads
            .published_capture_stamps
            .retain(|pos, _| pos.chunk_position() != chunk_pos);
        drop(loads);
        drop(retired);
        self.finish_block_entity_unload(chunk_pos);
    }

    pub(super) fn replace_live_block_entity(
        &self,
        pos: BlockPos,
        entity: Arc<dyn BlockEntity>,
    ) -> bool {
        let loads = self.invalidate_pending_block_entity_load(&pos);
        let chunk_pos = pos.chunk_position();
        if loads.sealed_unloading.contains(&chunk_pos)
            || (loads.unloading.contains_key(&chunk_pos)
                && !self
                    .block_entities
                    .get(&chunk_pos)
                    .is_some_and(|entities| entities.contains_key(&pos)))
        {
            return false;
        }
        let retired = self
            .block_entities
            .entry(chunk_pos)
            .or_default()
            .insert(pos, entity);
        drop(loads);
        drop(retired);
        true
    }

    pub(super) fn remove_loaded_block_entity(&self, pos: &BlockPos) -> RetiredBlockEntity {
        {
            // LevelChunk.removeBlockEntity retires live and pending state before another promotion.
            let mut loads = self.invalidate_pending_block_entity_load(pos);
            let chunk_pos = pos.chunk_position();
            loads.published_capture_stamps.remove(pos);
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
}
