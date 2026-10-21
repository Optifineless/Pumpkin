use std::sync::{Arc, TryLockError};

use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use super::{BlockPalette, ChunkData, ChunkSections};

impl ChunkData {
    /// Commits an observed terrain batch before any world callbacks run.
    /// Retry contended sections after releasing every guard, without a blocking lock-order edge.
    pub fn set_observed_blocks(
        chunks: &[Arc<Self>],
        updates: &[(BlockPos, BlockStateId, BlockStateId)],
    ) -> bool {
        if chunks.iter().enumerate().any(|(index, chunk)| {
            chunks[..index]
                .iter()
                .any(|other| Arc::ptr_eq(chunk, other))
        }) {
            return false;
        }
        let mut sections = 'acquire: loop {
            let mut guards = Vec::with_capacity(chunks.len());
            for chunk in chunks {
                let guard = match chunk.section.block_sections.try_write() {
                    Ok(guard) => guard,
                    Err(TryLockError::Poisoned(error)) => error.into_inner(),
                    Err(TryLockError::WouldBlock) => {
                        drop(guards);
                        std::thread::yield_now();
                        continue 'acquire;
                    }
                };
                guards.push(guard);
            }
            break guards;
        };
        let mut cells = Vec::with_capacity(updates.len());
        for (position, expected, new) in updates {
            let (coordinate, relative) = position.chunk_and_chunk_relative_position();
            let Some(index) = chunks
                .iter()
                .position(|chunk| chunk.x == coordinate.x && chunk.z == coordinate.y)
            else {
                return false;
            };
            let y = position.0.y - chunks[index].section.min_y;
            let Ok(y) = usize::try_from(y) else {
                return false;
            };
            let x = relative.x as usize;
            let z = relative.z as usize;
            let Some(palette) = sections[index].get(y / BlockPalette::SIZE) else {
                return false;
            };
            if palette.get(x, y % BlockPalette::SIZE, z) != *expected {
                return false;
            }
            if expected != new {
                cells.push((index, x, y, z, *new));
            }
        }
        let mut changed_chunks = FxHashSet::default();
        let mut changed_columns = FxHashSet::default();
        for &(index, x, y, z, new) in &cells {
            changed_chunks.insert(index);
            changed_columns.insert((index, x, z));
            sections[index][y / BlockPalette::SIZE].set(x, y % BlockPalette::SIZE, z, new);
        }
        drop(sections);
        // Refresh against live palettes: a later single-cell write may already have committed.
        for index in changed_chunks {
            chunks[index].refresh_observed_write_caches();
        }
        for (index, x, z) in changed_columns {
            let chunk = &chunks[index];
            let mut heightmap = chunk
                .heightmap
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            chunk.populate_heightmaps(&mut heightmap, chunk.get_highest_non_empty_subchunk(), x, z);
        }
        true
    }

    fn refresh_observed_write_caches(&self) {
        let sections = self
            .section
            .block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ticks = self
            .section
            .random_tick_sections
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (cache, mask) = ChunkSections::build_random_tick_sections_cache(&sections);
        *ticks = cache;
        for (index, ticking) in mask.into_iter().enumerate() {
            self.section.randomly_ticking_mask.set(index, ticking);
        }
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
