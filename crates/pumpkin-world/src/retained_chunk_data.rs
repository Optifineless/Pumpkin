use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::vector2::Vector2;

use crate::level::{Level, SyncChunk, chunk_lifecycle::ChunkMutation};

/// The handle no longer denotes an admitted canonical chunk.
#[derive(Debug, thiserror::Error)]
#[error("Chunk is unloaded or quiescing")]
pub struct ChunkMutationRejected;

impl Level {
    /// Returns the canonical chunk and its permit, cancelling any obsolete unload.
    pub fn try_mutate_chunk_at(&self, pos: Vector2<i32>) -> Option<(SyncChunk, ChunkMutation)> {
        let chunk = self.read_chunk_sync(&pos, Clone::clone)?;
        let permit = self.try_mutate_retained_chunk(&chunk)?;
        Some((chunk, permit))
    }

    /// Writes custom data only while this exact retained chunk is still canonical and admitted.
    pub fn set_retained_chunk_custom_data(
        &self,
        chunk: &SyncChunk,
        namespace: &str,
        key: &str,
        value: NbtTag,
    ) -> Result<(), ChunkMutationRejected> {
        // ServerChunkCache.getChunk / ChunkMap.scheduleUnload serialize access with unload.
        let _permit = self
            .try_mutate_retained_chunk(chunk)
            .ok_or(ChunkMutationRejected)?;
        chunk.set_custom_data(namespace, key, value);
        Ok(())
    }

    /// Removes custom data under canonical retained-chunk admission; stale handles are errors.
    pub fn remove_retained_chunk_custom_data(
        &self,
        chunk: &SyncChunk,
        namespace: &str,
        key: &str,
    ) -> Result<(), ChunkMutationRejected> {
        let _permit = self
            .try_mutate_retained_chunk(chunk)
            .ok_or(ChunkMutationRejected)?;
        chunk.remove_custom_data(namespace, key);
        Ok(())
    }
}
