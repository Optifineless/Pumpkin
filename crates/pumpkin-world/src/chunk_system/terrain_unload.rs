//! ChunkMap.scheduleUnload identity checks plus fork admission and ticket retention.
use std::sync::{Arc, PoisonError};

use super::{Chunk, ChunkHolder, ChunkPos, GenerationSchedule, LoadedChunkChange, StagedChunkEnum};

impl GenerationSchedule {
    pub(super) fn claim_terrain_unload(&self, pos: ChunkPos, holder: &ChunkHolder) -> bool {
        {
            let admission = self
                .entity_unloads
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // ChunkMap.updateChunkScheduling rescinds unloads after new tickets. In the fork,
            // admission 10 -> tickets 30 spans both retention checks and exact-Arc removal.
            let loading = self
                .chunk_loading
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // The scheduler exclusively owns this holder; recheck its effective target and published Arc.
            if admission.retains_terrain(pos)
                || loading.pos_level.contains_key(&pos)
                || holder.target_stage != StagedChunkEnum::None
                || holder.dependency_stage != StagedChunkEnum::None
            {
                return false;
            }
            if !holder.public {
                return true;
            }
            let Some(Chunk::Level(expected)) = &holder.chunk else {
                return false;
            };
            let removed = self
                .public_chunk_map
                .remove_if(&pos, |_, current| Arc::ptr_eq(current, expected));
            if removed.is_none() {
                return false;
            }
        }
        self.loaded_chunk_changes
            .push(LoadedChunkChange::Unloaded(pos));
        true
    }
}
