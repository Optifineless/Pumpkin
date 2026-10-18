use super::*;
use crate::{
    entity::{
        EntityBase,
        ai::goal::{Goal, track_target::TrackTargetGoal},
        death_test_world::DeathTestWorld,
        player::{Player, RespawnPoint},
    },
    net::{
        ClientPlatform, PlayerConfig,
        java::combat_test_support::{TestPlayer, captured_client},
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::{
                player_gamemode_change::PlayerGamemodeChangeEvent,
                player_spawn_location::PlayerSpawnLocationEvent,
            },
            vehicle::vehicle_exit::VehicleExitEvent,
            world::chunk_send::ChunkSend,
        },
    },
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, biome::Biome, block_properties::RespawnAnchorLikeProperties, dimension::Dimension,
};
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::server::play::{SClientCommand, SConfirmTeleport, SPlayerPosition},
};
use pumpkin_util::PermissionLvl;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct GatedSpawnLocation {
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

impl EventHandler<PlayerSpawnLocationEvent> for GatedSpawnLocation {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerSpawnLocationEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.entered.notify_one();
            self.resume.notified().await;
        })
    }
}

struct ReturnToSurvivalOnVehicleExit {
    player: Arc<Player>,
    calls: AtomicUsize,
}

impl EventHandler<VehicleExitEvent> for ReturnToSurvivalOnVehicleExit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleExitEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.player.set_gamemode(GameMode::Survival);
        Box::pin(async {})
    }
}

struct SpectatorChange {
    calls: AtomicUsize,
    cancel: bool,
}

struct GatedChunkSend {
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    calls: AtomicUsize,
}

impl EventHandler<ChunkSend> for GatedChunkSend {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut ChunkSend,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            self.entered.notify_one();
            self.resume.notified().await;
        })
    }
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

fn publish_nether_chunk(world: &Arc<crate::world::World>) {
    let dimension = Dimension::THE_NETHER;
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
        publish_nether_chunk(&world);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardcore_disconnect_after_health_reset_saves_spectator_mode() {
    let fixture = DeathTestWorld::new().await;
    let storage = &fixture.server.player_data_storage;
    let mut client = saved_spawn(&fixture, &Block::WHITE_BED);
    let player = client.player.clone();
    client.client().set_player(player.clone());
    let (session, _) = storage.load_data(&player.gameprofile.id).await.unwrap();
    *player.storage_session.lock().unwrap() = Some(session);
    let gated_chunk = Arc::new(GatedChunkSend {
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register(gated_chunk.clone(), EventPriority::Normal, true);

    assert!(client.client().player.load().is_some());
    let mut respawn = Box::pin(client.with_outgoing_writer(respawn_after_death(&player, true)));
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        tokio::select! {
            () = gated_chunk.entered.notified() => {}
            _ = &mut respawn => panic!(
                "respawn passed the gated center chunk send (health={}, mode={:?}, chunk callbacks={})",
                player.living_entity.health.load(),
                player.gamemode.load(),
                gated_chunk.calls.load(Ordering::Relaxed),
            ),
        }
    })
    .await
    .expect("respawn should reach the center chunk send");
    assert!(player.living_entity.health.load() > 0.0);
    assert_eq!(player.gamemode.load(), GameMode::Spectator);

    if let ClientPlatform::Java(java_client) = player.client.as_ref() {
        java_client.close();
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::net::java::connection_owner::finish_disconnect(&player, &fixture.server),
    )
    .await
    .expect("disconnect should save after health reset");

    let (_session, saved) = storage.load_data(&player.gameprofile.id).await.unwrap();
    let saved = saved.unwrap();
    assert!(saved.get_float("Health").unwrap() > 0.0);
    assert_eq!(
        saved.get_int("playerGameType"),
        Some(GameMode::Spectator as i32)
    );

    gated_chunk.resume.notify_one();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(60), respawn)
        .await
        .expect("respawn should finish after chunk delivery resumes");
    assert!(completed);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logged_out_player_is_not_a_live_ai_target() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut client = TestPlayer::new(&world);
    let player = client.player.clone();
    player.get_entity().set_pos(Vector3::new(1.0, 64.0, 0.0));
    let mob = fixture.mob(&pumpkin_data::entity::EntityType::ZOMBIE);
    let mob_ai = mob.get_mob().unwrap();
    let target: Arc<dyn EntityBase> = player.clone();
    mob_ai.set_mob_target(Some(target));
    let mut target_goal = TrackTargetGoal::with_default(false);
    assert!(target_goal.should_continue(mob_ai));

    client.with_outgoing_writer(player.remove()).await;

    assert!(player.get_entity().is_removed());
    assert!(!target_goal.should_continue(mob_ai));
    target_goal.stop(mob_ai);
    assert!(mob_ai.get_mob_entity().get_target().is_none());
    fixture.server.shutdown().await;
}

