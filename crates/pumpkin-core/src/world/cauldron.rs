use std::sync::Arc;

use super::{World, neighbor_updater::UPDATE_LIMIT};
use pumpkin_data::BlockStateId;
use pumpkin_util::{
    math::position::BlockPos,
    sync::{SnapshotBoundary, assert_not_in_snapshot},
};
use pumpkin_world::world::BlockFlags;

impl World {
    // CauldronInteractions.emptyBucket delivers its result and statistics before
    // Level.setBlockAndUpdate callbacks. Admit the conditional write first, but
    // retain its ordinary effects until the inventory payment has settled. Item/stat
    // callbacks still see filled terrain, as in the pre-existing fork handlers.
    pub(crate) fn prepare_cauldron_fill(
        self: &Arc<Self>,
        position: BlockPos,
        old_state: BlockStateId,
        new_state: BlockStateId,
    ) -> Option<impl FnOnce() + 'static> {
        assert_not_in_snapshot(SnapshotBoundary::Mutation, "World::prepare_cauldron_fill");
        if !self.is_in_build_limit(position) {
            return None;
        }
        self.write_block_state_if(&position, new_state, |current| current == old_state)?;
        let world = Arc::clone(self);
        Some(move || {
            world.on_block_state_set(
                &position,
                old_state,
                new_state,
                BlockFlags::NOTIFY_ALL,
                UPDATE_LIMIT,
            );
        })
    }
}
