//! ChunkMap.scheduleUnload ownership, extended through durable publication.

#[path = "chunk_admission_cache.rs"]
mod chunk_admission_cache;
pub use chunk_admission_cache::ChunkAdmissionCache;

#[path = "chunk_admission_retirement.rs"]
mod chunk_admission_retirement;

#[path = "chunk_unload_admission.rs"]
mod chunk_unload_admission;

#[path = "tick_admission.rs"]
mod tick_admission;
pub use tick_admission::TickMutationScope;

use std::sync::{
    Arc, Mutex, OnceLock, RwLock,
    atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
};

use dashmap::DashMap;
use pumpkin_util::math::vector2::Vector2;

use crate::{
    chunk::io::{Dirtiable, FileIO},
    chunk_system::ChunkLoading,
    level::{Level, SyncChunk},
};

const SAVING: u8 = 0;
const SAVED: u8 = 1;
const FAILED: u8 = 2;
// ChunkMap.scheduleUnload compares future identities even when a holder is replaced.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct ChunkLifecycle {
    pub watchers: usize,
    pub generation: u64,
    /// Read under the lifecycle mutex; change only through `set_quiescing`.
    pub quiescing: bool,
    /// Retains admission until the last canonical block entity is unpublished under this mutex.
    pub live_block_entities: bool,
    admission: Arc<ChunkAdmission>,
    pending: Option<PendingUnload>,
    caches: Vec<std::sync::Weak<ChunkAdmissionCache>>,
    cache_purge_at: usize,
}

struct PendingUnload {
    generation: u64,
    revision: u64,
    entity: Option<(crate::level::SyncEntityChunk, u64)>,
    completion: Arc<AtomicU8>,
}

impl ChunkLifecycle {
    /// Counts admitted operations; close admission before deciding to unload.
    #[must_use]
    pub fn mutations(&self) -> usize {
        self.admission.mutations.load(Ordering::SeqCst)
    }

    /// Cancels completion of any snapshot captured by the preceding generation.
    pub fn invalidate(&mut self) {
        self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        self.set_quiescing(false);
    }
    /// Changes unload admission while holding this position's lifecycle mutex.
    pub fn set_quiescing(&mut self, value: bool) {
        self.quiescing = value;
        self.admission.quiescing.store(value, Ordering::SeqCst);
        if value {
            // Callback.onMove/removeSectionIfEmpty: a left section must not stay pinned by a cache.
            chunk_admission_cache::clear_caches(&mut self.caches);
            self.cache_purge_at = 0;
        }
    }
}

#[derive(Default)]
struct ChunkAdmission {
    quiescing: AtomicBool,
    mutations: AtomicUsize,
    retirement: OnceLock<chunk_admission_retirement::AbsentAdmission>,
}

/// A position's slow lifecycle state and its mutex-free mutation admission.
pub struct ChunkLifecycleCell {
    state: Mutex<ChunkLifecycle>,
    admission: Arc<ChunkAdmission>,
}

impl Default for ChunkLifecycleCell {
    fn default() -> Self {
        let state = ChunkLifecycle {
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            ..ChunkLifecycle::default()
        };
        let admission = state.admission.clone();
        Self {
            state: Mutex::new(state),
            admission,
        }
    }
}