// PlayerList.respawn pauses here inside the spawn-location hook.
async fn start_respawn_until_spawn_callback(
    fixture: &DeathTestWorld,
    client: &TestPlayer,
    player: &Arc<Player>,
    gated_spawn: &Arc<GatedSpawnLocation>,
) {
    fixture
        .server
        .plugin_manager
        .register(gated_spawn.clone(), EventPriority::Normal, true);

    let request = SClientCommand {
        action_id: VarInt(SClientCommand::PERFORM_RESPAWN),
    };
    client.client().handle_client_status(player, &request);
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        gated_spawn.entered.notified(),
    )
    .await
    .expect("respawn should reach its controllable spawn callback");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_respawn_cannot_remove_or_publish_rejoined_session() {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    publish_nether_chunk(&destination);
    let mut client = TestPlayer::new(&source);
    let old_player = client.player.clone();
    let (session, _) = fixture
        .server
        .player_data_storage
        .load_data(&old_player.gameprofile.id)
        .await
        .unwrap();
    *old_player.storage_session.lock().unwrap() = Some(session);
    old_player.living_entity.health.store(0.0);
    *old_player.respawn_point.lock().unwrap() = Some(RespawnPoint {
        dimension: Dimension::THE_NETHER,
        position: BlockPos::new(8, 64, 8),
        yaw: 0.0,
        force: false,
    });
    let mut anchor =
        RespawnAnchorLikeProperties::from_state_id(Block::RESPAWN_ANCHOR.default_state.id);
    anchor.charges = 2;
    destination.set_block_state(
        &BlockPos::new(8, 64, 8),
        anchor.to_state_id(&Block::RESPAWN_ANCHOR),
        BlockFlags::FORCE_STATE,
    );
    publish(&source, proto(&Biome::PLAINS, &Block::STONE));
    assert!(source.can_respawn_player(&old_player));
    let gated_spawn = Arc::new(GatedSpawnLocation {
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    start_respawn_until_spawn_callback(&fixture, &client, &old_player, &gated_spawn).await;

    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.with_outgoing_writer(crate::net::java::connection_owner::finish_disconnect(
            &old_player,
            &fixture.server,
        )),
    )
    .await
    .expect("disconnect cleanup should finish");
    let profile = old_player.gameprofile.clone();
    let (new_client, _new_packets) = captured_client(profile.clone());
    let (new_player, new_world) = fixture
        .server
        .add_player(
            Arc::new(ClientPlatform::Java(new_client)),
            profile,
            Some(PlayerConfig::default()),
        )
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&new_world, &source));
    new_player.set_client_loaded(true);
    assert!(source.remove_player(&old_player, false).await.is_none());
    assert!(
        source
            .players
            .load()
            .iter()
            .any(|player| Arc::ptr_eq(player, &new_player))
    );

    gated_spawn.resume.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.with_outgoing_writer(crate::server::combat_test_support::wait_for_tasks(
            &fixture.server,
        )),
    )
    .await
    .expect("stale respawn task should finish after the callback is released");

    assert!(
        source
            .players
            .load()
            .iter()
            .any(|player| Arc::ptr_eq(player, &new_player))
    );
    assert!(
        !source
            .players
            .load()
            .iter()
            .any(|player| Arc::ptr_eq(player, &old_player))
    );
    assert!(
        !destination
            .players
            .load()
            .iter()
            .any(|player| Arc::ptr_eq(player, &old_player))
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn respawn_and_spectator_release_rider_before_position_packets() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut client = TestPlayer::new(&world);
    let player = client.player.clone();
    player.permission_lvl.store(PermissionLvl::Four);
    let mount = crate::entity::r#type::from_type(
        &pumpkin_data::entity::EntityType::OAK_BOAT,
        Vector3::new(0.0, 64.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    assert!(world.spawn_entity(mount.clone()));
    mount
        .get_entity()
        .add_passenger(mount.clone(), player.clone() as Arc<dyn EntityBase>);
    let source = player.get_command_source(&fixture.server);
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input("gamemode spectator", &source)
        .unwrap();
    assert!(player.get_entity().get_vehicle().is_none());
    assert!(!mount.get_entity().has_passenger(player.entity_id()));

    let pending = player.awaiting_teleport.lock().unwrap().unwrap();
    client.client().handle_confirm_teleport(
        &player,
        &SConfirmTeleport {
            teleport_id: pending.0,
            position: pending.1,
            yaw: player.get_entity().yaw.load(),
            pitch: player.get_entity().pitch.load(),
        },
    );
    let position = player.get_entity().pos.load();
    client.client().handle_position(
        &player,
        &fixture.server,
        &SPlayerPosition {
            position: Vector3::new(position.x + 1.0, position.y, position.z),
            collision: 0,
        },
    );
    assert_eq!(player.get_entity().pos.load().x, position.x + 1.0);

    player.set_gamemode(GameMode::Survival);
    player.living_entity.health.store(0.0);
    mount
        .get_entity()
        .add_passenger(mount.clone(), player.clone() as Arc<dyn EntityBase>);
    let request = SClientCommand {
        action_id: VarInt(SClientCommand::PERFORM_RESPAWN),
    };
    client.client().handle_client_status(&player, &request);
    client
        .with_outgoing_writer(crate::server::combat_test_support::wait_for_tasks(
            &fixture.server,
        ))
        .await;

    assert!(player.get_entity().get_vehicle().is_none());
    assert!(!mount.get_entity().has_passenger(player.entity_id()));
    let pending = player.awaiting_teleport.lock().unwrap().unwrap();
    client.client().handle_confirm_teleport(
        &player,
        &SConfirmTeleport {
            teleport_id: pending.0,
            position: pending.1,
            yaw: player.get_entity().yaw.load(),
            pitch: player.get_entity().pitch.load(),
        },
    );
    let position = player.get_entity().pos.load();
    client.client().handle_position(
        &player,
        &fixture.server,
        &SPlayerPosition {
            position: Vector3::new(position.x + 1.0, position.y, position.z),
            collision: 0,
        },
    );
    assert_eq!(player.get_entity().pos.load().x, position.x + 1.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spectator_vehicle_exit_callback_cannot_leave_stale_physics() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = fixture.player("spectator-callback");
    player.permission_lvl.store(PermissionLvl::Four);
    let mount = crate::entity::r#type::from_type(
        &pumpkin_data::entity::EntityType::OAK_BOAT,
        Vector3::new(0.0, 64.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    assert!(world.spawn_entity(mount.clone()));
    mount
        .get_entity()
        .add_passenger(mount.clone(), player.clone() as Arc<dyn EntityBase>);
    let callback = Arc::new(ReturnToSurvivalOnVehicleExit {
        player: player.clone(),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register(callback.clone(), EventPriority::Normal, true);

    let source = player.get_command_source(&fixture.server);
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input("gamemode spectator", &source)
        .unwrap();

    assert_eq!(callback.calls.load(Ordering::Relaxed), 1);
    assert_eq!(player.gamemode.load(), GameMode::Survival);
    assert!(!player.get_entity().no_physics.load(Ordering::Relaxed));
    fixture.server.shutdown().await;
}
