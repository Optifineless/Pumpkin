use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use super::{ChunkLoading, ChunkPos};
use crate::chunk::ChunkReadingError;

/// Retains full-chunk tickets until dropped, including when an async load is cancelled.
/// Await chunk readiness separately before reading terrain.
pub struct ChunkResidency {
    loading: Arc<Mutex<ChunkLoading>>,
    chunks: Vec<ChunkPos>,
    positions: HashSet<ChunkPos>,
}

impl ChunkResidency {
    #[must_use]
    pub fn new(loading: Arc<Mutex<ChunkLoading>>) -> Self {
        Self {
            loading,
            chunks: Vec::new(),
            positions: HashSet::new(),
        }
    }

    /// Retains one ticket per position without holding a lock between operations.
    pub fn add(&mut self, chunk: ChunkPos) {
        self.add_all([chunk]);
    }

    /// Adds a deduplicated batch with one scheduler notification.
    pub fn add_all(&mut self, chunks: impl IntoIterator<Item = ChunkPos>) {
        // ServerChunkCache.addTicketWithRadius: retain terrain for the pending operation.
        let mut loading = self
            .loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        for chunk in chunks {
            if self.positions.insert(chunk) {
                loading.add_ticket(chunk, ChunkLoading::FULL_CHUNK_LEVEL);
                self.chunks.push(chunk);
                changed = true;
            }
        }
        if changed {
            loading.send_change();
        }
    }

    #[must_use]
    pub fn chunks(&self) -> &[ChunkPos] {
        &self.chunks
    }
}

impl Drop for ChunkResidency {
    fn drop(&mut self) {
        let mut loading = self
            .loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for chunk in &self.chunks {
            loading.remove_ticket(*chunk, ChunkLoading::FULL_CHUNK_LEVEL);
        }
        loading.send_change();
    }
}

// ServerChunkCache.getChunkFutureMainThread: register before requesting and recheck publication.
pub(crate) async fn fetch_chunk(
    level: &Arc<crate::level::Level>,
    pos: ChunkPos,
) -> Result<crate::level::SyncChunk, crate::chunk::ChunkReadingError> {
    #[cfg(test)]
    {
        let hook = level.before_chunk_listener.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }
    let mut listener = level.chunk_listener.listen(pos);

    let mut residency =
        crate::chunk_system::residency::ChunkResidency::new(level.chunk_loading.clone());
    residency.add(pos);

    // ServerChunkCache.getChunkFutureMainThread uses a persistent holder future.
    // Publication may precede listener registration, so recheck under our ticket.
    if let Some(chunk) = level.loaded_chunks.get(&pos) {
        return Ok(chunk.clone());
    }

    let chunk = tokio::select! {
        result = &mut listener.receiver => result.map_err(|error| error.to_string()).and_then(std::convert::identity),
        () = level.cancel_token.cancelled() => Err("Level is shutting down".to_owned()),
    };

    chunk.map_err(|error| ChunkReadingError::IoError(std::io::Error::other(error)))
}

#[cfg(test)]
mod tests {
    use std::{future::Future, task::Poll};

    use super::*;
    use crate::level::Level;

    #[tokio::test]
    async fn cancelled_chunk_fetch_releases_its_temporary_ticket() {
        let directory = tempfile::tempdir().unwrap();
        let level = Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            directory.path().into(),
            0,
            pumpkin_data::dimension::Dimension::OVERWORLD,
        );
        let pos = ChunkPos::new(0, 0);
        let mut fetch = Box::pin(level.get_or_fetch_chunk(pos, |_| ()));
        // Cancel at the actual fetch's await, before storage/generation completes.
        std::future::poll_fn(|cx| {
            assert!(fetch.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(
            level
                .chunk_loading
                .lock()
                .unwrap()
                .ticket
                .contains_key(&pos)
        );
        drop(fetch);
        let released = !level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .contains_key(&pos);
        level.shutdown().await.unwrap();
        assert!(released, "cancelled chunk fetch leaked its ticket");
    }
}
