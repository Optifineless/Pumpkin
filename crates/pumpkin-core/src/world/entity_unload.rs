//! Tick-boundary PersistentEntitySectionManager.processPendingLoads/processUnloads.
use std::sync::{Arc, PoisonError, Weak, atomic::Ordering};

use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::level::{EntityUnload, SyncEntityChunk};
use rustc_hash::FxHashSet;

use super::{World, entity_persistence, guard_effects::GuardEffects, merge_entity_records};
use crate::entity::{RemovalReason, player::Player};

#[cfg(test)]
#[path = "entity_unload_recapture_tests.rs"]
mod recapture_tests;
#[cfg(test)]
#[path = "entity_unload_terrain_tests.rs"]
mod terrain_tests;
#[cfg(test)]
#[path = "entity_unload_tests.rs"]
mod tests;

pub(super) struct EntityActivation {
    storage: SyncEntityChunk,
    player: Option<Weak<Player>>,
}

#[cfg(test)]
pub struct RecaptureTestHook {
    pub(crate) scheduled: std::sync::atomic::AtomicUsize,
    pub(crate) resume: tokio::sync::Semaphore,
    pub(crate) before_claim: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub(crate) completed: std::sync::mpsc::Sender<()>,
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
        let mut deferred = Vec::new();
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
            if !self.make_chunk_entities_live(&activation.storage, player.as_ref()) {
                deferred.push(activation);
            }
        }
        for activation in deferred {
            self.pending_entity_activations.push(activation);
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
            // Reopened terrain can spawn roots after its previous entity storage was cleaned.
            // Request FRESH storage through the existing async unload path, then retry next tick.
            #[cfg(test)]
            let hook = self
                .persistence_test_hooks
                .entity_recapture
                .lock()
                .unwrap()
                .clone();
            #[cfg(test)]
            if let Some(hook) = &hook {
                let before_claim = hook.before_claim.lock().unwrap().take();
                if let Some(before_claim) = before_claim {
                    before_claim();
                }
            }
            if self.level.begin_entity_storage_recapture(request) {
                let world = self.clone();
                let request = request.clone();
                #[cfg(test)]
                if let Some(hook) = &hook {
                    hook.scheduled.fetch_add(1, Ordering::SeqCst);
                }
                self.level.spawn_task(async move {
                    #[cfg(test)]
                    if let Some(hook) = &hook {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            hook.resume.acquire(),
                        )
                        .await
                        .unwrap()
                        .unwrap()
                        .forget();
                    }
                    if let Err(error) = world.level.get_entity_chunk(request.position).await {
                        tracing::error!(
                            "Entity unload cannot load chunk {:?}: {error}",
                            request.position
                        );
                    }
                    world.level.finish_entity_storage_recapture(&request);
                    #[cfg(test)]
                    if let Some(hook) = hook {
                        let _ = hook.completed.send(());
                    }
                });
            }
            return;
        }
        let ids: FxHashSet<_> = members
            .iter()
            .map(|entity| entity.get_entity().entity_id)
            .collect();
        let remaining = (!members.is_empty()).then(|| {
            Arc::new(
                current
                    .iter()
                    .filter(|entity| !ids.contains(&entity.get_entity().entity_id))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        });
        let bases: Vec<_> = members.iter().map(|entity| entity.get_entity()).collect();
        let effects = GuardEffects::new();
        // Entry checks cannot exclude a writer already admitted before the retirement mark.
        if current.iter().any(|entity| {
            ids.contains(&entity.get_entity().entity_id)
                != (entity_persistence::root_chunk(entity) == request.position)
        }) {
            return;
        }
        // Close placement and replacement admission before the last LevelChunk save image.
        if !self.begin_block_entity_unload(request.position) {
            return;
        }
        let Some(was_live) = self.commit_entity_retirement(request, &current, remaining, &bases)
        else {
            self.finish_block_entity_unload(request.position);
            return;
        };
        for member in &members {
            super::container_lifecycle::retire_entity_container(member);
        }
        Self::store_retired_entities(request, &members, was_live);
        // Collect then dispatch: no admission/storage guard reaches these object calls.
        for member in &members {
            self.spawn_state.load().remove_entity(self, member.as_ref());
        }
        entity_persistence::detach_unloaded_trees(&members);
        for member in &members {
            self.entity_tracker.remove_entity(member.as_ref(), self);
        }
        self.finish_block_entity_unload_capture(request.position);
        self.level.finish_entity_unload(request);
        self.level.clean_entity_chunks([request.position]);
        drop(effects);
    }

    // PersistentEntitySectionManager.unloadEntity, adapted to irrevocable mark-before-save.
    #[expect(
        clippy::rc_buffer,
        reason = "Retirement must compare the existing ArcSwap<Vec<...>> registry incarnation"
    )]
    fn commit_entity_retirement(
        &self,
        request: &EntityUnload,
        current: &Arc<Vec<Arc<dyn crate::entity::EntityBase>>>,
        remaining: Option<Arc<Vec<Arc<dyn crate::entity::EntityBase>>>>,
        bases: &[&crate::entity::Entity],
    ) -> Option<bool> {
        let mut was_live = false;
        let committed = self.level.commit_entity_unload(request, || {
            if let Some(remaining) = remaining {
                let previous = self.entities.compare_and_swap(current, remaining);
                if !Arc::ptr_eq(&previous, current) {
                    return false;
                }
            } else if !Arc::ptr_eq(&self.entities.load_full(), current) {
                return false;
            }
            // Entity.setRemoved preserves the first reason; this fork marks before serialization.
            for entity in bases {
                let _ = entity
                    .removal_reason
                    .compare_exchange(None, Some(RemovalReason::UnloadedToChunk));
                entity.removed.store(true, Ordering::Release);
            }
            if let Some(storage) = &request.storage {
                let _data = storage.data.lock().unwrap_or_else(PoisonError::into_inner);
                was_live = storage.live.swap(false, Ordering::AcqRel);
                storage.dirty.store(true, Ordering::Relaxed);
            }
            true
        });
        committed.then_some(was_live)
    }

    fn store_retired_entities(
        request: &EntityUnload,
        members: &[Arc<dyn crate::entity::EntityBase>],
        was_live: bool,
    ) {
        // Virtual serialization must run outside admission, before releasing terrain/cleanup authority.
        let records: Vec<_> = members
            .iter()
            .filter(|entity| {
                entity_persistence::can_save_retired(entity.get_entity())
                    && entity.get_entity().get_vehicle().is_none()
            })
            .map(entity_persistence::save_retired_riding_tree)
            .collect();
        if let Some(storage) = &request.storage {
            let mut data = storage.data.lock().unwrap_or_else(PoisonError::into_inner);
            if was_live || !records.is_empty() {
                merge_entity_records(&mut data, was_live, records);
                storage.dirty.store(true, Ordering::Relaxed);
            }
        }
    }
}
