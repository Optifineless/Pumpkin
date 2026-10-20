use std::sync::Arc;

use pumpkin_data::{Block, BlockStateId, block_properties::BedPart};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::{World, neighbor_updater::UPDATE_LIMIT, portal::frame_effects::RetiredBlockEffects};
use crate::plugin::api::events::block::block_break::BlockBreakEvent;

type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;

impl World {
    pub(crate) fn bed_pair_for_use(
        &self,
        block: &Block,
        position: BlockPos,
    ) -> Option<(BlockPos, BlockPos, BlockStateId)> {
        let state = self.get_block_state_id(&position);
        if Block::from_state_id(state) != block {
            return None;
        }
        let props = BedProperties::from_state_id(state);
        let (head, foot) = if props.part == BedPart::Head {
            (
                position,
                position.offset(props.facing.opposite().to_offset()),
            )
        } else {
            (position.offset(props.facing.to_offset()), position)
        };
        let head_state = self.get_block_state_id(&head);
        let foot_state = self.get_block_state_id(&foot);
        if Block::from_state_id(head_state) != block || Block::from_state_id(foot_state) != block {
            return None;
        }
        let head_props = BedProperties::from_state_id(head_state);
        let foot_props = BedProperties::from_state_id(foot_state);
        // Reject malformed crossed pairs so consuming one bed cannot remove another bed's half.
        (head_props.part == BedPart::Head
            && foot_props.part == BedPart::Foot
            && head_props.facing == props.facing
            && foot_props.facing == props.facing)
            .then_some((head, foot, head_state))
    }

    pub(crate) fn destroy_bed_on_use(
        self: &Arc<Self>,
        head: BlockPos,
        foot: BlockPos,
        head_state: BlockStateId,
    ) -> bool {
        pumpkin_util::sync::assert_not_in_snapshot(
            pumpkin_util::sync::SnapshotBoundary::Mutation,
            "World::destroy_bed_on_use",
        );
        let block = Block::from_state_id(head_state);
        // BedBlock.destroyOnUse removes head then foot. Preflight the fork's two break events
        // before either write, so cancelling the partner cannot leave a half-removed bed.
        let halves = [(head, head_state), (foot, self.get_block_state_id(&foot))];
        // Keep recursive and concurrent unsafe uses out until both callbacks finish and commit.
        let Some(reservation) = self.reserve_bed_use(&halves) else {
            return false;
        };
        let effects =
            halves.map(|(position, state)| RetiredBlockEffects::capture(self, &position, state));
        let mut drops = [false; 2];
        for (index, (position, _)) in halves.iter().enumerate() {
            let mut event = BlockBreakEvent::new(None, block, *position, 0, false);
            if let Some(server) = self.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.cancelled
                || halves
                    .iter()
                    .any(|(position, state)| self.get_block_state_id(position) != *state)
            {
                return false;
            }
            drops[index] = event.drop;
        }
        // No owner is retained across plugin callbacks. Reject replaced halves before committing.
        if self.bed_pair_for_use(block, head) != Some((head, foot, head_state))
            || halves
                .iter()
                .any(|(position, state)| self.get_block_state_id(position) != *state)
        {
            return false;
        }
        let Some(_head_mutation) = self.block_entity_mutation_permit(head) else {
            return false;
        };
        let Some(_foot_mutation) = self.block_entity_mutation_permit(foot) else {
            return false;
        };
        let retired = self.remove_bed_states_if(&halves);
        let committed = retired.len();
        drop(reservation);
        // The established owner is per position; even a later rejected write must finish
        // the notifications and block-entity retirement of an already committed half.
        for (index, (position, state, entity)) in retired.into_iter().enumerate() {
            entity.on_block_replaced(self, &position);
            self.on_block_state_set_with_block_entity(
                &position,
                state,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_ALL,
                UPDATE_LIMIT,
                Some(&effects[index]),
            );
            if drops[index] {
                let params = super::loot::LootContextParameters {
                    block_state: Some(state.to_state()),
                    position: Some(position.to_f64()),
                    killed_by_player: Some(false),
                    ..Default::default()
                };
                crate::block::drop_loot(self, block, &position, true, &params);
            }
        }
        // BedBlock.destroyOnUse still explodes after consuming an available half.
        committed != 0
    }
}

#[cfg(test)]
#[path = "bed_use_tests.rs"]
mod tests;

#[path = "bed_sleep.rs"]
mod sleep;
