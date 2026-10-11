//! Tick-boundary `PersistentEntitySectionManager.processPendingLoads/processUnloads`.

use std::sync::{Arc, Weak, atomic::Ordering};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::{
    chunk::io::Dirtiable,
    level::{EntityUnload, SyncEntityChunk},
};
use rustc_hash::FxHashSet;

use super::{World, entity_persistence, merge_entity_records};
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
        #[cfg(test)]
        if let Some(queued) = self.after_entity_activation_queued.lock().unwrap().take() {
            let _ = queued.send(());
        }
    }

    /// Requests retirement at the next world tick; a stale position list cannot remove live entities.
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
        // Vanilla integrates pending loads before considering hidden chunks for unloading.
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

    fn retire_entity_chunk(&self, request: &EntityUnload) {
        let current = self.entities.load_full();
        let members: Vec<_> = current
            .iter()
            .filter(|entity| entity_persistence::root_chunk(entity) == request.position)
            .cloned()
            .collect();
        let records: Vec<NbtCompound> = members
            .iter()
            .filter(|entity| {
                let base = entity.get_entity();
                !base.is_removed() && base.get_vehicle().is_none()
            })
            .map(entity_persistence::save_riding_tree)
            .collect();
        self.save_block_entities(request.position);

        #[cfg(test)]
        let hook = self.before_entity_unload_commit.lock().unwrap().take();
        #[cfg(test)]
        if let Some(hook) = hook {
            hook();
        }

        if !self.level.commit_entity_unload(request, || {
            let ids: FxHashSet<_> = members
                .iter()
                .map(|member| member.get_entity().entity_id)
                .collect();
            // Snapshot and retirement need the same complete membership, including riding roots.
            if current.iter().any(|entity| {
                ids.contains(&entity.get_entity().entity_id)
                    != (entity_persistence::root_chunk(entity) == request.position)
            }) {
                return false;
            }
            let remaining: Vec<_> = current
                .iter()
                .filter(|entity| !ids.contains(&entity.get_entity().entity_id))
                .cloned()
                .collect();
            let previous = self
                .entities
                .compare_and_swap(&current, Arc::new(remaining));
            if !Arc::ptr_eq(&previous, &current) {
                return false;
            }

            let mut data = request
                .storage
                .data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let live = request.storage.live.load(Ordering::Acquire);
            if live || !records.is_empty() {
                merge_entity_records(&mut data, live, records);
                request.storage.mark_dirty(true);
            }
            request.storage.live.store(false, Ordering::Release);
            for member in &members {
                let entity = member.get_entity();
                // Entity.setRemoved preserves an existing reason and removes before tracker callbacks.
                let _ = entity
                    .removal_reason
                    .compare_exchange(None, Some(RemovalReason::UnloadedToChunk));
                entity.removed.store(true, Ordering::Release);
            }
            true
        }) {
            return;
        }

        // No lifecycle/storage guard survives packet sends or plugin callbacks.
        for member in &members {
            self.spawn_state.load().remove_entity(self, member.as_ref());
        }
        // Entity.setRemoved detaches passengers before its tracking-removal callback.
        entity_persistence::detach_unloaded_trees(&members);
        for member in &members {
            self.entity_tracker.remove_entity(member.as_ref(), self);
        }
        self.block_entities.remove(&request.position);
        self.level.clean_entity_chunks([request.position]);
    }
}
