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

// ServerChunkCache.getChunkFutureMainThread retains its ticket through chunk readiness.
pub(crate) async fn fetch_chunk(
    level: &Arc<crate::level::Level>,
    pos: ChunkPos,
) -> Result<crate::level::SyncChunk, ChunkReadingError> {
    let recv = level.chunk_listener.add_single_chunk_listener(pos);
    let mut residency = ChunkResidency::new(level.chunk_loading.clone());
    residency.add(pos);
    // A chunk may have been published between the initial read and subscription.
    if let Some(chunk) = level.loaded_chunks.get(&pos) {
        return Ok(chunk.clone());
    }
    let chunk = tokio::select! {
        result = recv => result.map_err(|error| error.to_string()).and_then(std::convert::identity),
        () = level.cancel_token.cancelled() => Err("Level is shutting down".to_owned()),
    };
    chunk.map_err(|error| ChunkReadingError::IoError(std::io::Error::other(error)))
}

#[cfg(test)]
mod tests;
