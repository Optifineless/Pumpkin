use super::{Arc, FileIO, Level, Ordering, Vector2, error};

impl Level {
    /// Publish watcher admission before releasing terrain tickets; returns only cleanup candidates.
    pub fn update_chunk_watchers(
        &self,
        watched: &[Vector2<i32>],
        unwatched: &[Vector2<i32>],
    ) -> Vec<Vector2<i32>> {
        for pos in watched {
            let lifecycle = self.chunk_lifecycles.at(*pos);
            let mut state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let old_generation = state.generation;
            state.invalidate();
            // PersistentEntitySectionManager.updateChunkStatus cancels obsolete unload work.
            if let Some(portal) = self.world_portal.load().as_ref() {
                portal.cancel_chunk_unload(*pos, old_generation);
            }
            state.watchers = state.watchers.saturating_add(1);
            self.chunk_watchers.insert(*pos, state.watchers);
        }
        let mut cleanup = Vec::new();
        for pos in unwatched {
            let lifecycle = self.chunk_lifecycles.at(*pos);
            let mut state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.watchers == 0 {
                continue;
            }
            state.watchers -= 1;
            if state.watchers == 0 {
                state.invalidate();
                self.chunk_watchers.remove(pos);
                cleanup.push(*pos);
            } else {
                self.chunk_watchers.insert(*pos, state.watchers);
            }
        }
        cleanup
    }

    /// Updates storage-region residency after the synchronous watcher admission transaction.
    pub async fn update_entity_region_watchers(
        &self,
        watched: &[Vector2<i32>],
        unwatched: &[Vector2<i32>],
    ) {
        self.entity_saver
            .watch_chunks(&self.level_folder, watched)
            .await;
        self.entity_saver
            .unwatch_chunks(&self.level_folder, unwatched)
            .await;
    }

    /// Rewatch cancels obsolete cleanup under the same ownership lock as unload completion.
    pub async fn mark_chunks_as_newly_watched(&self, chunks: &[Vector2<i32>]) {
        self.update_chunk_watchers(chunks, &[]);
        self.entity_saver
            .watch_chunks(&self.level_folder, chunks)
            .await;
    }

    pub async fn mark_chunks_as_not_watched(
        &self,
        chunks: impl IntoIterator<Item = impl std::borrow::Borrow<Vector2<i32>>>,
    ) -> Vec<Vector2<i32>> {
        let positions: Vec<_> = chunks.into_iter().map(|pos| *pos.borrow()).collect();
        let cleanup = self.update_chunk_watchers(&[], &positions);
        self.entity_saver
            .unwatch_chunks(&self.level_folder, &positions)
            .await;
        cleanup
    }

    /// Live entities are snapshotted and detached by the terrain lifecycle, never stale cleanup.
    pub fn clean_entity_chunks(
        self: &Arc<Self>,
        chunks: impl IntoIterator<Item = impl std::borrow::Borrow<Vector2<i32>>>,
    ) {
        let mut removed = Vec::new();
        for pos in chunks {
            let pos = *pos.borrow();
            let lifecycle = self.chunk_lifecycles.at(pos);
            let mut state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.watchers != 0 || state.quiescing {
                continue;
            }
            state.set_quiescing(true);
            if state.mutations() != 0 {
                state.set_quiescing(false);
                continue;
            }
            let mut loads = self
                .entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let chunk = self.loaded_entity_chunks.remove_if(&pos, |_, chunk| {
                if chunk.live.load(Ordering::Acquire) {
                    return false;
                }
                self.entity_saver
                    .queue_chunks(&self.level_folder, vec![(pos, chunk.clone())]);
                true
            });
            state.set_quiescing(false);
            if chunk.is_some() {
                if let Some(load) = loads.get_mut(&pos) {
                    load.epoch = load.epoch.wrapping_add(1);
                }
                removed.push(pos);
            }
        }
        if removed.is_empty() {
            return;
        }
        let level = self.clone();
        self.spawn_task(async move {
            if let Err(error) = level
                .entity_saver
                .flush_chunks(&level.level_folder, &removed)
                .await
            {
                error!(%error, "Entity unload save failed");
            }
        });
    }
}
