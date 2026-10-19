//! `PersistentEntitySectionManager`: `updateChunkStatus`, `processPendingLoads`, `processUnloads`.

use std::sync::{Arc, PoisonError, atomic::Ordering};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    math::vector2::Vector2,
    sync::{LockClass, Mutex},
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{Level, SyncChunk, SyncEntityChunk};
use crate::chunk::io::FileIO;

#[derive(Default)]
pub struct EntityUnloads {
    next_generation: u64,
    generations: FxHashMap<Vector2<i32>, u64>,
    pending: FxHashMap<Vector2<i32>, EntityUnload>,
    retained_terrain: FxHashSet<Vector2<i32>>,
}

impl LockClass for EntityUnloads {
    const NAME: &'static str = "entity unload admission";
    const LEVEL: u8 = 10;
}

pub(super) enum EntityLoads {}
impl LockClass for EntityLoads {
    const NAME: &'static str = "entity storage load registry";
    const LEVEL: u8 = 20;
}

pub type UnloadAdmission = Arc<Mutex<EntityUnloads, EntityUnloads>>;

impl EntityUnloads {
    pub(super) fn forget(&mut self, pos: Vector2<i32>) {
        self.generations.remove(&pos);
        self.pending.remove(&pos);
        self.retained_terrain.remove(&pos);
    }

