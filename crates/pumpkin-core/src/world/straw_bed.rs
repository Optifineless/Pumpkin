use std::sync::Arc;

use pumpkin_data::{Block, BlockState, BlockStateId, world::WorldEvent};
use pumpkin_protocol::{
    VarInt,
    bedrock::client::level_event::{CLevelEvent, LevelEvent},
    java::client::play::CWorldEvent,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::World;
use crate::plugin::api::events::block::block_break::BlockBreakEvent;

impl World {
    /// Removes only the captured straw half, retaining the fork's cancellable break event.
    pub(crate) fn break_straw_bed_half(
        self: &Arc<Self>,
        position: &BlockPos,
        expected: BlockStateId,
        flags: BlockFlags,
        update_limit: u32,
    ) -> Option<BlockStateId> {
        pumpkin_util::sync::assert_not_in_snapshot(
            pumpkin_util::sync::SnapshotBoundary::Mutation,
            "World::break_straw_bed_half",
        );
        if Block::from_state_id(expected) != &Block::STRAW_BED
            || !self.is_in_build_limit(*position)
            || self.get_block_state_id(position) != expected
        {
            return None;
        }
        // Capture the partner before a break listener can install a different straw pair.
        let retired_effects =
            super::portal::frame_effects::RetiredBlockEffects::capture(self, position, expected);
        let mut event = BlockBreakEvent::new(
            None,
            &Block::STRAW_BED,
            *position,
            0,
            !flags.contains(BlockFlags::SKIP_DROPS),
        );
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return None;
        }
        // StrawBedBlock.destroyBed removes one half; callbacks must not authorize a newer block.
        // The existing atomic chunk write releases its guards before any completion effects.
        self.write_block_state_if(position, BlockStateId::AIR, |current| current == expected)?;
        let retired_entity = self.remove_loaded_block_entity(position);
        if !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK) {
            retired_entity.on_block_replaced(self, position);
        }
        self.on_block_state_set_with_block_entity(
            position,
            expected,
            BlockStateId::AIR,
            flags - BlockFlags::SKIP_DROPS,
            update_limit,
            Some(&retired_effects),
        );
        if event.drop {
            let params = super::loot::LootContextParameters {
                block_state: Some(expected.to_state()),
                position: Some(position.to_f64()),
                killed_by_player: Some(false),
                ..Default::default()
            };
            crate::block::drop_loot(self, &Block::STRAW_BED, position, true, &params);
        }
        let java = CWorldEvent::new(
            WorldEvent::ParticlesDestroyBlock as i32,
            *position,
            expected.as_u16().into(),
            false,
        );
        let bedrock = CLevelEvent {
            event_id: VarInt(LevelEvent::ParticlesDestroyBlock as i32),
            position: position.to_centered_f64().to_f32_lossy(),
            data: VarInt(BlockState::to_be_network_id(expected) as i32),
        };
        self.broadcast_to_chunk_editioned(position.chunk_position(), &java, &bedrock);
        Some(expected)
    }
}

#[cfg(test)]
#[path = "straw_bed_tests.rs"]
mod tests;
