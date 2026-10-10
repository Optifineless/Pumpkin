use std::{
    collections::HashSet,
    sync::{Arc, atomic::Ordering},
};

use crate::entity::EntityBase;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::{chunk::io::Dirtiable, level::SyncEntityChunk};

pub(super) use super::chunk_unload_drain::ChunkUnloadRequests;
use super::{World, chunk_unload_drain::entity_address, entity_persistence, merge_entity_records};

pub(super) struct EntityUnloadSnapshot {
    generation: u64,
    members: Vec<Arc<dyn EntityBase>>,
    records: Vec<NbtCompound>,
}

// PersistentEntitySectionManager.processUnloads keeps unsuccessful unloads for later retry.
struct EntityCleanup {
    world: Arc<World>,
    pos: Vector2<i32>,
    generation: Option<u64>,
    completed: bool,
}

impl Drop for EntityCleanup {
    fn drop(&mut self) {
        if let Some(generation) = self.generation {
            let lifecycle = self.world.level.chunk_lifecycles.at(self.pos);
            let mut state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !self.completed && state.generation == generation {
                state.set_quiescing(false);
            }
            self.world.cancel_entity_unload(self.pos, generation);
        }
        if !self.completed {
            // clean_memory retains live entity chunks for retry, including invalidated saves.
            self.world
                .level
                .should_unload
                .store(true, Ordering::Release);
        }
    }
}

fn root_records(members: &[Arc<dyn EntityBase>]) -> Vec<NbtCompound> {
    members
        .iter()
        .filter(|entity| {
            !entity.get_entity().is_removed() && entity.get_entity().get_vehicle().is_none()
        })
        .map(entity_persistence::save_riding_tree)
        .collect()
}

impl World {
    pub(super) fn hold_tick_chunks(&self) -> std::sync::RwLockReadGuard<'_, ()> {
        // Admission spans player, entity and block-entity phases, including cross-chunk hoppers.
        self.chunk_lifecycle_tick
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn snapshot_unloading_entities(&self, pos: Vector2<i32>, generation: u64) -> bool {
        let Some(selected) = self.unload_members(pos, generation) else {
            return false;
        };
        // Admission follows entity identities when an outstanding writer moves chunks.
        if selected
            .iter()
            .any(|entity| entity.get_entity().has_admitted_mutations())
        {
            return false;
        }
        let Some(chunk) = self.level.get_entity_chunk_sync(&pos) else {
            if !selected.is_empty() {
                return false;
            }
            self.unloading_entities.insert(
                pos,
                EntityUnloadSnapshot {
                    generation,
                    members: selected,
                    records: Vec::new(),
                },
            );
            return true;
        };
        // A riding tree crossing a watched chunk must remain attached to its live passenger.
        if selected.iter().any(|entity| {
            let member_pos = entity.get_entity().chunk_pos.load();
            let mut root = entity.clone();
            while let Some(vehicle) = root.get_entity().get_vehicle() {
                root = vehicle;
            }
            root.get_player().is_some()
                || (member_pos != pos && self.level.is_chunk_watched(&member_pos))
        }) {
            return false;
        }
        let records = root_records(&selected);
        let mut data = chunk
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        merge_entity_records(
            &mut data,
            chunk.live.load(Ordering::Acquire),
            records.clone(),
        );
        chunk.mark_dirty(true);
        self.unloading_entities.insert(
            pos,
            EntityUnloadSnapshot {
                generation,
                members: selected,
                records,
            },
        );
        true
    }

    pub(super) fn cancel_entity_unload(&self, pos: Vector2<i32>, generation: u64) {
        self.unloading_entities
            .remove_if(&pos, |_, snapshot| snapshot.generation == generation);
    }

    pub(super) fn detach_unloading_entities(&self, pos: Vector2<i32>, generation: u64) -> bool {
        // PersistentEntitySectionManager.storeChunkSections applies unload to the SAME saved roots.
        let Some(snapshot) = self.unloading_entities.get(&pos) else {
            return false;
        };
        if snapshot.generation != generation {
            return false;
        }
        if snapshot
            .members
            .iter()
            .any(|entity| entity.get_entity().has_admitted_mutations())
        {
            return false;
        }
        let Some(current_members) = self.unload_members(pos, generation) else {
            return false;
        };
        let live: HashSet<_> = current_members.iter().map(entity_address).collect();
        if current_members.len() != snapshot.members.len()
            || snapshot
                .members
                .iter()
                .any(|saved| !live.contains(&entity_address(saved)))
            || root_records(&snapshot.members) != snapshot.records
        {
            return false;
        }
        let removed = snapshot.members.clone();
        drop(snapshot);
        // Mutators hold admission counters; completion owns the lifecycle and tick barrier.
        self.remove_unloaded_members(&removed);
        for entity in &removed {
            entity
                .get_entity()
                .removal_reason
                .store(Some(crate::entity::RemovalReason::UnloadedToChunk));
            entity.get_entity().removed.store(true, Ordering::Release);
            self.entity_tracker.remove_entity(entity.as_ref(), self);
            self.spawn_state.load().remove_entity(self, entity.as_ref());
        }
        entity_persistence::detach_unloaded_trees(&removed);
        self.cancel_entity_unload(pos, generation);
        true
    }

