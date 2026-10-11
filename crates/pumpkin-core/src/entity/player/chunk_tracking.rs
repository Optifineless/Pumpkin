#[cfg(test)]
use super::chunk_tracking_tests;
use super::{CUnloadChunk, Cylindrical, Player};
use crate::{entity::EntityBase, world::World};
use pumpkin_util::math::vector2::Vector2;
use std::{
    num::NonZero,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering::Relaxed},
    },
};
use tokio::sync::oneshot;

#[derive(Default)]
pub struct ChunkTrackingState {
    stopped: AtomicBool,
    pending: Mutex<Option<oneshot::Receiver<()>>>,
}

impl Player {
    pub(super) fn remove_chunk_tickets(&self, level: &pumpkin_world::level::Level) {
        // ChunkMap.updatePlayerStatus(false); match move's held-tickets -> loading lock order.
        #[cfg(test)]
        crate::world::chunker::tests::before_ticket_cleanup(self.entity_id());
        let mut held = self
            .held_chunk_tickets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut loading = level
            .chunk_loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((view_level, sim_level)) = held.take() {
            let center = self.get_entity().chunk_pos.load();
            if let Some(view) = view_level {
                loading.remove_ticket(center, view);
            }
            if let Some(sim) = sim_level {
                loading.remove_ticket(center, sim);
            }
        }
        loading.send_change();
        level.should_unload.store(true, Relaxed);
        level.level_channel.notify();
    }

    pub(super) fn take_watched_section(&self) -> (Cylindrical, Option<oneshot::Receiver<()>>) {
        let _owner = self.living_entity.own_damage();
        self.chunk_tracking.stopped.store(true, Relaxed);
        // ChunkMap.applyChunkTrackingView(EMPTY); generated radius-one offsets are empty.
        let section = self
            .watched_section
            .swap(Cylindrical::new(Vector2::new(0, 0), NonZero::<u8>::MIN));
        let pending = self
            .chunk_tracking
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        (section, pending)
    }

    /// Reports whether EMPTY teardown has stopped movement from rebuilding the view.
    /// Hold player combat ownership when using this to commit a view.
    pub(crate) fn chunk_tracking_stopped(&self) -> bool {
        self.chunk_tracking.stopped.load(Relaxed)
    }

    pub(super) fn resume_chunk_tracking(&self) {
        let _owner = self.living_entity.own_damage();
        self.chunk_tracking.stopped.store(false, Relaxed);
    }

    /// Queues watcher deltas in view order; the caller must hold player combat ownership.
    // ChunkMap.applyChunkTrackingView commits deltas in order before EMPTY teardown.
    pub(crate) fn queue_chunk_watch_update(
        &self,
    ) -> (Option<oneshot::Receiver<()>>, oneshot::Sender<()>) {
        let (done, pending) = oneshot::channel();
        let previous = self
            .chunk_tracking
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(pending);
        (previous, done)
    }

    #[cfg(test)]
    pub(super) async fn await_chunk_watch_updates(&self) {
        let pending = self
            .chunk_tracking
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Self::await_chunk_watch_update(pending).await;
    }

    pub(super) async fn await_chunk_watch_update(pending: Option<oneshot::Receiver<()>>) {
        if let Some(pending) = pending {
            let _ = pending.await;
        }
    }

    pub async fn unload_watched_chunks(&self, world: &World) {
        // ChunkMap.applyChunkTrackingView(EMPTY) claims once before asynchronous cleanup.
        let (section, pending) = self.take_watched_section();
        let radial_chunks = section.all_chunks_within();
        if radial_chunks.len() == 0 {
            return;
        }
        Self::await_chunk_watch_update(pending).await;
        let level = &world.level;
        let chunks_to_clean = level.mark_chunks_as_not_watched(radial_chunks).await;
        #[cfg(test)]
        chunk_tracking_tests::pause_unload(self.entity_id()).await;
        if !chunks_to_clean.is_empty() {
            world.remove_entities_in_chunks(&chunks_to_clean).await;
            level.clean_entity_chunks(&chunks_to_clean);
        }
        for chunk in &chunks_to_clean {
            self.send_client_packet(&CUnloadChunk::new(chunk.x, chunk.y))
                .await;
        }
    }
}

#[cfg(test)]
pub async fn pause_pending_watch_update(entity_id: i32) {
    chunk_tracking_tests::pause_update(entity_id).await;
}
