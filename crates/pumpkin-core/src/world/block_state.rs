use std::sync::Arc;

use pumpkin_data::{Block, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::World;
use crate::block::{
    blocks::copper_chest::should_changed_state_keep_block_entity, entities::block_entity_name,
};

impl World {
    pub(super) fn remove_replaced_block_entity(
        self: &Arc<Self>,
        position: &BlockPos,
        old_block: &Block,
        new_block: &Block,
        flags: BlockFlags,
    ) {
        // LevelChunk.setBlockState respects CopperChestBlock.shouldChangedStateKeepBlockEntity.
        if old_block != new_block
            && block_entity_name(old_block).is_some()
            && !should_changed_state_keep_block_entity(old_block, new_block)
        {
            if let Some(entity) = self.get_block_entity(position)
                && !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK)
            {
                entity.on_block_replaced(self, position);
            }
            self.remove_block_entity(position);
        }
    }

    pub(super) fn on_block_placed(
        self: &Arc<Self>,
        position: &BlockPos,
        replaced_block_state_id: BlockStateId,
        block_state_id: BlockStateId,
        block_moved: bool,
    ) {
        let old_block = Block::from_state_id(replaced_block_state_id);
        let new_block = Block::from_state_id(block_state_id);
        // LevelChunk.setBlockState reuses copper-chest entities rather than creating them again.
        if !should_changed_state_keep_block_entity(old_block, new_block) {
            self.block_registry.on_placed(
                self,
                new_block,
                block_state_id,
                position,
                replaced_block_state_id,
                block_moved,
            );
        }
    }
}
