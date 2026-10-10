//! Cached ownership must not retain sections after Callback.onMove/removeSectionIfEmpty.
use std::sync::{Arc, Weak, atomic::AtomicBool, atomic::Ordering};

use super::{ChunkAdmission, ChunkLifecycleCell, ChunkMutation};
use crate::level::Level;

const INITIAL_CACHE_PURGE_THRESHOLD: usize = 32;

/// A cached canonical cell that relinquishes ownership when its chunk starts quiescing.
pub struct ChunkAdmissionCache {
    cell: arc_swap::ArcSwapOption<ChunkLifecycleCell>,
    admission: Arc<ChunkAdmission>,
    live: AtomicBool,
}

impl ChunkAdmissionCache {
    /// Retains canonical ownership for this read; an empty cache requires a fresh level lookup.
    #[must_use]
    pub fn load(&self) -> arc_swap::Guard<Option<Arc<ChunkLifecycleCell>>> {
        self.cell.load()
    }

    pub(super) fn clear(&self) {
        self.live.store(false, Ordering::SeqCst);
        self.cell.store(None);
    }

    pub(super) fn try_admit(&self, tick: bool) -> Option<ChunkMutation> {
        let permit = self.admission.try_admit(tick)?;
        // Quiescence clears live before releasing the cell. A counted permit prevents retirement;
        // tick permits retain the read barrier. This final SeqCst check also rejects cleared caches
        // after an abort/reopen, so an old cache can never admit through a retired cell (no ABA).
        self.live.load(Ordering::SeqCst).then_some(permit)
    }
}

impl Level {
    /// Admits through a registered cache; a cleared slot requires a fresh canonical lookup.
    /// The cache must belong to this level. Never hold any lifecycle mutex while admitting.
    /// Tick-scoped callers must retain the level's tick read barrier through the permit.
    pub fn admit_cached_chunk_mutation(
        &self,
        cache: &ChunkAdmissionCache,
    ) -> Option<ChunkMutation> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.chunk_lifecycles.benchmark_admission_disabled() {
            return Some(ChunkMutation(None));
        }
        cache.try_admit(self.has_tick_mutation_scope())
    }
}

impl ChunkLifecycleCell {
    /// Registers cached ownership under the lifecycle lock, so quiescence can release it.
    /// Admission must be acquired first; an already quiescing cell is never cached.
    /// Never admit an entity while holding any lifecycle mutex.
    #[must_use]
    pub fn cache(self: &Arc<Self>) -> Arc<ChunkAdmissionCache> {
        let mut state = self
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache = Arc::new(ChunkAdmissionCache {
            cell: arc_swap::ArcSwapOption::empty(),
            admission: self.admission.clone(),
            live: AtomicBool::new(false),
        });
        if !state.quiescing {
            cache.cell.store(Some(self.clone()));
            cache.live.store(true, Ordering::SeqCst);
            // Registration must serialize with cache clearing; only scan after the list doubles.
            if state.caches.len() >= state.cache_purge_at {
                state.caches.retain(|cached| cached.strong_count() != 0);
                state.cache_purge_at = state
                    .caches
                    .len()
                    .saturating_mul(2)
                    .max(INITIAL_CACHE_PURGE_THRESHOLD);
            }
            state.caches.push(Arc::downgrade(&cache));
        }
        cache
    }
}

pub(super) fn clear_caches(caches: &mut Vec<Weak<ChunkAdmissionCache>>) {
    for cache in caches.drain(..).filter_map(|cache| cache.upgrade()) {
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::chunk_lifecycle::ChunkLifecycles;
    use pumpkin_util::math::vector2::Vector2;

    #[test]
    fn quiescence_releases_cached_cells_but_preserves_active_readers() {
        let lifecycles = ChunkLifecycles::default();
        let pos = Vector2::new(0, 0);
        let cell = lifecycles.at(pos);
        let cache = cell.cache();
        drop(cell);
        let reader = cache.load();
        {
            let mut state = reader.as_ref().unwrap().lock().unwrap();
            state.set_quiescing(true);
            assert!(cache.load().is_none());
            // Model successful detach, which reopens the flag before retiring the holder.
            state.set_quiescing(false);
        };
        lifecycles.retire(pos);
        assert!(
            lifecycles.get(pos).is_some(),
            "an active reader must still pin its canonical cell"
        );
        drop(reader);
        lifecycles.retire(pos);
        assert!(lifecycles.get(pos).is_none());
    }

    #[test]
    fn cleared_cache_cannot_admit_after_reopen_and_retirement() {
        let lifecycles = ChunkLifecycles::default();
        let pos = Vector2::new(0, 0);
        let cell = lifecycles.at(pos);
        let cache = cell.cache();
        let permit = cache.try_admit(false).unwrap();
        {
            let mut state = cell.lock().unwrap();
            state.set_quiescing(true);
            state.set_quiescing(false);
        };
        drop(cell);
        lifecycles.retire(pos);
        assert!(
            lifecycles.get(pos).is_some(),
            "a counted cached reader pins retirement"
        );
        drop(permit);
        lifecycles.retire(pos);
        assert!(lifecycles.get(pos).is_none());
        let replacement = lifecycles.at(pos);
        assert!(cache.try_admit(false).is_none());
        assert_eq!(replacement.lock().unwrap().mutations(), 0);
    }

    #[test]
    fn cache_registration_amortizes_dead_entry_purges() {
        let cell = Arc::new(ChunkLifecycleCell::default());
        for _ in 0..100 {
            drop(cell.cache());
        }
        let state = cell.lock().unwrap();
        assert!(
            state.caches.len() > 1,
            "dead registrations should not be scanned on every miss"
        );
        assert!(
            state.caches.len() < 100,
            "dead registrations must eventually be purged"
        );
    }
}
