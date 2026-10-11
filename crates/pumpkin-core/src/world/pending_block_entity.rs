use std::sync::Arc;

use pumpkin_util::math::position::BlockPos;

use super::World;
use crate::block::entities::{BlockEntity, block_entity_from_nbt};

#[cfg(test)]
pub(super) type PublicationHook = Arc<dyn Fn(&Arc<dyn BlockEntity>) + Send + Sync>;

impl World {
    pub(super) fn load_pending_block_entity(&self, pos: &BlockPos) -> Option<Arc<dyn BlockEntity>> {
        let chunk_pos = pos.chunk_position();
        let nbt = self
            .level
            .read_chunk_sync(&chunk_pos, |chunk| {
                chunk
                    .pending_block_entities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(pos)
                    .cloned()
            })
            .flatten()?;
        if let Some(custom_data) = nbt
            .get_compound("PumpkinCustomData")
            .or_else(|| nbt.get_compound("BukkitValues"))
        {
            self.custom_block_entity_data
                .insert(*pos, custom_data.clone());
        }
        let candidate = block_entity_from_nbt(&nbt)?;
        #[cfg(test)]
        {
            let hook = self.lazy_block_entity_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(&candidate);
            }
        }
        // LevelChunk.getBlockEntity/promotePendingBlockEntity has one canonical object per position.
        // Construction may race, but no caller can retain a losing inventory.
        let entity = self
            .block_entities
            .entry(chunk_pos)
            .or_default()
            .entry(*pos)
            .or_insert(candidate)
            .clone();
        self.bind_block_entity_context(entity.as_ref());
        Some(entity)
    }
}
