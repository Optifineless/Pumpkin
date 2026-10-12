//! `PersistentEntitySectionManager` visibility and pending unload ownership.

use std::sync::{Arc, atomic::Ordering};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector2::Vector2;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{Level, SyncEntityChunk};
use crate::chunk::io::FileIO;

#[derive(Default)]
pub struct EntityUnloads {
    next_generation: u64,
    generations: FxHashMap<Vector2<i32>, u64>,
    pending: FxHashMap<Vector2<i32>, EntityUnload>,
    retained_terrain: FxHashSet<Vector2<i32>>,
}

impl EntityUnloads {
    pub(super) fn forget(&mut self, pos: Vector2<i32>) {
        self.generations.remove(&pos);
        self.pending.remove(&pos);
        self.retained_terrain.remove(&pos);
    }

    pub(crate) fn retains_terrain(&self, pos: Vector2<i32>) -> bool {
        self.retained_terrain.contains(&pos)
    }

    #[must_use]
    pub(super) fn is_pending(&self, pos: Vector2<i32>) -> bool {
        self.pending.contains_key(&pos)
    }

    fn invalidate(&mut self, pos: Vector2<i32>) {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.generations.insert(pos, self.next_generation);
        self.pending.remove(&pos);
    }
}

/// An unload owns only the captured generation and storage instance.
#[derive(Clone)]
pub struct EntityUnload {
    pub position: Vector2<i32>,
    pub storage: Option<SyncEntityChunk>,
    generation: u64,
}

impl Level {
    /// Publishes watcher ownership synchronously, before releasing player tickets.
    pub fn watch_entity_chunks(&self, chunks: &[Vector2<i32>]) {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for &pos in chunks {
            unloads.invalidate(pos);
            unloads.retained_terrain.insert(pos);
            self.chunk_watchers
                .entry(pos)
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
        }
    }

    /// Removes watcher ownership and captures unload authority at the last unwatch.
    pub fn unwatch_entity_chunks(&self, chunks: &[Vector2<i32>]) -> Vec<Vector2<i32>> {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut hidden = Vec::new();
        for &pos in chunks {
            if let dashmap::Entry::Occupied(mut entry) = self.chunk_watchers.entry(pos) {
                *entry.get_mut() = entry.get().saturating_sub(1);
                if *entry.get() == 0 {
                    entry.remove();
                    hidden.push(pos);
                    self.request_entity_unload_locked(&mut unloads, pos);
                }
            }
        }
        hidden
    }

    /// Updates storage cache watch counts after synchronous visibility publication.
    pub async fn update_entity_storage_watchers(
        &self,
        watched: &[Vector2<i32>],
        hidden: &[Vector2<i32>],
    ) {
        self.entity_saver
            .watch_chunks(&self.level_folder, watched)
            .await;
        self.entity_saver
            .unwatch_chunks(&self.level_folder, hidden)
            .await;
    }

    fn request_entity_unload_locked(&self, unloads: &mut EntityUnloads, pos: Vector2<i32>) {
        if self.is_chunk_watched(&pos) {
            return;
        }
        let generation = *unloads.generations.entry(pos).or_insert_with(|| {
            unloads.next_generation = unloads.next_generation.wrapping_add(1);
            unloads.next_generation
        });
        unloads.retained_terrain.insert(pos);
        unloads.pending.insert(
            pos,
            EntityUnload {
                position: pos,
                storage: self.get_entity_chunk_sync(&pos),
                generation,
            },
        );
    }

    /// Requests tick-boundary retirement; positions alone never authorize removal.
    pub fn request_entity_unloads(
        &self,
        chunks: impl IntoIterator<Item = impl std::borrow::Borrow<Vector2<i32>>>,
    ) {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for pos in chunks {
            self.request_entity_unload_locked(&mut unloads, *pos.borrow());
        }
    }

    /// Copies pending work while keeping its storage protected during snapshot preparation.
    pub fn pending_entity_unloads(&self) -> Vec<EntityUnload> {
        self.entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .values()
            .cloned()
            .collect()
    }

    /// Commits only current, hidden storage. The closure must not send packets or call plugins.
    pub fn commit_entity_unload(
        &self,
        request: &EntityUnload,
        commit: impl FnOnce() -> bool,
    ) -> bool {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pos = request.position;
        // ChunkMap.scheduleUnload rechecks both the pending future and expected holder.
        if unloads.generations.get(&pos) != Some(&request.generation) || self.is_chunk_watched(&pos)
        {
            return false;
        }
        let storage = self.get_entity_chunk_sync(&pos);
        if !same_storage(storage.as_ref(), request.storage.as_ref()) {
            if let Some(pending) = unloads.pending.get_mut(&pos) {
                pending.storage = storage;
            }
            return false;
        }
        if !commit() {
            unloads
                .pending
                .entry(pos)
                .or_insert_with(|| request.clone());
            return false;
        }
        unloads.invalidate(pos);
        unloads.retained_terrain.remove(&pos);
        true
    }

    /// Claims an installed storage incarnation for activation, cancelling older unloads.
    pub fn activate_entity_chunk(&self, storage: &SyncEntityChunk) -> Option<Vec<NbtCompound>> {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pos = Vector2::new(storage.x, storage.z);
        if !self
            .get_entity_chunk_sync(&pos)
            .is_some_and(|current| Arc::ptr_eq(&current, storage))
        {
            return None;
        }
        unloads.invalidate(pos);
        let records = storage.entities_for_activation();
        // An unwatched activation still needs a fresh request for its new incarnation.
        if !self.is_chunk_watched(&pos) && storage.live.load(Ordering::Acquire) {
            self.request_entity_unload_locked(&mut unloads, pos);
        }
        Some(records)
    }
}

fn same_storage(a: Option<&SyncEntityChunk>, b: Option<&SyncEntityChunk>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

impl Level {
    /// Captures save authority before reading the live registry. No guard escapes this call.
    pub fn entity_snapshot(&self, storage: &SyncEntityChunk) -> Option<EntityUnload> {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pos = Vector2::new(storage.x, storage.z);
        if !self
            .get_entity_chunk_sync(&pos)
            .is_some_and(|current| Arc::ptr_eq(&current, storage))
        {
            return None;
        }
        let generation = *unloads.generations.entry(pos).or_insert(0);
        Some(EntityUnload {
            position: pos,
            storage: Some(storage.clone()),
            generation,
        })
    }

    /// Validates an ordinary save under the same admission as retirement. Commit must be leaf-only.
    pub fn commit_entity_snapshot(
        &self,
        request: &EntityUnload,
        commit: impl FnOnce() -> bool,
    ) -> bool {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if unloads.generations.get(&request.position) != Some(&request.generation)
            || !same_storage(
                self.get_entity_chunk_sync(&request.position).as_ref(),
                request.storage.as_ref(),
            )
            || !commit()
        {
            return false;
        }
        unloads.next_generation = unloads.next_generation.wrapping_add(1);
        let generation = unloads.next_generation;
        unloads.generations.insert(request.position, generation);
        if let Some(pending) = unloads.pending.get_mut(&request.position) {
            pending.generation = generation;
        }
        true
    }
}
