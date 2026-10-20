use std::sync::Arc;

use pumpkin_data::{Block, BlockStateId, block_properties::WhiteBedLikeProperties};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::super::World;

impl World {
    pub(crate) fn set_sleep_bed_half_occupied_if(
        self: &Arc<Self>,
        half: (BlockPos, BlockStateId),
        occupied: bool,
        is_current: impl Fn() -> bool,
    ) -> bool {
        let Some(mutation) = self.block_entity_mutation_permit(half.0) else {
            return false;
        };
        if !is_current() {
            return false;
        }
        let mut properties = WhiteBedLikeProperties::from_state_id(half.1);
        properties.occupied = occupied;
        let next = properties.to_state_id(Block::from_state_id(half.1));
        let committed = self
            .write_block_state_if(&half.0, next, |state| state == half.1)
            .is_some();
        drop(mutation);
        if committed {
            self.on_block_state_set(
                &half.0,
                half.1,
                next,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
                super::super::neighbor_updater::UPDATE_LIMIT,
            );
        }
        committed
    }

    pub(crate) fn set_sleep_bed_occupied_if(
        self: &Arc<Self>,
        halves: &[(BlockPos, BlockStateId); 2],
        occupied: bool,
        is_current: impl Fn() -> bool,
    ) -> bool {
        let Some(reservation) = self.reserve_bed_use(halves) else {
            return false;
        };
        let Some(head_mutation) = self.block_entity_mutation_permit(halves[0].0) else {
            return false;
        };
        let Some(foot_mutation) = self.block_entity_mutation_permit(halves[1].0) else {
            return false;
        };
        if !is_current() {
            return false;
        }
        let next = halves.map(|(_, state)| {
            let mut properties = WhiteBedLikeProperties::from_state_id(state);
            properties.occupied = occupied;
            properties.to_state_id(Block::from_state_id(state))
        });
        if self
            .write_block_state_if(&halves[0].0, next[0], |state| state == halves[0].1)
            .is_none()
        {
            return false;
        }
        let committed = self
            .write_block_state_if(&halves[1].0, next[1], |state| state == halves[1].1)
            .is_some();
        if !committed {
            let _ = self.write_block_state_if(&halves[0].0, halves[0].1, |state| state == next[0]);
        }
        drop(foot_mutation);
        drop(head_mutation);
        drop(reservation);
        // Shape propagation is unnecessary: both halves were committed before any notification.
        if committed {
            for (index, &(position, previous)) in halves.iter().enumerate() {
                self.on_block_state_set(
                    &position,
                    previous,
                    next[index],
                    BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
                    super::super::neighbor_updater::UPDATE_LIMIT,
                );
            }
        }
        committed
    }
}
