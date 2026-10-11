//! ChunkMap.processUnloads leaves skipped work live until the next budgeted drain.
use pumpkin_util::math::vector2::Vector2;

use crate::level::Level;

impl Level {
    /// Closes a queued position before a shared root index is built. Active writers
    /// are checked again by poll; later writers invalidate the captured generation.
    /// `owned_mutations` counts only the caller's retained entity-cleanup permits.
    pub fn close_queued_chunk_admission(
        &self,
        pos: Vector2<i32>,
        owned_mutations: usize,
    ) -> Option<u64> {
        let cell = self.chunk_lifecycles.at(pos);
        let mut state = cell
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.watchers != 0 {
            return Some(state.generation);
        }
        let was_quiescing = state.quiescing;
        state.set_quiescing(true);
        if state.mutations() > owned_mutations {
            state.set_quiescing(was_quiescing);
            return None;
        }
        Some(state.generation)
    }

    /// Releases pre-index admission skipped by the tick budget, preserving any retained save.
    pub fn reopen_queued_chunk_admission(&self, pos: Vector2<i32>, generation: u64) {
        if let Some(cell) = self.chunk_lifecycles.get(pos) {
            let mut state = cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.generation == generation
                && state
                    .pending
                    .as_ref()
                    .is_none_or(|pending| pending.generation != generation)
            {
                // Discard the shared root index's generation before admitting new writers.
                state.invalidate();
            }
        }
    }
}
