use std::sync::Arc;

use crate::{
    server::{Server, combat_test_support},
    world::World,
};

pub struct PlayerFixture {
    pub world: Arc<World>,
    _server: Arc<Server>,
    _dir: tempfile::TempDir,
}

impl PlayerFixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
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

pub fn creative_break_java(world: &Arc<World>, position: pumpkin_util::math::position::BlockPos) {
    use crate::entity::EntityBase;
    let client = crate::net::java::combat_test_support::TestPlayer::new(world);
    client
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Creative);
    client
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    client
        .player
        .get_entity()
        .set_pos(position.to_centered_f64());
    client.client().handle_player_action(
        &client.player,
        &pumpkin_protocol::java::server::play::SPlayerAction {
            status: pumpkin_protocol::VarInt(0),
            position,
            face: 1,
            sequence: pumpkin_protocol::VarInt(1),
        },
        &world.server.upgrade().unwrap(),
    );
}

pub async fn creative_break_bedrock(
    world: &Arc<World>,
    position: pumpkin_util::math::position::BlockPos,
) {
    use crate::entity::EntityBase;
    let client = crate::net::bedrock::combat_test_support::TestBedrockPlayer::new(world).await;
    client
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Creative);
    client
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    client
        .player
        .get_entity()
        .set_pos(position.to_centered_f64());
    client.client().handle_player_action(
        &client.player,
        &world.server.upgrade().unwrap(),
        &pumpkin_protocol::bedrock::server::player_action::SPlayerAction {
            player_runtime_id: pumpkin_protocol::codec::var_ulong::VarULong(0),
            action: pumpkin_protocol::bedrock::server::player_action::PlayerActionType::CreativeDestroyBlock,
            block_position: position,
            result_pos: position,
            face: pumpkin_protocol::VarInt(1),
        },
    );
    client.close().await;
}
