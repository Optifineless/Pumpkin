//! Tick-boundary PersistentEntitySectionManager.processPendingLoads/processUnloads.
use std::sync::{Arc, PoisonError, Weak, atomic::Ordering};

use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::level::{EntityUnload, SyncEntityChunk};
use rustc_hash::FxHashSet;

use super::{World, entity_persistence, guard_effects::GuardEffects, merge_entity_records};
use crate::entity::{RemovalReason, player::Player};

pub(super) struct EntityActivation {
    storage: SyncEntityChunk,
    player: Option<Weak<Player>>,
}

impl World {
    pub(super) fn queue_entity_activation(
        &self,
        storage: &SyncEntityChunk,
        player: Option<&Arc<Player>>,
    ) {
        self.pending_entity_activations.push(EntityActivation {
            storage: storage.clone(),
            player: player.map(Arc::downgrade),
        });
    }

    /// Requests retirement at the next entity phase; stale positions cannot authorize removal.
    pub async fn remove_entities_in_chunks(
        &self,
        chunks: impl IntoIterator<Item = impl std::borrow::Borrow<Vector2<i32>>>,
    ) {
        let positions: Vec<_> = chunks.into_iter().map(|pos| *pos.borrow()).collect();
        let entities = self.entities.load_full();
        // PersistentEntitySectionManager.storeChunkSections requests FRESH storage before saving roots.
        for &pos in &positions {
            if !self.level.is_chunk_watched(&pos)
                && self.level.get_entity_chunk_sync(&pos).is_none()
                && entities
                    .iter()
                    .any(|entity| entity_persistence::root_chunk(entity) == pos)
                && let Err(error) = self.level.get_entity_chunk(pos).await
            {
                tracing::error!("Entity unload cannot load chunk {pos:?}: {error}");
            }
        }
        self.level.request_entity_unloads(positions);
    }

    pub(super) fn process_entity_chunk_lifecycle(self: &Arc<Self>) {
        while let Some(activation) = self.pending_entity_activations.pop() {
            let pos = Vector2::new(activation.storage.x, activation.storage.z);
            if !self.level.is_chunk_watched(&pos) {
                continue;
            }
            let player = activation.player.as_ref().and_then(Weak::upgrade);
            if activation.player.is_some()
                && player.as_ref().is_none_or(|player| {
                    player.client.closed() || !Arc::ptr_eq(&player.world(), self)
                })
            {
                continue;
            }
            self.make_chunk_entities_live(&activation.storage, player.as_ref());
        }
        for request in self.level.pending_entity_unloads() {
            self.retire_entity_chunk(&request);
        }
    }

    fn retire_entity_chunk(self: &Arc<Self>, request: &EntityUnload) {
        let current = self.entities.load_full();
        let members: Vec<_> = current
            .iter()
            .filter(|entity| entity_persistence::root_chunk(entity) == request.position)
            .cloned()
            .collect();
        if request.storage.is_none() && !members.is_empty() {
            return;
        }
        let records: Vec<_> = members
            .iter()
            .filter(|entity| {
                !entity.get_entity().is_removed() && entity.get_entity().get_vehicle().is_none()
            })
            .map(entity_persistence::save_riding_tree)
            .collect();
        // Keep block-entity saving independent of entity-sidecar availability (PR-2 owns detach atomicity).
        self.save_block_entities(request.position);
        let ids: FxHashSet<_> = members
            .iter()
            .map(|entity| entity.get_entity().entity_id)
            .collect();
        let remaining: Vec<_> = current
            .iter()
            .filter(|entity| !ids.contains(&entity.get_entity().entity_id))
            .cloned()
            .collect();
        let bases: Vec<_> = members.iter().map(|entity| entity.get_entity()).collect();
        let effects = GuardEffects::new();
        // Recheck complete membership before admission; Tokio-side mutation remains PR-2 work.
        if current.iter().any(|entity| {
            ids.contains(&entity.get_entity().entity_id)
                != (entity_persistence::root_chunk(entity) == request.position)
        }) {
            return;
        }
        if !self.level.commit_entity_unload(request, || {
            if !members.is_empty() {
                let previous = self
                    .entities
                    .compare_and_swap(&current, Arc::new(remaining));
                if !Arc::ptr_eq(&previous, &current) {
                    return false;
                }
            } else if !Arc::ptr_eq(&self.entities.load_full(), &current) {
                return false;
            }
            if let Some(storage) = &request.storage {
                let mut data = storage.data.lock().unwrap_or_else(PoisonError::into_inner);
                let live = storage.live.load(Ordering::Acquire);
                if live || !records.is_empty() {
                    merge_entity_records(&mut data, live, records);
                    storage.dirty.store(true, Ordering::Relaxed);
                }
                storage.live.store(false, Ordering::Release);
            }
            // PersistentEntitySectionManager.unloadEntity marks removed before tracker callbacks.
            for entity in &bases {
                if entity.removal_reason.load().is_none() {
                    entity
                        .removal_reason
                        .store(Some(RemovalReason::UnloadedToChunk));
                }
                entity.removed.store(true, Ordering::Release);
            }
            true
        }) {
            return;
        }
        // Collect then dispatch: no admission/storage guard reaches these object calls.
        for member in &members {
            self.spawn_state.load().remove_entity(self, member.as_ref());
        }
        entity_persistence::detach_unloaded_trees(&members);
        for member in &members {
            self.entity_tracker.remove_entity(member.as_ref(), self);
        }
        self.unload_block_entities(request.position);
        self.level.clean_entity_chunks([request.position]);
        drop(effects);
    }
}
