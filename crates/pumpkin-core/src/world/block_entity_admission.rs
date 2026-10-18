//! Admission remains closed for a retired terrain incarnation until activation or reload.

use std::sync::{Arc, Weak};

use pumpkin_util::math::{position::BlockPos, vector2::Vector2};

use super::{BlockEntityMutationPermit, BlockEntityUnload, PendingBlockEntityLoads, World};

impl World {
    /// Internal host bridge reserving a mutation before the final chunk capture.
    #[doc(hidden)]
    pub fn block_entity_mutation_permit(
        &self,
        pos: BlockPos,
    ) -> Option<BlockEntityMutationPermit<'_>> {
        let chunk_pos = pos.chunk_position();
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.stop_admission || !self.block_entity_terrain_admits_mutation(&mut loads, chunk_pos)
        {
            return None;
        }
        *loads.active_mutations.entry(chunk_pos).or_default() += 1;
        Some(BlockEntityMutationPermit {
            world: self,
            chunk_pos,
        })
    }

    pub(in crate::world) fn begin_block_entity_unload(&self, chunk_pos: Vector2<i32>) -> bool {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.active_mutations.contains_key(&chunk_pos) {
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
                completed: false,
                terrain: terrain.clone(),
            });
        if unloading.captures == 0 && !Weak::ptr_eq(&unloading.terrain, &terrain) {
            unloading.terrain = terrain;
            unloading.completed = false;
        }
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

    pub(super) fn block_entity_terrain_admits_mutation(
        &self,
        loads: &mut PendingBlockEntityLoads,
        chunk_pos: Vector2<i32>,
    ) -> bool {
        let Some(unloading) = loads.unloading.get(&chunk_pos) else {
            return true;
        };
        if unloading.captures != 0 {
            return false;
        }
        let reloaded = self
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .is_some_and(|chunk| !Weak::ptr_eq(&unloading.terrain, &Arc::downgrade(chunk.value())));
        if reloaded {
            loads.unloading.remove(&chunk_pos);
        }
        reloaded
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
                .get(&chunk_pos)
                .is_some_and(|unloading| unloading.captures != 0)
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
