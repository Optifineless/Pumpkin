use std::sync::{Arc, Mutex};

use super::{ChunkLoading, ChunkPos};

/// Retains full-chunk tickets until dropped, including when an async load is cancelled.
/// Await chunk readiness separately before reading terrain.
pub struct ChunkResidency {
    loading: Arc<Mutex<ChunkLoading>>,
    chunks: Vec<ChunkPos>,
}

impl ChunkResidency {
    #[must_use]
    pub const fn new(loading: Arc<Mutex<ChunkLoading>>) -> Self {
        Self {
            loading,
            chunks: Vec::new(),
        }
    }

    /// Retains one ticket per position without holding a lock between operations.
    pub fn add(&mut self, chunk: ChunkPos) {
        if self.chunks.contains(&chunk) {
            return;
        }
        // ServerChunkCache.addTicketWithRadius: retain terrain for the pending operation.
        let mut loading = self
            .loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loading.add_ticket(chunk, ChunkLoading::FULL_CHUNK_LEVEL);
        self.chunks.push(chunk);
        loading.send_change();
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
