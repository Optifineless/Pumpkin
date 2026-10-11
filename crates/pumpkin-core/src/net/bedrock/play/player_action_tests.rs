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

struct CountDamage(std::sync::atomic::AtomicUsize);
impl EventHandler<BlockDamageEvent> for CountDamage {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut BlockDamageEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}
fn dig(player: &TestBedrockPlayer, server: &Server, action: PlayerAction, position: BlockPos) {
    player.client().handle_player_action(
        &player.player,
        server,
        &SPlayerAction {
            player_runtime_id: VarULong(player.player.entity_id() as u64),
            action,
            block_position: position,
            result_pos: position,
            face: VarInt(1),
        },
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_continued_dig_fires_damage_once() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestBedrockPlayer::new(&world).await;
    player.player.permission_lvl.store(PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let events = Arc::new(CountDamage(std::sync::atomic::AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<BlockDamageEvent, _>(events.clone(), EventPriority::Normal, true);
    let position = BlockPos::new(8, 63, 8);
    dig(
        &player,
        &fixture.server,
        PlayerAction::StartDestroyBlock,
        position,
    );
    for _ in 0..5 {
        dig(
            &player,
            &fixture.server,
            PlayerAction::ContinueDestroyBlock,
            position,
        );
    }
    assert_eq!(events.0.load(Ordering::Relaxed), 1);
    assert!(player.player.mining.load(Ordering::Relaxed));
    player.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_bedrock_continue_new_target_starts_once_and_creative_fires_once() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestBedrockPlayer::new(&world).await;
    player.player.permission_lvl.store(PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let events = Arc::new(CountDamage(std::sync::atomic::AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<BlockDamageEvent, _>(events.clone(), EventPriority::Normal, true);
    let position = BlockPos::new(8, 63, 8);
    dig(
        &player,
        &fixture.server,
        PlayerAction::StartDestroyBlock,
        position,
    );
    let next = position.offset(Vector3::new(1, 0, 0));
    dig(
        &player,
        &fixture.server,
        PlayerAction::ContinueDestroyBlock,
        next,
    );
    assert!(player.player.is_destroying_block_at(&next));
    assert_eq!(events.0.load(Ordering::Relaxed), 2);
    for action in [
        PlayerAction::StartDestroyBlock,
        PlayerAction::ContinueDestroyBlock,
    ] {
        dig(&player, &fixture.server, action, next);
    }
    assert_eq!(events.0.load(Ordering::Relaxed), 2);
    player.player.set_gamemode(GameMode::Creative);
    player.player.set_client_loaded(true);
    dig(
        &player,
        &fixture.server,
        PlayerAction::ContinueDestroyBlock,
        next,
    );
    assert!(world.get_block_state(&next).is_air());
    assert_eq!(events.0.load(Ordering::Relaxed), 3);
    player.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_finish_and_tick_recheck_mid_dig_permissions() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestBedrockPlayer::new(&world).await;
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let position = BlockPos::new(8, 63, 8);
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = 8;
        info.spawn_z = 8;
        info
    });
    for finish in [
        PlayerAction::PredictDestroyBlock,
        PlayerAction::StopDestroyBlock,
    ] {
        for denial in 0..4 {
            player.player.set_gamemode(GameMode::Survival);
            player.player.set_client_loaded(true);
            player.player.permission_lvl.store(PermissionLvl::Four);
            world.worldborder.lock().unwrap().new_diameter = 1000.0;
            dig(
                &player,
                &fixture.server,
                PlayerAction::StartDestroyBlock,
                position,
            );
            assert!(player.player.mining.load(Ordering::Relaxed));
            player
                .player
                .tick_counter
                .fetch_add(1000, Ordering::Relaxed);
            match denial {
                0 => {
                    player.player.set_gamemode(GameMode::Spectator);
                }
                1 => {
                    player.player.set_gamemode(GameMode::Adventure);
                }
                2 => world.worldborder.lock().unwrap().new_diameter = 2.0,
                _ => player.player.permission_lvl.store(PermissionLvl::Zero),
            }
            player.player.set_client_loaded(true);
            dig(&player, &fixture.server, finish, position);
            assert_eq!(world.get_block(&position), &Block::STONE);
        }
    }
    // Bedrock's automatic tick completion must use the same final gate.
    player.player.permission_lvl.store(PermissionLvl::Four);
    player.player.set_gamemode(GameMode::Survival);
    player.player.set_client_loaded(true);
    dig(
        &player,
        &fixture.server,
        PlayerAction::StartDestroyBlock,
        position,
    );
    player
        .player
        .tick_counter
        .fetch_add(1000, Ordering::Relaxed);
    player.player.set_gamemode(GameMode::Spectator);
    player.player.tick_block_breaking();
    assert_eq!(world.get_block(&position), &Block::STONE);
    player.close().await;
    fixture.server.shutdown().await;
}
