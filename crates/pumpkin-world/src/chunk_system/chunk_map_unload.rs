use super::{Chunk, ChunkHolder, ChunkPos, GenerationSchedule, LoadedChunkChange, Relaxed};
use dashmap::mapref::entry::Entry;
use std::sync::{Arc, PoisonError};

impl GenerationSchedule {
    /// Claims the exact terrain incarnation; cancellation never freezes its schedulers.
    pub(super) fn claim_terrain_unload(&self, pos: ChunkPos, holder: &ChunkHolder) -> bool {
        // ChunkMap.scheduleUnload: recheck retention and expected holder before saving/unloading.
        let admission = self
            .entity_unloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let loading = self
            .chunk_loading
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if admission.retains_terrain(pos)
            || loading.pos_level.contains_key(&pos)
            || holder.target_stage != super::StagedChunkEnum::None
            || holder.dependency_stage != super::StagedChunkEnum::None
        {
            return false;
        }

        // The entry guard fences admitted Level producers before either scheduler freezes.
        let entry = match self.public_chunk_map.entry(pos) {
            Entry::Occupied(entry) => {
                let Some(Chunk::Level(chunk)) = &holder.chunk else {
                    return false;
                };
                if !Arc::ptr_eq(entry.get(), chunk) {
                    return false;
                }
                Some(entry)
            }
            Entry::Vacant(_) => {
                if holder.public {
                    return false;
                }
                None
            }
        };

        // ChunkMap.save captures ticks before ServerLevel.unload. No cancellation after freeze.
        // Only nesting: admission 10 -> tickets 30 -> scheduler 31 -> observation 40.
        // Ticket writers never acquire a scheduler; collectors/producers never acquire tickets.
        if let Some(Chunk::Level(chunk)) = &holder.chunk {
            let game_time = self.game_time.load(Relaxed);
            chunk.block_ticks.freeze_for_unload(game_time);
            chunk.fluid_ticks.freeze_for_unload(game_time);
        }
        let removed = entry.is_some();
        if let Some(entry) = entry {
            entry.remove();
        }
        drop(loading);
        drop(admission);
        if removed {
            self.loaded_chunk_changes
                .push(LoadedChunkChange::Unloaded(pos));
        }
        true
    }
}
