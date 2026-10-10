//! PersistentEntitySectionManager.removeSectionIfEmpty retires empty entity sections.
use std::sync::{Arc, Weak, atomic::AtomicBool, atomic::Ordering};

use dashmap::DashMap;
use pumpkin_util::math::vector2::Vector2;

use super::{ChunkAdmission, ChunkLifecycleCell};
use crate::level::{Level, SyncChunk, SyncEntityChunk};

type StateMap = DashMap<Vector2<i32>, Arc<ChunkLifecycleCell>>;

pub(super) struct AbsentAdmission {
    pos: Vector2<i32>,
    states: Weak<StateMap>,
    terrain: Weak<DashMap<Vector2<i32>, SyncChunk>>,
    entities: Weak<DashMap<Vector2<i32>, SyncEntityChunk>>,
    pub(super) resident: AtomicBool,
}

impl AbsentAdmission {
    fn is_resident(&self) -> bool {
        self.terrain
            .upgrade()
            .is_some_and(|chunks| chunks.contains_key(&self.pos))
            || self
                .entities
                .upgrade()
                .is_some_and(|chunks| chunks.contains_key(&self.pos))
    }
}

impl ChunkAdmission {
    pub(super) fn retire_absent(&self) {
        let Some(retirement) = self.retirement.get() else {
            return;
        };
        // ServerChunkCache.tick's read barrier protects resident writes after a fetch publishes.
        let resident = retirement.is_resident();
        retirement.resident.store(resident, Ordering::Release);
        if resident {
            return;
        }
        let Some(states) = retirement.states.upgrade() else {
            return;
        };
        states.remove_if(&retirement.pos, |_, cell| {
            // Lookup and removal share the map shard: a concurrent load must retain this cell
            // or create a new generation only after all counted writers have finished.
            if Arc::strong_count(cell) != 1 || !std::ptr::eq(cell.admission.as_ref(), self) {
                return false;
            }
            cell.try_lock().is_ok_and(|state| {
                state.watchers == 0
                    && state.mutations() == 0
                    && !state.quiescing
                    && !state.live_block_entities
                    && state.pending.is_none()
                    && !retirement.is_resident()
            })
        });
    }
}

impl Level {
    /// Retains canonical mutation admission, arranging idle retirement for absent positions.
    /// Cached entity admission must use this lookup before retaining the returned cell.
    #[must_use]
    pub fn chunk_mutation_cell(&self, pos: Vector2<i32>) -> Arc<ChunkLifecycleCell> {
        let cell = self.chunk_lifecycles.at(pos);
        if !self.is_chunk_loaded(&pos) && self.get_entity_chunk_sync(&pos).is_none() {
            let retirement = cell.admission.retirement.get_or_init(|| AbsentAdmission {
                pos,
                states: Arc::downgrade(&self.chunk_lifecycles.states),
                terrain: Arc::downgrade(&self.loaded_chunks),
                entities: Arc::downgrade(&self.loaded_entity_chunks),
                resident: AtomicBool::new(false),
            });
            retirement.resident.store(false, Ordering::Release);
        } else if let Some(retirement) = cell.admission.retirement.get() {
            retirement.resident.store(true, Ordering::Release);
        }
        cell
    }
}
