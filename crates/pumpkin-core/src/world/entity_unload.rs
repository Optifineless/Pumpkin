//! Tick-boundary `PersistentEntitySectionManager.processPendingLoads/processUnloads`.

use std::sync::{Arc, Weak};

use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::level::{EntityUnload, SyncEntityChunk};

use super::{World, entity_persistence, entity_snapshot::EntityChunkSnapshot};
use crate::entity::player::Player;

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
        let snapshot = EntityChunkSnapshot::prepare(self, request.position);
        let members = snapshot.members.clone();
        self.save_block_entities(request.position);

        #[cfg(test)]
        let hook = self.before_entity_unload_commit.lock().unwrap().take();
        #[cfg(test)]
        if let Some(hook) = hook {
            hook();
        }

        if !snapshot.commit(self, request, true) {
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
