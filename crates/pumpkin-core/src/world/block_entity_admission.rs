//! A write to resident terrain reopens its completed block-entity retirement.

use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use pumpkin_util::math::{position::BlockPos, vector2::Vector2};

use super::{BlockEntityMutationPermit, BlockEntityUnload, World};

impl World {
    /// Internal host bridge reserving a mutation before the final chunk capture.
    #[doc(hidden)]
    pub fn block_entity_mutation_permit(
        &self,
        pos: BlockPos,
    ) -> Option<BlockEntityMutationPermit<'_>> {
        // LevelChunk.setBlockState needs the old entity before changing terrain: a
        // pending chest must deliver its captured inventory to removal callbacks.
        let _ = self.get_block_entity(&pos);
        self.reserve_block_entity_mutation(pos)
    }

    pub(in crate::world) fn reserve_block_entity_mutation(
        &self,
        pos: BlockPos,
    ) -> Option<BlockEntityMutationPermit<'_>> {
        let chunk_pos = pos.chunk_position();
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let mut loads = self
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if loads.stop_admission {
                return None;
            }
            if loads
                .unloading
                .values()
                .any(|unloading| unloading.captures != 0)
            {
                // A nested physics callback may write another chunk. Admission is
                // world-wide until #224 moves retirement to the terrain lifetime.
                if loads.unloading.values().any(|unloading| {
                    unloading.captures != 0
                        && unloading.capture_thread == std::thread::current().id()
                }) || Instant::now() >= deadline
                {
                    return None;
                }
                drop(loads);
                // No owner or callback is retained during this bounded wait.
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            let reopening = if let Some(unloading) = loads.unloading.get(&chunk_pos) {
                let terrain = self.level.loaded_chunks.get(&chunk_pos)?.clone();
                if Weak::ptr_eq(&unloading.terrain, &Arc::downgrade(&terrain)) {
                    if !unloading.completed {
                        return None;
                    }
                    Some(terrain)
                } else {
                    loads.unloading.remove(&chunk_pos);
                    None
                }
            } else {
                None
            };
            *loads.active_mutations.entry(chunk_pos).or_default() += 1;
            let permit = BlockEntityMutationPermit {
                world: self,
                chunk_pos,
            };
            drop(loads);
            if let Some(terrain) = reopening {
                // The Level gate excludes terrain eviction; never nest its admission
                // owner with ours. Keep the completed marker until retention succeeds.
                if !self
                    .level
                    .request_block_entity_recapture(chunk_pos, &terrain)
                {
                    return None;
                }
                self.pending_block_entity_loads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .unloading
                    .remove(&chunk_pos);
            }
            return Some(permit);
        }
    }

    pub(in crate::world) fn begin_block_entity_unload(&self, chunk_pos: Vector2<i32>) -> bool {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !loads.active_mutations.is_empty() {
            return false;
        }
        let terrain = self
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .map(|chunk| Arc::downgrade(chunk.value()))
            .unwrap_or_default();
        let unloading = loads
            .unloading
            .entry(chunk_pos)
            .or_insert_with(|| BlockEntityUnload {
                captures: 0,
                capture_thread: std::thread::current().id(),
                completed: false,
                terrain: terrain.clone(),
            });
        if unloading.captures != 0 {
            return false;
        }
        if unloading.captures == 0 && !Weak::ptr_eq(&unloading.terrain, &terrain) {
            unloading.terrain = terrain;
            unloading.completed = false;
        }
        unloading.capture_thread = std::thread::current().id();
        unloading.captures += 1;
        loads
            .snapshots
            .retain(|pos, _| pos.chunk_position() != chunk_pos);
        true
    }

    pub(in crate::world) fn try_stop_block_entity_mutations(&self) -> bool {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !loads.active_mutations.is_empty() {
            return false;
        }
        // Closing under the same gate makes this atomic with the last permit admission.
        loads.stop_admission = true;
        true
    }

    pub(in crate::world) fn finish_block_entity_unload(&self, chunk_pos: Vector2<i32>) {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(unloading) = loads.unloading.get_mut(&chunk_pos) {
            unloading.captures -= 1;
            if unloading.captures == 0 && !unloading.completed {
                loads.unloading.remove(&chunk_pos);
            }
        }
    }

    pub(in crate::world) fn block_entity_activation_permit(
        &self,
        chunk_pos: Vector2<i32>,
    ) -> Option<BlockEntityMutationPermit<'_>> {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.stop_admission
            || loads
                .unloading
                .values()
                .any(|unloading| unloading.captures != 0)
        {
            return None;
        }
        *loads.active_mutations.entry(chunk_pos).or_default() += 1;
        Some(BlockEntityMutationPermit {
            world: self,
            chunk_pos,
        })
    }

    pub(in crate::world) fn reopen_block_entities_after_activation(&self, chunk_pos: Vector2<i32>) {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The activation permit excludes a final capture until all restored roots are live.
        loads.unloading.remove(&chunk_pos);
    }

    pub(in crate::world) fn prune_completed_block_entity_unloads(&self) {
        self.pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unloading
            .retain(|_, unloading| {
                unloading.captures != 0 || unloading.terrain.strong_count() != 0
            });
    }
}

#[cfg(test)]
#[path = "clone_move_admission_tests.rs"]
mod clone_move_admission_tests;