impl std::ops::Deref for ChunkLifecycleCell {
    type Target = Mutex<ChunkLifecycle>;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl ChunkLifecycleCell {
    fn try_admit(&self, tick: bool) -> Option<ChunkMutation> {
        self.admission.try_admit(tick)
    }

    fn admit_locked(&self) -> ChunkMutation {
        self.admission.mutations.fetch_add(1, Ordering::SeqCst);
        ChunkMutation(Some(self.admission.clone()))
    }
}

impl ChunkAdmission {
    fn try_admit(self: &Arc<Self>, tick: bool) -> Option<ChunkMutation> {
        if self.quiescing.load(Ordering::SeqCst) {
            return None;
        }
        if tick
            && self
                .retirement
                .get()
                .is_none_or(|retirement| retirement.resident.load(Ordering::Acquire))
        {
            // The tick read barrier excludes both snapshot and detach until this scope ends.
            return Some(ChunkMutation(None));
        }
        self.mutations.fetch_add(1, Ordering::SeqCst);
        // ChunkMap.scheduleUnload: either the closing flag or this count wins the race.
        if self.quiescing.load(Ordering::SeqCst) {
            self.mutations.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(ChunkMutation(Some(self.clone())))
    }
}

type UnloadGate = Arc<dyn Fn(Vector2<i32>) -> bool + Send + Sync>;

#[derive(Default)]
pub struct ChunkLifecycles {
    states: Arc<DashMap<Vector2<i32>, Arc<ChunkLifecycleCell>>>,
    absent_retirements: Arc<crossbeam::queue::SegQueue<std::sync::Weak<ChunkAdmission>>>,
    unload_gate: RwLock<Option<UnloadGate>>,
    #[cfg(any(test, feature = "test-hooks"))]
    benchmark_off: std::sync::atomic::AtomicBool,
    #[cfg(any(test, feature = "test-hooks"))]
    benchmark_legacy: AtomicBool,
}

impl ChunkLifecycles {
    /// Selects the previous counted/uncached path in isolated performance fixtures.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn benchmark_use_legacy_admission(&self, legacy: bool) {
        self.benchmark_legacy.store(legacy, Ordering::Relaxed);
    }
    #[must_use]
    pub fn benchmark_legacy_admission(&self) -> bool {
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.benchmark_legacy.load(Ordering::Relaxed)
        }
        #[cfg(not(any(test, feature = "test-hooks")))]
        {
            false
        }
    }
    /// Disables admission for isolated timing scenes that never unload chunks.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn benchmark_disable_admission(&self, off: bool) {
        self.benchmark_off.store(off, Ordering::Relaxed);
    }
    /// Reports whether an isolated timing fixture bypasses admission.
    #[must_use]
    pub fn benchmark_admission_disabled(&self) -> bool {
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.benchmark_off.load(Ordering::Relaxed)
        }
        #[cfg(not(any(test, feature = "test-hooks")))]
        {
            false
        }
    }
    /// Acquire before live-object maps and chunk contents; never await or call plugins while held.
    /// Never admit an entity while holding any lifecycle mutex.
    #[must_use]
    pub fn at(&self, pos: Vector2<i32>) -> Arc<ChunkLifecycleCell> {
        self.get(pos)
            .unwrap_or_else(|| self.states.entry(pos).or_default().clone())
    }

    /// Looks up existing admission without creating state for absent terrain.
    #[must_use]
    pub fn get(&self, pos: Vector2<i32>) -> Option<Arc<ChunkLifecycleCell>> {
        self.states.get(&pos).map(|state| state.clone())
    }

    /// Counts lifecycle positions for absent-lookup regression tests.
    #[cfg(any(test, feature = "test-hooks"))]
    #[must_use]
    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    /// Installs the lease lane's short admission check; it must never serialize or call plugins.
    /// Lease acquisition must hold `at(pos)` and refuse quiescing before changing its counts.
    pub fn set_unload_gate(&self, gate: UnloadGate) {
        *self
            .unload_gate
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(gate);
    }

    /// Checks leases while the caller owns `at(pos)`; release any lease mutex before returning.
    #[must_use]
    pub fn can_begin_unload(&self, pos: Vector2<i32>) -> bool {
        let gate = self
            .unload_gate
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        gate.is_none_or(|gate| gate(pos))
    }

    /// Retires unused lifecycle state after its terrain holder and entity storage detach.
    pub(crate) fn retire(&self, pos: Vector2<i32>) {
        self.states.remove_if(&pos, |_, state| {
            if Arc::strong_count(state) != 1 {
                return false;
            }
            state.try_lock().is_ok_and(|state| {
                state.watchers == 0
                    && state.mutations() == 0
                    && !state.quiescing
                    && !state.live_block_entities
                    && state.pending.is_none()
            })
        });
    }

    /// Exercises retirement after detach in cross-crate lifecycle regression tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_retire(&self, pos: Vector2<i32>) {
        self.retire(pos);
    }
}