    pub(crate) fn retains_terrain(&self, pos: Vector2<i32>) -> bool {
        self.retained_terrain.contains(&pos)
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
    /// Retains this terrain incarnation until a reopened block-entity image is captured.
    /// Call outside block-entity admission and without a loaded-chunk entry guard.
    pub fn request_block_entity_recapture(&self, pos: Vector2<i32>, expected: &SyncChunk) -> bool {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // ChunkMap.scheduleUnload takes this same admission before removing published terrain.
        if !self
            .loaded_chunks
            .get(&pos)
            .is_some_and(|chunk| Arc::ptr_eq(chunk.value(), expected))
        {
            return false;
        }
        if !self.chunk_watchers.contains_key(&pos) {
            // A reopening may race the preceding retirement's finish. Give the new capture its
            // own generation so that finish cannot release this terrain fence.
            unloads.invalidate(pos);
            self.request_entity_unload_locked(&mut unloads, pos);
        }
        true
    }

    /// Publishes watcher ownership synchronously, before releasing player tickets.
    pub fn watch_entity_chunks(&self, chunks: &[Vector2<i32>]) {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.watch_entity_chunks_locked(&mut unloads, chunks);
    }

    fn watch_entity_chunks_locked(&self, unloads: &mut EntityUnloads, chunks: &[Vector2<i32>]) {
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
            .unwrap_or_else(PoisonError::into_inner);
        self.unwatch_entity_chunks_locked(&mut unloads, chunks)
    }

    fn unwatch_entity_chunks_locked(
        &self,
        unloads: &mut EntityUnloads,
        chunks: &[Vector2<i32>],
    ) -> Vec<Vector2<i32>> {
        let mut hidden = Vec::new();
        for &pos in chunks {
            if let dashmap::Entry::Occupied(mut entry) = self.chunk_watchers.entry(pos) {
                *entry.get_mut() = entry.get().saturating_sub(1);
                if *entry.get() == 0 {
                    entry.remove();
                    hidden.push(pos);
                    self.request_entity_unload_locked(unloads, pos);
                }
            }
        }
        hidden
    }

    /// Publishes forced membership/tickets and visibility under the same unload admission.
    /// The change closure is leaf-only: no callbacks, serialization, sends or awaits.
    pub fn change_forced_entity_chunks<R>(
        &self,
        forced: bool,
        change: impl FnOnce() -> (Vec<Vector2<i32>>, R),
    ) -> R {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (changed, result) = change();
        if forced {
            self.watch_entity_chunks_locked(&mut unloads, &changed);
        } else {
            self.unwatch_entity_chunks_locked(&mut unloads, &changed);
        }
        result
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
        if self.chunk_watchers.contains_key(&pos) {
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
                storage: self
                    .loaded_entity_chunks
                    .get(&pos)
                    .map(|entry| entry.clone()),
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
            .unwrap_or_else(PoisonError::into_inner);
        for pos in chunks {
            self.request_entity_unload_locked(&mut unloads, *pos.borrow());
        }
    }

    /// Copies pending work while keeping its storage protected during snapshot preparation.
    pub fn pending_entity_unloads(&self) -> Vec<EntityUnload> {
        self.entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .values()
            .cloned()
            .collect()
    }

    /// Commits current hidden storage; the closure may only publish prepared leaf state.
    /// No object callbacks, serialization, packets, plugins or awaits are allowed inside it.
    /// On success the tick owner must publish detached records, then call `finish_entity_unload`.
    pub fn commit_entity_unload(
        &self,
        request: &EntityUnload,
        commit: impl FnOnce() -> bool,
    ) -> bool {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let pos = request.position;
        // ChunkMap.scheduleUnload rechecks both the pending future and expected holder.
        if unloads.generations.get(&pos) != Some(&request.generation)
            || self.chunk_watchers.contains_key(&pos)
        {
            return false;
        }
        let storage = self
            .loaded_entity_chunks
            .get(&pos)
            .map(|entry| entry.clone());
        if storage.as_ref().map(Arc::as_ptr) != request.storage.as_ref().map(Arc::as_ptr) {
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
        true
    }

    /// Releases a committed unload after detached records are published outside admission.
    /// Rewatch or a fresh hidden incarnation retains its own authority.
    pub fn finish_entity_unload(&self, request: &EntityUnload) {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if unloads.generations.get(&request.position) == Some(&request.generation) {
            unloads.invalidate(request.position);
            unloads.retained_terrain.remove(&request.position);
        }
    }

    /// Claims an installed storage incarnation for activation, cancelling older unloads.
    pub fn activate_entity_chunk(&self, storage: &SyncEntityChunk) -> Option<Vec<NbtCompound>> {
        let mut unloads = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let pos = Vector2::new(storage.x, storage.z);
        if !self
            .loaded_entity_chunks
            .get(&pos)
            .map(|entry| entry.clone())
            .is_some_and(|current| Arc::ptr_eq(&current, storage))
        {
            return None;
        }
        unloads.invalidate(pos);
        let data = storage.data.lock().unwrap_or_else(PoisonError::into_inner);
        let records = if storage.live.swap(true, Ordering::AcqRel) {
            Vec::new()
        } else {
            data.clone()
        };
        drop(data);
        // An unwatched activation still needs a fresh request for its new incarnation.
        if !self.chunk_watchers.contains_key(&pos) && storage.live.load(Ordering::Acquire) {
            self.request_entity_unload_locked(&mut unloads, pos);
        }
        Some(records)
    }
}

impl Level {
    pub(super) fn clean_retired_entity_chunks(self: &Arc<Self>, positions: Vec<Vector2<i32>>) {
        for pos in positions {
            let candidate = {
                let mut unloads = self
                    .entity_unloads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if self.chunk_watchers.contains_key(&pos) || unloads.pending.contains_key(&pos) {
                    continue;
                }
                let candidate = self
                    .loaded_entity_chunks
                    .get(&pos)
                    .map(|entry| entry.clone());
                if candidate.is_none()
                    && !self
                        .entity_loads
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .contains_key(&pos)
                    && !self.pending_entity_generations.contains_key(&pos)
                {
                    unloads.forget(pos);
                }
                candidate
            };
            let Some(storage) = candidate else { continue };
            if storage.live.load(Ordering::Acquire) {
                continue;
            }
            // IOWorker.store/loadAsync: register while still published, without owner guards.
            self.entity_saver
                .queue_chunks(&self.level_folder, vec![(pos, storage.clone())]);
            let removed = {
                let mut unloads = self
                    .entity_unloads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let mut loads = self
                    .entity_loads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if self.chunk_watchers.contains_key(&pos) || unloads.pending.contains_key(&pos) {
                    continue;
                }
                let removed = self.loaded_entity_chunks.remove_if(&pos, |_, current| {
                    Arc::ptr_eq(current, &storage) && !current.live.load(Ordering::Acquire)
                });
                if removed.is_some() {
                    if let Some(load) = loads.get_mut(&pos) {
                        load.epoch = load.epoch.wrapping_add(1);
                    }
                    unloads.forget(pos);
                }
                removed.is_some()
            };
            if removed {
                let level = self.clone();
                self.spawn_task(async move {
                    if let Err(error) = level
                        .entity_saver
                        .flush_chunks(&level.level_folder, &[pos])
                        .await
                    {
                        tracing::error!("Entity unload save failed: {error}");
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn forgotten_never_installed_storage_does_not_reuse_unload_authority() {
        let directory = tempfile::tempdir().unwrap();
        let level = Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            directory.path().into(),
            0,
            pumpkin_data::dimension::Dimension::OVERWORLD,
        );
        let pos = Vector2::new(0, 0);
        level.watch_entity_chunks(&[pos]);
        level.unwatch_entity_chunks(&[pos]);
        let old = level.pending_entity_unloads().pop().unwrap();
        assert!(old.storage.is_none());
        assert!(level.commit_entity_unload(&old, || true));
        level.finish_entity_unload(&old);
        level.clean_retired_entity_chunks(vec![pos]);
        assert!(
            !level
                .entity_unloads
                .lock()
                .unwrap()
                .generations
                .contains_key(&pos)
        );
        level.watch_entity_chunks(&[pos]);
        level.unwatch_entity_chunks(&[pos]);
        let new = level.pending_entity_unloads().pop().unwrap();
        assert!(new.generation > old.generation);
        assert!(!level.commit_entity_unload(&old, || panic!("forgotten authority was reused")));
        assert!(level.commit_entity_unload(&new, || true));
        level.finish_entity_unload(&new);
        level.shutdown().await.unwrap();
    }
}
