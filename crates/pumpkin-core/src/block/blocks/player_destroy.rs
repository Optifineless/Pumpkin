use crate::{block::PlayerWillDestroyArgs, entity::EntityBase, net::ClientPlatform};
use pumpkin_data::{BlockState, BlockStateId, world::WorldEvent};
use pumpkin_protocol::{
    VarInt,
    bedrock::client::level_event::{CLevelEvent, LevelEvent},
    java::client::play::CWorldEvent,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

/// Removes a paired half without drops and sends its destroy particles to both editions.
pub fn remove_partner(args: PlayerWillDestroyArgs<'_>, pos: BlockPos, replacement: BlockStateId) {
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

#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use crate::{entity::EntityBase, server::Server, world::World};
    use pumpkin_protocol::{
        VarInt,
        bedrock::server::player_action::{PlayerActionType, SPlayerAction as BedrockPlayerAction},
        java::server::play::SPlayerAction as JavaPlayerAction,
    };
    use pumpkin_util::{GameMode, math::position::BlockPos, permission::PermissionLvl};

    pub struct PlayerFixture {
        pub world: Arc<World>,
        _server: Arc<Server>,
        _dir: tempfile::TempDir,
    }

    impl PlayerFixture {
        #[must_use]
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let server = crate::server::combat_test_support::server(dir.path());
            let world = crate::server::combat_test_support::world(&server, dir.path());
            Self {
                world,
                _server: server,
                _dir: dir,
            }
        }

        pub async fn finish(self) {
            self.world.level.shutdown().await.unwrap();
        }
    }

    impl Default for PlayerFixture {
        fn default() -> Self {
            Self::new()
        }
    }

    pub fn creative_break_java(world: &Arc<World>, position: BlockPos) {
        let client = crate::net::java::combat_test_support::TestPlayer::new(world);
        client.player.gamemode.store(GameMode::Creative);
        client.player.permission_lvl.store(PermissionLvl::Four);
        client
            .player
            .get_entity()
            .set_pos(position.to_centered_f64());
        client.client().handle_player_action(
            &client.player,
            &JavaPlayerAction {
                status: VarInt(0),
                position,
                face: 1,
                sequence: VarInt(1),
            },
            &world.server.upgrade().unwrap(),
        );
    }

    pub async fn creative_break_bedrock(world: &Arc<World>, position: BlockPos) {
        let client = crate::net::bedrock::combat_test_support::TestBedrockPlayer::new(world).await;
        client.player.gamemode.store(GameMode::Creative);
        client.player.permission_lvl.store(PermissionLvl::Four);
        client
            .player
            .get_entity()
            .set_pos(position.to_centered_f64());
        client.client().handle_player_action(
            &client.player,
            &world.server.upgrade().unwrap(),
            &BedrockPlayerAction {
                player_runtime_id: pumpkin_protocol::codec::var_ulong::VarULong(0),
                action: PlayerActionType::CreativeDestroyBlock,
                block_position: position,
                result_pos: position,
                face: VarInt(1),
            },
        );
        client.close().await;
    }
}
