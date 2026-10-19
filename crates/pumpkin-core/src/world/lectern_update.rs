//! Lectern terrain writes share page admission, while notifications run after it.

use std::sync::Arc;

use pumpkin_data::{Block, block_properties::LecternLikeProperties};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::sync::{SnapshotBoundary as SB, assert_not_in_snapshot};
use pumpkin_world::world::BlockFlags;

use super::{World, neighbor_updater};

impl World {
    /// Commits the lectern state under the caller's mutation permit, returning its effects.
    pub(crate) fn prepare_lectern_state_update(
        self: &Arc<Self>,
        position: BlockPos,
        powered: bool,
        has_book: Option<bool>,
    ) -> Option<impl FnOnce() + use<>> {
        assert_not_in_snapshot(SB::Mutation, "World::prepare_lectern_state_update");
        let (replaced, updated) = loop {
            let (block, state) = self.get_block_and_state_id(&position);
            if block != &Block::LECTERN {
                return None;
            }
            let mut properties = LecternLikeProperties::from_state_id(state);
            properties.powered = powered;
            if let Some(has_book) = has_book {
                properties.has_book = has_book;
            }
            let updated = properties.to_state_id(block);
            if let Some(replaced) =
                self.write_block_state_if(&position, updated, |current| current == state)
            {
                break (replaced, updated);
            }
        };
        let world = self.clone();
        Some(move || {
            world.on_block_state_set(
                &position,
                replaced,
                updated,
                BlockFlags::NOTIFY_ALL,
                neighbor_updater::UPDATE_LIMIT,
            );
        })
    }
}