    pub async fn remove_entities_in_chunks(
        self: &Arc<Self>,
        chunks: impl IntoIterator<Item = impl std::borrow::Borrow<Vector2<i32>>>,
    ) {
        // Cleanup lists can be stale after an await. Only the scheduler's owned generation
        // may snapshot and detach; a rewatch keeps the same live objects in the meantime.
        for pos in chunks {
            let pos = *pos.borrow();
            if self.level.is_chunk_loaded(&pos) {
                self.level.should_unload.store(true, Ordering::Release);
            } else {
                self.clean_unpublished_entity_chunk(pos).await;
            }
        }
    }

    async fn clean_unpublished_entity_chunk(self: &Arc<Self>, pos: Vector2<i32>) {
        let Some(chunk) = self.level.get_entity_chunk_sync(&pos) else {
            return;
        };
        let Some(_mutation) = self.level.try_chunk_mutation(pos) else {
            self.level.should_unload.store(true, Ordering::Release);
            return;
        };
        let mut cleanup = EntityCleanup {
            world: self.clone(),
            pos,
            generation: None,
            completed: false,
        };
        if let Ok(generation) = self.prepare_unpublished_entity_unload(pos) {
            cleanup.generation = generation;
            cleanup.completed = generation.is_none();
        } else {
            let weak = Arc::downgrade(self);
            let Some(prepared) = self
                .at_chunk_tick_boundary(pos, move |_| {
                    let world = weak.upgrade()?;
                    let generation = world.prepare_unpublished_entity_unload(pos).ok()?;
                    // An unread boundary reply must release its prepared snapshot on cancellation.
                    Some(EntityCleanup {
                        world,
                        pos,
                        generation,
                        completed: generation.is_none(),
                    })
                })
                .await
                .flatten()
            else {
                return;
            };
            cleanup.completed = true;
            cleanup = prepared;
        }
        let Some(generation) = cleanup.generation else {
            return;
        };
        let revision = chunk.dirty.version();
        if let Err(error) = self.level.save_retained_entity_chunk(chunk.clone()).await {
            tracing::error!(?pos, %error, "Entity unload save failed");
            return;
        }
        cleanup.completed =
            match self.finish_unpublished_entity_unload(pos, &chunk, generation, revision) {
                Ok(done) => done,
                Err(()) => self
                    .at_chunk_tick_boundary(pos, move |world| {
                        world
                            .finish_unpublished_entity_unload(pos, &chunk, generation, revision)
                            .unwrap_or(false)
                    })
                    .await
                    .unwrap_or(false),
            };
    }

    fn prepare_unpublished_entity_unload(&self, pos: Vector2<i32>) -> Result<Option<u64>, ()> {
        let lifecycle = self.level.chunk_lifecycles.at(pos);
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if (state.quiescing && !self.owns_queued_unload(pos, state.generation))
            || state.watchers != 0
        {
            return Ok(None);
        }
        state.set_quiescing(true);
        if state.mutations() != 1 {
            state.set_quiescing(false);
            return Err(());
        }
        if !self.level.chunk_lifecycles.can_begin_unload(pos) {
            state.set_quiescing(false);
            return Ok(None);
        }
        let Ok(_ticks) = self.chunk_lifecycle_tick.try_write() else {
            state.set_quiescing(false);
            return Err(());
        };
        if !self.snapshot_unloading_entities(pos, state.generation) {
            state.set_quiescing(false);
            return Ok(None);
        }
        Ok(Some(state.generation))
    }

    fn finish_unpublished_entity_unload(
        &self,
        pos: Vector2<i32>,
        chunk: &SyncEntityChunk,
        generation: u64,
        revision: u64,
    ) -> Result<bool, ()> {
        let lifecycle = self.level.chunk_lifecycles.at(pos);
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.generation != generation || !chunk.dirty.is_published(revision) {
            return Ok(false);
        }
        let Ok(_ticks) = self.chunk_lifecycle_tick.try_write() else {
            return Err(());
        };
        if !self.detach_unloading_entities(pos, generation) {
            return Ok(false);
        }
        state.set_quiescing(false);
        chunk.live.store(false, Ordering::Release);
        Ok(true)
    }

    pub(super) fn activate_chunk_entities(
        self: &Arc<Self>,
        chunk: &SyncEntityChunk,
        player: Option<&Arc<crate::entity::player::Player>>,
    ) {
        let pos = Vector2::new(chunk.x, chunk.z);
        let Some(_mutation) = self.level.try_chunk_mutation(pos) else {
            return;
        };
        let lifecycle = self.level.chunk_lifecycles.at(pos);
        {
            let state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.watchers == 0
                || !self
                    .level
                    .get_entity_chunk_sync(&pos)
                    .is_some_and(|current| Arc::ptr_eq(&current, chunk))
            {
                return;
            }
        }
        // PersistentEntitySectionManager.processPendingLoads: IO cache hints are not authority.
        self.make_chunk_entities_live(chunk, player);
    }
}
