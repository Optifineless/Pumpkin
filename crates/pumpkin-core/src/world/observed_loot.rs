use std::sync::Arc;

use pumpkin_data::{BlockStateId, item_stack::ItemStack};
use pumpkin_util::math::position::BlockPos;

use super::{World, loot::LootContextParameters};
use crate::block::entities::BlockEntity;

/// Keeps the pre-removal plugin view while admitting physical drops only after commit.
pub(super) struct PreparedBlockDrops {
    state: BlockStateId,
    params: LootContextParameters,
    items: Vec<ItemStack>,
}

impl PreparedBlockDrops {
    pub(super) const fn tool(&self) -> Option<&ItemStack> {
        self.params.tool.as_ref()
    }

    pub(super) fn prepare(
        world: &Arc<World>,
        position: &BlockPos,
        state: BlockStateId,
        params: &LootContextParameters,
    ) -> Self {
        // Block.getDrops runs while the original terrain and block entity are visible.
        let params = super::loot::build_block_loot_context(world, position, params);
        let items =
            crate::block::block_drops_with_context(world, state.to_block(), position, &params);
        Self {
            state,
            params,
            items,
        }
    }

    pub(super) fn finish(
        mut self,
        world: &Arc<World>,
        position: &BlockPos,
        entity: Option<&dyn BlockEntity>,
    ) -> Vec<ItemStack> {
        // Cancellation/cleared output remains authoritative even if contents changed.
        if self.items.is_empty() {
            return self.items;
        }
        let components = entity
            .map(super::loot::collect_retired_block_entity_components)
            .unwrap_or_default();
        let unchanged = self.params.block_entity_components.len() == components.len()
            && self
                .params
                .block_entity_components
                .iter()
                .all(|(key, old)| {
                    components
                        .get(key)
                        .is_some_and(|new| old.equal(new.as_ref()))
                });
        if unchanged {
            return self.items;
        }
        // A transfer or callback changed contents before retirement. Re-evaluate only
        // this interference case, using the now-retired owner's stable components.
        self.params.block_entity_components = components;
        crate::block::block_drops_with_context(world, self.state.to_block(), position, &self.params)
    }
}
