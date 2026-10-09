use super::*;
use crate::{
    entity::{death_test_world::DeathTestWorld, player::RespawnPoint},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_gamemode_change::PlayerGamemodeChangeEvent,
    },
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, biome::Biome, block_properties::RespawnAnchorLikeProperties, dimension::Dimension,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::{AtomicUsize, Ordering};

struct SpectatorChange {
    calls: AtomicUsize,
    cancel: bool,
}
impl EventHandler<PlayerGamemodeChangeEvent> for SpectatorChange {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerGamemodeChangeEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.previous_gamemode, GameMode::Survival);
        assert_eq!(event.new_gamemode, GameMode::Spectator);
        assert!(
            event.player.position().x > 5.0,
            "saved spawn is used before changing mode"
        );
        self.calls.fetch_add(1, Ordering::Relaxed);
        event.cancelled = self.cancel;
        Box::pin(async {})
    }
}

fn saved_spawn(fixture: &DeathTestWorld, block: &Block) -> TestPlayer {
    // ServerPlayer.findRespawnAndUseSpawnBlock requires RespawnAnchorBlock.canSetSpawn.
    let dimension = if block == &Block::RESPAWN_ANCHOR {
        Dimension::THE_NETHER
    } else {
        Dimension::OVERWORLD
    };
    let world = fixture.server.get_world_from_dimension(&dimension);
    if block == &Block::RESPAWN_ANCHOR {
        assert!(world.dimension.respawn_anchor_works);
        let generator = pumpkin_world::generation::generator::WorldGenerator::Flat(Box::new(
            pumpkin_world::generation::generator::flat::FlatGenerator::new(
                pumpkin_util::world_seed::Seed(0),
                dimension.clone(),
                Vec::new(),
                Biome::NETHER_WASTES.registry_id.to_owned(),
            ),
        ));
        let mut chunk = pumpkin_world::generation::proto_chunk::ProtoChunk::new(0, 0, &generator);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block_state(x, 63, z, Block::STONE.default_state);
            }
        }
        let mut chunk = pumpkin_world::chunk_system::chunk_state::Chunk::Proto(Box::new(chunk));
        chunk.upgrade_to_level_chunk(
            &dimension,
            &pumpkin_config::lighting::LightingEngineConfig::default(),
        );
        let pumpkin_world::chunk_system::chunk_state::Chunk::Level(chunk) = chunk else {
            panic!("Nether fixture must be loaded");
        };
        world
            .level
            .loaded_chunks
            .insert(pumpkin_util::math::vector2::Vector2::new(0, 0), chunk);
    } else {
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    }
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = 3;
        info.spawn_y = 64;
        info.spawn_z = 3;
        info
    });
    let pos = BlockPos::new(8, 64, 8);
    world.set_block_state(&pos, block.default_state.id, BlockFlags::FORCE_STATE);
    let client = TestPlayer::new(&world);
    let player = &client.player;
    player.living_entity.health.store(0.0);
    *player.respawn_point.lock().unwrap() = Some(RespawnPoint {
        dimension,
        position: pos,
        yaw: 90.0,
        force: false,
    });
    client
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardcore_respawn_uses_saved_bed_or_anchor_then_spectates() {
    let fixture = DeathTestWorld::new().await;
    for block in [&Block::WHITE_BED, &Block::RESPAWN_ANCHOR, &Block::AIR] {
        let mut client = saved_spawn(&fixture, block);
        let player = client.player.clone();
        let world = player.world();
        let pos = BlockPos::new(8, 64, 8);
        if block == &Block::RESPAWN_ANCHOR {
            let mut props = RespawnAnchorLikeProperties::from_state_id(block.default_state.id);
            props.charges = 2;
            world.set_block_state(&pos, props.to_state_id(block), BlockFlags::FORCE_STATE);
        }
        client
            .with_outgoing_writer(respawn_after_death(&player, true))
            .await;
        assert_eq!(player.gamemode.load(), GameMode::Spectator);
        if block == &Block::AIR {
            assert!(player.respawn_point.lock().unwrap().is_none());
        } else {
            assert!(player.respawn_point.lock().unwrap().is_some());
            assert!((player.position().x - 8.5).abs() <= 2.0);
        }
        if block == &Block::RESPAWN_ANCHOR {
            assert_eq!(
                RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&pos)).charges,
                1
            );
        }
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardcore_respawn_preserves_gamemode_event_contract() {
    for cancel in [false, true] {
        let fixture = DeathTestWorld::new().await;
        let mut client = saved_spawn(&fixture, &Block::WHITE_BED);
        let player = client.player.clone();
        let control = Arc::new(SpectatorChange {
            calls: AtomicUsize::new(0),
            cancel,
        });
        fixture
            .server
            .plugin_manager
            .register::<PlayerGamemodeChangeEvent, _>(control.clone(), EventPriority::Normal, true);
        client
            .with_outgoing_writer(respawn_after_death(&player, true))
            .await;
        assert_eq!(control.calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            player.gamemode.load(),
            if cancel {
                GameMode::Survival
            } else {
                GameMode::Spectator
            }
        );
        fixture.server.shutdown().await;
    }
}
