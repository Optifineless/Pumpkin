use crate::{block::PlayerWillDestroyArgs, entity::EntityBase, net::ClientPlatform};
use pumpkin_data::{BlockState, BlockStateId, world::WorldEvent};
use pumpkin_protocol::{
    VarInt,
    bedrock::client::level_event::{CLevelEvent, LevelEvent},
    java::client::play::CWorldEvent,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

pub(super) fn remove_partner(
    args: PlayerWillDestroyArgs<'_>,
    pos: BlockPos,
    replacement: BlockStateId,
) {
    // AbstractBedBlock.playerWillDestroy / DoublePlantBlock.preventDropFromBottomPart: flag 35.
    let state = args.world.get_block_state_id(&pos);
    args.world.set_block_state(
        &pos,
        replacement,
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
    );
    let java = CWorldEvent::new(
        WorldEvent::ParticlesDestroyBlock as i32,
        pos,
        i32::from(state.as_u16()),
        false,
    );
    let bedrock = CLevelEvent {
        event_id: VarInt(LevelEvent::ParticlesDestroyBlock as i32),
        position: pos.to_centered_f64().to_f32_lossy(),
        data: VarInt(BlockState::to_be_network_id(state) as i32),
    };
    // Level.levelEvent(player, ...) excludes Java's predicting actor; Bedrock needs the effect.
    if let ClientPlatform::Bedrock(client) = args.player.client.as_ref() {
        client.try_enqueue_client_packet(&bedrock);
    }
    args.world.broadcast_to_chunk_except_editioned(
        pos.chunk_position(),
        &[args.player.get_entity().entity_uuid],
        &java,
        &bedrock,
    );
}
