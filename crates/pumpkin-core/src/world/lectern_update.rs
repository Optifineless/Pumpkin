//! Lectern terrain writes share page admission, while notifications run after it.

use std::sync::Arc;

use pumpkin_data::{Block, block_properties::LecternLikeProperties};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::sync::{SnapshotBoundary as SB, assert_not_in_snapshot};
use pumpkin_world::chunk::ChunkData;
use pumpkin_world::world::BlockFlags;

use super::{World, neighbor_updater};

// The retired inventory cannot change after this capture. A queued take may have
// removed its book before its deferred LecternBlock.resetBookState was admitted.
pub(super) fn reconcile_retiring_lectern(chunk: &ChunkData, position: BlockPos, nbt: &NbtCompound) {
    if nbt.get_string("id") != Some("minecraft:lectern") {
        return;
    }
    let relative = position.chunk_relative_position();
    let Some(state) =
        chunk
            .section
            .get_block_absolute_y(relative.x as usize, relative.y, relative.z as usize)
    else {
        return;
    };
    if Block::from_state_id(state) != &Block::LECTERN {
        return;
    }
    let mut properties = LecternLikeProperties::from_state_id(state);
    properties.has_book = nbt.get_compound("Book").is_some();
    if !properties.has_book {
        properties.powered = false;
    }
    let _ = chunk.set_block_absolute_y_if(
        relative.x as usize,
        relative.y,
        relative.z as usize,
        properties.to_state_id(&Block::LECTERN),
        |current| current == state,
    );
}

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
