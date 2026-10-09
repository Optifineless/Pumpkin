use super::*;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::bedrock::combat_test_support::TestBedrockPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::block::block_damage::BlockDamageEvent,
    },
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome};
use pumpkin_protocol::{VarInt, codec::var_ulong::VarULong};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
};

struct CancelDamage;
impl EventHandler<BlockDamageEvent> for CancelDamage {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockDamageEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}
async fn egg_attack(mode: GameMode, protected: bool, cancelled: bool, allowed: bool) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestBedrockPlayer::new(&world).await;
    player.player.set_gamemode(mode);
    player.player.set_client_loaded(true);
    if !protected {
        player.player.permission_lvl.store(PermissionLvl::Four);
    }
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = 8;
        info.spawn_z = 8;
        info
    });
    let position = BlockPos::new(8, 64, 8);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    world.set_block_state(
        &position,
        Block::DRAGON_EGG.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    if cancelled {
        fixture
            .server
            .plugin_manager
            .register::<BlockDamageEvent, _>(Arc::new(CancelDamage), EventPriority::Normal, true);
    }
    if protected {
        assert!(world.is_in_spawn_protection(&player.player, &position));
    }
    player.client().handle_player_action(
        &player.player,
        &fixture.server,
        &SPlayerAction {
            player_runtime_id: VarULong(player.player.entity_id() as u64),
            action: PlayerAction::StartDestroyBlock,
            block_position: position,
            result_pos: position,
            face: VarInt(1),
        },
    );
    assert_eq!(world.get_block_state(&position).is_air(), allowed);
    assert!(
        world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_item_entity().is_none())
    );
    player.close().await;
    fixture.server.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_protected_egg_attack_has_no_effect() {
    egg_attack(GameMode::Survival, true, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_cancelled_egg_attack_has_no_effect() {
    egg_attack(GameMode::Survival, false, true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_spectator_egg_attack_has_no_effect() {
    egg_attack(GameMode::Spectator, false, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_adventure_egg_attack_has_no_effect() {
    egg_attack(GameMode::Adventure, false, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_border_rejected_note_attack_has_no_effect() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestBedrockPlayer::new(&world).await;
    player.player.permission_lvl.store(PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::NOTE_BLOCK.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.worldborder.lock().unwrap().new_diameter = 2.0;
    player.client().handle_player_action(
        &player.player,
        &fixture.server,
        &SPlayerAction {
            player_runtime_id: VarULong(player.player.entity_id() as u64),
            action: PlayerAction::StartDestroyBlock,
            block_position: position,
            result_pos: position,
            face: VarInt(1),
        },
    );
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::PlayNoteblock as i32
        ),
        0
    );
    player.close().await;
    fixture.server.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_survival_egg_attack_teleports() {
    egg_attack(GameMode::Survival, false, false, true).await;
}
