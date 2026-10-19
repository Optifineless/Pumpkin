use std::sync::Arc;

use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::{BedPart, WhiteBedLikeProperties},
    world::WorldEvent,
};
use pumpkin_protocol::{
    VarInt,
    bedrock::client::level_event::{CLevelEvent, LevelEvent},
    java::client::play::CWorldEvent,
};
use pumpkin_util::{GameMode, math::position::BlockPos};
use pumpkin_world::world::BlockFlags;

use super::{World, portal::frame_effects::RetiredBlockEffects};
use crate::{
    entity::{EntityBase, player::Player},
    net::ClientPlatform,
    plugin::api::events::block::block_break::BlockBreakEvent,
};

impl World {
    pub(crate) fn break_bed_block(
        self: &Arc<Self>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        effect_actor: Option<&Arc<Player>>,
        flags: BlockFlags,
        update_limit: u32,
        expected: BlockStateId,
    ) -> Option<BlockStateId> {
        let block = Block::from_state_id(expected);
        let retired = RetiredBlockEffects::capture(self, position, expected);
        let mut event = BlockBreakEvent::new(
            cause.cloned(),
            block,
            *position,
            0,
            !flags.contains(BlockFlags::SKIP_DROPS),
        );
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled || self.get_block_state_id(position) != expected {
            return None;
        }
        let mut flags = flags;
        flags.set(BlockFlags::SKIP_DROPS, !event.drop);
        self.block_registry
            .player_will_destroy(self, cause, position, expected.to_state());
        if cause.is_some_and(|player| player.gamemode.load() == GameMode::Creative)
            && WhiteBedLikeProperties::from_state_id(expected).part == BedPart::Foot
            && retired.bed_partner_is_unchanged(self, expected)
        {
            // A cancelled Creative partner event must not be retried by the foot's shape cascade.
            flags.insert(BlockFlags::UPDATE_KNOWN_SHAPE);
        }
        // LevelChunk.setBlockState commits the admitted half before its neighbor callbacks.
        let (_, _, entity) = self.remove_bed_states_if(&[(*position, expected)]).pop()?;
        if !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK) {
            entity.on_block_replaced(self, position);
        }
        if event.drop {
            let tool = cause.and_then(|player| {
                let stack = player.inventory().held_item();
                (!stack.is_empty()).then_some(stack)
            });
            let params = super::loot::LootContextParameters {
                tool,
                block_state: Some(expected.to_state()),
                position: Some(position.to_f64()),
                killed_by_player: Some(cause.is_some()),
                ..Default::default()
            };
            crate::block::drop_loot(self, block, position, true, &params);
        }
        let mut completion_flags = flags;
        if WhiteBedLikeProperties::from_state_id(expected).part == BedPart::Head {
            completion_flags.insert(BlockFlags::SKIP_DROPS);
        }
        // A loot listener can replace the committed cell before Level.setBlock's completion.
        // Retire only its captured orphan, retaining a listener's replacement bed pair.
        if self.get_block_state_id(position) != BlockStateId::AIR
            && let Some((partner, state)) = retired.orphaned_bed_partner(self, position, expected)
        {
            self.break_bed_block(&partner, None, None, completion_flags, update_limit, state);
        }
        self.on_block_state_set_with_block_entity(
            position,
            expected,
            BlockStateId::AIR,
            completion_flags,
            update_limit,
            Some(&retired),
        );
        self.send_block_destroy_effect(position, expected, effect_actor);
        Some(expected)
    }

    pub(super) fn send_block_destroy_effect(
        &self,
        position: &BlockPos,
        state: BlockStateId,
        cause: Option<&Arc<Player>>,
    ) {
        let java = CWorldEvent::new(
            WorldEvent::ParticlesDestroyBlock as i32,
            *position,
            i32::from(state.as_u16()),
            false,
        );
        let bedrock = CLevelEvent {
            event_id: VarInt(LevelEvent::ParticlesDestroyBlock as i32),
            position: position.to_centered_f64().to_f32_lossy(),
            data: VarInt(BlockState::to_be_network_id(state) as i32),
        };
        let chunk = position.chunk_position();
        // Java predicts its own break effect; Bedrock needs the server event.
        if let Some(player) = cause {
            if let ClientPlatform::Bedrock(client) = player.client.as_ref() {
                client.try_enqueue_client_packet(&bedrock);
            }
            self.broadcast_to_chunk_except_editioned(
                chunk,
                &[player.get_entity().entity_uuid],
                &java,
                &bedrock,
            );
        } else {
            self.broadcast_to_chunk_editioned(chunk, &java, &bedrock);
        }
    }
}