/// Keeps an admitted chunk mutation attached until it finishes.
#[must_use]
pub struct ChunkMutation(Option<Arc<ChunkAdmission>>);

impl Drop for ChunkMutation {
    fn drop(&mut self) {
        if let Some(admission) = &self.0
            && admission.mutations.fetch_sub(1, Ordering::SeqCst) == 1
        {
            admission.retire_absent();
        }
    }
}

impl Level {
    /// Skips unfinished save futures without consulting chunk tickets or building a root index.
    #[must_use]
    pub fn chunk_unload_save_in_progress(&self, pos: Vector2<i32>) -> bool {
        self.chunk_lifecycles.get(pos).is_some_and(|cell| {
            cell.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending
                .as_ref()
                .is_some_and(|pending| pending.completion.load(Ordering::Acquire) == SAVING)
        })
    }

    /// Pauses the next entity-region publication after serialization for regression tests.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pause_entity_storage_publication(
        &self,
    ) -> crate::chunk::io::file_manager::PublicationBarrier {
        self.entity_saver.pause_next_publication()
    }

    /// Publishes this exact entity snapshot and reports failure; callers own lifecycle admission.
    /// The saver retains failed Arcs and clears only the successfully published dirty revision.
    pub async fn save_retained_entity_chunk(
        &self,
        chunk: crate::level::SyncEntityChunk,
    ) -> Result<(), crate::chunk::ChunkWritingError> {
        self.entity_saver
            .save_chunks(
                &self.level_folder,
                vec![(Vector2::new(chunk.x, chunk.z), chunk)],
            )
            .await
    }

    /// Owns this position through explicit loading or writes and their side effects.
    /// Never call while holding `chunk_lifecycles.at(pos)` for this position: closing
    /// admission needs that same mutex. Acquire admission before lifecycle ownership.
    pub fn begin_chunk_mutation(&self, pos: Vector2<i32>) -> ChunkMutation {
        let lifecycle = self.chunk_mutation_cell(pos);
        self.admit_chunk_mutation(&lifecycle)
    }

    /// Admits a write only when the position already has terrain, entity storage or admission.
    /// Absent positions return None so a later concurrent load cannot receive an unadmitted write.
    pub fn begin_existing_chunk_mutation(&self, pos: Vector2<i32>) -> Option<ChunkMutation> {
        let lifecycle = self.mutation_lifecycle(pos)?;
        Some(self.admit_chunk_mutation(&lifecycle))
    }

    /// Admits through a cached cell retained from this level's `chunk_lifecycles.at(pos)`.
    /// Its owning Arc prevents retirement; never pass a different level's cell.
    pub fn admit_chunk_mutation(&self, lifecycle: &ChunkLifecycleCell) -> ChunkMutation {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.chunk_lifecycles.benchmark_admission_disabled() {
            return ChunkMutation(None);
        }
        if let Some(permit) = lifecycle.try_admit(self.has_tick_mutation_scope()) {
            return permit;
        }
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.invalidate();
        lifecycle.admit_locked()
    }

    fn mutation_lifecycle(&self, pos: Vector2<i32>) -> Option<Arc<ChunkLifecycleCell>> {
        self.chunk_lifecycles.get(pos).or_else(|| {
            (self.is_chunk_loaded(&pos) || self.get_entity_chunk_sync(&pos).is_some())
                .then(|| self.chunk_lifecycles.at(pos))
        })
    }

    /// Tick/activation admission closes while an unload snapshot is awaiting publication.
    pub fn try_chunk_mutation(&self, pos: Vector2<i32>) -> Option<ChunkMutation> {
        self.mutation_lifecycle(pos)?
            .try_admit(self.has_tick_mutation_scope())
    }

    /// Validate a retained plugin/piston handle before writes; ownership Arcs are not leases.
    /// Refuses detached objects; a canonical write cancels quiescence.
    /// Hold the returned permit through all side effects.
    pub fn try_mutate_retained_chunk(&self, chunk: &SyncChunk) -> Option<ChunkMutation> {
        let pos = Vector2::new(chunk.x, chunk.z);
        let lifecycle = self.mutation_lifecycle(pos)?;
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self
            .loaded_chunks
            .get(&pos)
            .is_some_and(|current| Arc::ptr_eq(&current, chunk))
        {
            return None;
        }
        // ServerChunkCache.getChunk: a write to loaded terrain cancels an obsolete unload.
        state.invalidate();
        Some(lifecycle.admit_locked())
    }

    /// Poll the exact retained object; true authorizes scheduler detachment.
    /// Storage must retain this Arc, clear only the published dirty revision, and report errors.
    /// Rewatch or an explicit mutation invalidates this completion before any live object detaches.
    pub fn poll_chunk_unload(self: &Arc<Self>, chunk: &SyncChunk) -> bool {
        let pos = Vector2::new(chunk.x, chunk.z);
        if !self.is_chunk_loaded(&pos) {
            return true;
        }
        let lifecycle = self.chunk_lifecycles.at(pos);
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self
            .loaded_chunks
            .get(&pos)
            .is_some_and(|current| Arc::ptr_eq(&current, chunk))
        {
            tracing::error!(?pos, "Cannot unload a noncanonical terrain chunk");
            return false;
        }
        state.set_quiescing(true);
        // ChunkMap.scheduleUnload retains ownership until the save future completes.
        // A writer preempted before its second flag read is not admitted to this snapshot.
        if let Some(pending) = &state.pending
            && (pending.completion.load(Ordering::Acquire) == SAVING
                || (pending.generation == state.generation && state.mutations() != 0))
        {
            if pending.generation != state.generation {
                state.set_quiescing(false);
            }
            return false;
        }
        if state.watchers != 0 || state.mutations() != 0 {
            // ChunkMap.processUnloads retries after the live tick releases its mutations.
            if state.watchers == 0
                && let Some(portal) = self.world_portal.load().as_ref()
            {
                portal.queue_chunk_unload(chunk);
            }
            state.set_quiescing(false);
            return false;
        }
        // Ticket propagation may have rewatched this holder before the scheduler drained changes.
        if self
            .chunk_loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pos_level
            .contains_key(&pos)
        {
            state.invalidate();
            return false;
        }
        if let Some(pending) = &state.pending {
            let result = pending.completion.load(Ordering::Acquire);
            if result == SAVING {
                if pending.generation != state.generation {
                    state.set_quiescing(false);
                }
                return false;
            }
            let current = pending.generation == state.generation
                && chunk.dirty.is_published(pending.revision)
                && pending.entity.as_ref().is_none_or(|(entity, revision)| {
                    entity.dirty.is_published(*revision)
                        && self
                            .get_entity_chunk_sync(&pos)
                            .is_some_and(|current| Arc::ptr_eq(&current, entity))
                });
            if result == SAVED && current && !chunk.is_dirty() {
                if let Some(portal) = self.world_portal.load().as_ref() {
                    if !portal.finish_chunk_unload(chunk, pending.generation) {
                        portal.cancel_chunk_unload(pos, pending.generation);
                        state.pending = None;
                        state.invalidate();
                        return false;
                    }
                    state.live_block_entities = false;
                }
                state.pending = None;
                self.detach_saved_entity_chunk(pos);
                // Removal is inside the admission lock, so lookup cannot return a detached BE.
                self.loaded_chunks.remove(&pos);
                self.loaded_chunk_changes
                    .push(crate::level::LoadedChunkChange::Unloaded(pos));
                state.set_quiescing(false);
                return true;
            }
            if let Some(portal) = self.world_portal.load().as_ref() {
                portal.cancel_chunk_unload(pos, pending.generation);
            }
            state.pending = None;
            // ChunkMap.scheduleUnload keeps retry ownership through the next snapshot.
        }
        // Piston leases must prevent the snapshot, not only final unpublication.
        if !self.chunk_lifecycles.can_begin_unload(pos) {
            state.set_quiescing(false);
            return false;
        }
        state.set_quiescing(true);
        if let Some(portal) = self.world_portal.load().as_ref()
            && !portal.prepare_chunk_unload(chunk, state.generation)
        {
            state.set_quiescing(false);
            return false;
        }
        self.start_retained_save(chunk, &mut state);
        false
    }

    fn start_retained_save(self: &Arc<Self>, chunk: &SyncChunk, state: &mut ChunkLifecycle) {
        let pos = Vector2::new(chunk.x, chunk.z);
        state.set_quiescing(true);
        let completion = Arc::new(AtomicU8::new(SAVING));
        let entity = self.get_entity_chunk_sync(&pos);
        let has_entity = entity.is_some();
        state.pending = Some(PendingUnload {
            generation: state.generation,
            revision: chunk.dirty_version().unwrap_or_default(),
            entity: entity.as_ref().map(|entity| {
                let revision = entity.dirty_version().unwrap_or_default();
                (entity.clone(), revision)
            }),
            completion: completion.clone(),
        });
        // IOWorker.store: register synchronously before handing the retained object to I/O.
        self.chunk_saver
            .queue_chunks(&self.level_folder, vec![(pos, chunk.clone())]);
        if let Some(entity) = entity {
            self.entity_saver
                .queue_chunks(&self.level_folder, vec![(pos, entity)]);
        }
        let level = self.clone();
        self.spawn_task(async move {
            let result = async {
                level
                    .chunk_saver
                    .flush_chunks(&level.level_folder, &[pos])
                    .await?;
                if has_entity {
                    level
                        .entity_saver
                        .flush_chunks(&level.level_folder, &[pos])
                        .await?;
                }
                Ok::<_, crate::chunk::ChunkWritingError>(())
            }
            .await;
            if let Err(error) = &result {
                tracing::error!(?pos, %error, "Chunk unload save failed");
            }
            completion.store(
                if result.is_ok() { SAVED } else { FAILED },
                Ordering::Release,
            );
        });
    }

    fn detach_saved_entity_chunk(&self, pos: Vector2<i32>) {
        let mut loads = self
            .entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.loaded_entity_chunks.remove(&pos);
        if let Some(load) = loads.get_mut(&pos) {
            load.epoch = load.epoch.wrapping_add(1);
        }
    }
}

/// ServerChunkCache.getChunkFutureMainThread temporary ticket, including cancelled futures.
pub(crate) struct FetchTicket<'a> {
    level: &'a Level,
    pos: Vector2<i32>,
    _mutation: ChunkMutation,
}

impl<'a> FetchTicket<'a> {
    pub(crate) fn new(level: &'a Level, pos: Vector2<i32>) -> Self {
        let mutation = level.begin_chunk_mutation(pos);
        let mut tickets = level
            .chunk_loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tickets.add_ticket(pos, ChunkLoading::FULL_CHUNK_LEVEL);
        tickets.send_change();
        Self {
            level,
            pos,
            _mutation: mutation,
        }
    }
}

impl Drop for FetchTicket<'_> {
    fn drop(&mut self) {
        let mut tickets = self
            .level
            .chunk_loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tickets.remove_ticket(self.pos, ChunkLoading::FULL_CHUNK_LEVEL);
        tickets.send_change();
    }
}

#[cfg(test)]
#[path = "chunk_lifecycle_tests.rs"]
mod tests;
