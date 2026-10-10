use super::{
    World,
    spawn_test_support::{proto, publish},
};
use crate::{
    entity::{
        EntityBase, NBTStorage, RemovalReason,
        ai::goal::active_target::ActiveTargetGoal,
        death_test_world::DeathTestWorld,
        living::LivingEntity,
        player::{Player, RespawnPoint},
        r#type::from_type,
    },
    net::{
        ClientPlatform, PlayerConfig,
        java::{JavaClient, combat_test_support::TestPlayer},
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_change_world::PlayerChangeWorldEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block, biome::Biome, damage::DamageType, dimension::Dimension, entity::EntityType,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::java::server::play::{SClientCommand, SConfirmTeleport, SPlayerPosition};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::sync::Notify;

#[path = "player_lifecycle_save_tests.rs"]
mod save_tests;

fn replacement_player(player: &Arc<Player>, world: &Arc<World>) -> Arc<Player> {
    let profile = player.gameprofile.clone();
    Arc::new(Player::new(
        Arc::new(ClientPlatform::Java(JavaClient::without_connection(
            profile.clone(),
        ))),
        profile,
        PlayerConfig::default(),
        world,
        GameMode::Survival,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_killed_jockey_mount_leaves_zombie_after_chunk_reload() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let chicken = from_type(
            &EntityType::CHICKEN,
            Vector3::new(8.5, 64.0, 8.5),
            &world,
            uuid::Uuid::new_v4(),
        );
        let zombie = from_type(
            &EntityType::ZOMBIE,
            Vector3::new(8.5, 65.0, 8.5),
            &world,
            uuid::Uuid::new_v4(),
        );
        let zombie_uuid = zombie.get_entity().entity_uuid;
        let chicken_uuid = chicken.get_entity().entity_uuid;
        let mut root = NbtCompound::new();
        chicken.write_nbt(&mut root);
        let mut rider = NbtCompound::new();
        zombie.write_nbt(&mut rider);
        rider.put_bool("IsBaby", true);
        // Vanilla and older fork builds store riders under the root's Passengers list.
        root.put("Passengers", NbtTag::List(vec![NbtTag::Compound(rider)]));
        let pos = Vector2::new(0, 0);
        let chunk = world.level.get_entity_chunk(pos).await.unwrap();
        chunk.data.lock().unwrap().push(root);
        world.make_chunk_entities_live(&chunk, None);
        let chicken = world.get_entity_by_uuid(chicken_uuid).unwrap();
        chicken.get_living_entity().unwrap().damage(
            chicken.as_ref(),
            f32::MAX,
            DamageType::GENERIC_KILL,
        );
        for _ in 0..20 {
            chicken.tick(chicken.as_ref(), &fixture.server);
        }
        assert!(world.get_entity_by_uuid(chicken_uuid).is_none());
        world.remove_entities_in_chunks([pos]).await;
        assert!(
            chunk
                .data
                .lock()
                .unwrap()
                .iter()
                .any(|nbt| nbt.get_uuid("UUID") == Some(zombie_uuid))
        );
        world.level.clean_entity_chunks([pos]);
        let root_folder = world.level.level_folder.root_folder.clone();
        fixture.server.shutdown().await;
        let level = pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            root_folder,
            0,
            Dimension::OVERWORLD,
        );
        let restored = Arc::new(World::load(
            level,
            world.level_info.clone(),
            Dimension::OVERWORLD,
            crate::block::registry::default_registry(),
            Weak::new(),
        ));
        let chunk = restored.level.get_entity_chunk(pos).await.unwrap();
        restored.make_chunk_entities_live(&chunk, None);
        let survivor = restored.get_entity_by_uuid(zombie_uuid);
        restored.level.shutdown().await.unwrap();
        let survivor = survivor.expect("surviving jockey was omitted from the saved chunk");
        assert!(survivor.get_living_entity().unwrap().is_alive());
        assert!(!survivor.get_entity().has_vehicle());
        assert!(restored.get_entity_by_uuid(chicken_uuid).is_none());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_skeleton_drops_disconnected_target_on_next_ai_tick() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let player = fixture.player("Target");
        player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
        let skeleton = fixture.mob(&EntityType::SKELETON);
        skeleton.get_entity().set_pos(Vector3::new(12.5, 64.0, 8.5));
        let mob = skeleton.get_mob().unwrap();
        let mob_entity = mob.get_mob_entity();
        mob_entity.set_item_slot(
            &pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
            pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::BOW),
        );
        crate::entity::mob::skeleton::weapon_goal::reassess_weapon_goal(mob);
        // Remove random acquisition delay, but run the real target selector and mob AI step.
        {
            let mut selector = mob_entity.target_selector.lock().unwrap();
            selector.clear();
            selector.add_goal(
                2,
                Box::new(ActiveTargetGoal::new(
                    mob_entity,
                    &EntityType::PLAYER,
                    0,
                    false,
                    false,
                    None::<fn(&LivingEntity, &World) -> bool>,
                )),
            );
        };
        mob_entity.server_ai_step(mob, skeleton.as_ref());
        assert!(mob_entity.get_target().is_some());
        // RangedBowAttackGoal draws for 20 ticks; log out one tick before release.
        for _ in 0..19 {
            mob_entity.server_ai_step(mob, skeleton.as_ref());
        }
        let (session, _) = fixture
            .server
            .player_data_storage
            .load_data(&player.gameprofile.id)
            .await
            .unwrap();
        *player.storage_session.lock().unwrap() = Some(session);
        fixture
            .server
            .player_data_storage
            .handle_player_leave(&player, &fixture.server)
            .await
            .unwrap();
        mob_entity.server_ai_step(mob, skeleton.as_ref());
        assert!(
            mob_entity.get_target().is_none(),
            "skeleton retained the logged-out player"
        );
        assert!(matches!(
            player.get_entity().removal_reason.load(),
            Some(RemovalReason::UnloadedWithPlayer)
        ));
        for _ in 0..20 {
            mob_entity.server_ai_step(mob, skeleton.as_ref());
        }
        assert!(
            world
                .entities
                .load()
                .iter()
                .all(|entity| entity.get_entity().entity_type != &EntityType::ARROW)
        );
        player.storage_session.lock().unwrap().take();
        fixture.server.shutdown().await;
    })
    .await
    .unwrap();
}

struct PauseTransfer {
    reached: Notify,
    release: Notify,
}

struct ReplaceSession;
impl EventHandler<PlayerChangeWorldEvent> for ReplaceSession {
    fn handle<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // Simulate the obsolete task resuming after the UUID's session was replaced.
            let replacement = replacement_player(&event.player, &event.previous_world);
            event
                .previous_world
                .entity_tracker
                .remove_entity(event.player.as_ref(), &event.previous_world);
            event
                .previous_world
                .players
                .store(Arc::new(vec![replacement.clone()]));
            event
                .previous_world
                .entity_tracker
                .add_entity(&(replacement as Arc<dyn EntityBase>), &event.previous_world);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_stale_respawn_cannot_remove_or_publish_over_rejoined_session() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let destination = fixture.world();
        publish(&destination, proto(&Biome::PLAINS, &Block::STONE));
        let source = fixture
            .server
            .get_world_from_dimension(&Dimension::THE_NETHER);
        let mut client = TestPlayer::new(&source);
        let player = client.player.clone();
        player.living_entity.health.store(0.0);
        *player.respawn_point.lock().unwrap() = Some(RespawnPoint {
            dimension: Dimension::OVERWORLD,
            position: BlockPos::new(8, 64, 8),
            yaw: 0.0,
            force: true,
        });
        fixture.server.plugin_manager.register(
            Arc::new(ReplaceSession),
            EventPriority::Normal,
            false,
        );
        client
            .with_outgoing_writer(source.respawn_player(&player, false))
            .await;
        let replacement = source
            .get_player_by_uuid(player.gameprofile.id)
            .expect("stale respawn removed the rejoined session");
        assert!(!Arc::ptr_eq(&replacement, &player));
        assert!(!destination.contains_player_instance(&player));
        assert!(
            destination
                .entity_tracker
                .get_tracked_entity(player.entity_id())
                .is_none()
        );
        assert!(
            source
                .entity_tracker
                .get_tracked_entity(replacement.entity_id())
                .is_some()
        );
        fixture.server.shutdown().await;
    })
    .await
    .unwrap();
}
impl EventHandler<PlayerChangeWorldEvent> for PauseTransfer {
    fn handle<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.reached.notify_one();
            self.release.notified().await;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_respawn_racing_disconnect_is_joined_before_logout() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let destination = fixture.world();
        publish(&destination, proto(&Biome::PLAINS, &Block::STONE));
        let source = fixture
            .server
            .get_world_from_dimension(&Dimension::THE_NETHER);
        let mut client = TestPlayer::new(&source);
        let player = client.player.clone();
        player.living_entity.health.store(0.0);
        *player.respawn_point.lock().unwrap() = Some(RespawnPoint {
            dimension: Dimension::OVERWORLD,
            position: BlockPos::new(8, 64, 8),
            yaw: 0.0,
            force: true,
        });
        let (session, _) = fixture
            .server
            .player_data_storage
            .load_data(&player.gameprofile.id)
            .await
            .unwrap();
        *player.storage_session.lock().unwrap() = Some(session);
        let pause = Arc::new(PauseTransfer {
            reached: Notify::new(),
            release: Notify::new(),
        });
        fixture
            .server
            .plugin_manager
            .register(pause.clone(), EventPriority::Normal, false);
        client.client().handle_client_status(
            &player,
            &SClientCommand::new(SClientCommand::PERFORM_RESPAWN.into()),
        );
        client.with_outgoing_writer(pause.reached.notified()).await;
        client.client().close();
        let joined = client.client().await_tasks();
        tokio::pin!(joined);
        let pending = std::future::Future::poll(
            joined.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        )
        .is_pending();
        // Always release before asserting, including when the scheduler fix is reverted.
        pause.release.notify_one();
        joined.await;
        assert!(
            pending,
            "disconnect did not join the in-flight respawn task"
        );
        fixture
            .server
            .player_data_storage
            .handle_player_leave(&player, &fixture.server)
            .await
            .unwrap();
        player.storage_session.lock().unwrap().take();
        assert!(!destination.contains_player_instance(&player));
        assert!(!source.contains_player_instance(&player));
        assert!(
            destination
                .entity_tracker
                .get_tracked_entity(player.entity_id())
                .is_none()
        );
        // Rejoin goes through storage's real login gate after the old final save.
        let (session, saved) = fixture
            .server
            .player_data_storage
            .load_data(&player.gameprofile.id)
            .await
            .unwrap();
        let replacement = replacement_player(&player, &destination);
        NBTStorage::read_nbt_non_mut(replacement.as_ref(), &saved.unwrap());
        *replacement.storage_session.lock().unwrap() = Some(session);
        destination
            .players
            .store(Arc::new(vec![replacement.clone()]));
        assert!(destination.remove_player(&player, false).await.is_none());
        assert!(destination.contains_player_instance(&replacement));
        replacement.storage_session.lock().unwrap().take();
        fixture.server.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_stale_logout_does_not_remove_rejoined_uuid() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        let old = fixture.player("OldSession");
        let (session, _) = fixture
            .server
            .player_data_storage
            .load_data(&old.gameprofile.id)
            .await
            .unwrap();
        *old.storage_session.lock().unwrap() = Some(session);
        let replacement = replacement_player(&old, &world);
        world.players.store(Arc::new(vec![replacement.clone()]));
        fixture
            .server
            .player_data_storage
            .handle_player_leave(&old, &fixture.server)
            .await
            .unwrap();
        assert!(world.remove_player(&old, true).await.is_none());
        assert!(world.contains_player_instance(&replacement));
        assert!(old.get_entity().is_removed());
        assert!(!replacement.get_entity().is_removed());
        old.storage_session.lock().unwrap().take();
        fixture.server.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_death_and_respawn_release_riding_and_accept_position() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let mut client = TestPlayer::new(&world);
        let player = client.player.clone();
        player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
        *player.respawn_point.lock().unwrap() = Some(RespawnPoint {
            dimension: Dimension::OVERWORLD,
            position: BlockPos::new(8, 64, 8),
            yaw: 0.0,
            force: true,
        });
        let mount = fixture.mob(&EntityType::MINECART);
        mount
            .get_entity()
            .add_passenger(mount.clone(), player.clone());
        player
            .living_entity
            .damage(player.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
        assert!(
            !player.get_entity().has_vehicle(),
            "death retained the mount link"
        );
        assert!(!mount.get_entity().has_passengers());
        // An older build's death screen can still have riding links at respawn time.
        mount
            .get_entity()
            .add_passenger(mount.clone(), player.clone());
        client.client().handle_client_status(
            &player,
            &SClientCommand::new(SClientCommand::PERFORM_RESPAWN.into()),
        );
        client
            .with_outgoing_writer(player.client.java().unwrap().await_tasks())
            .await;
        assert!(
            !player.get_entity().has_vehicle(),
            "respawn retained the old mount link"
        );
        assert!(!mount.get_entity().has_passengers());
        confirm_teleport_and_move(&client, &player, &fixture.server);
        fixture.server.shutdown().await;
    })
    .await
    .unwrap();
}

fn confirm_teleport_and_move(client: &TestPlayer, player: &Arc<Player>, server: &Arc<Server>) {
    let awaiting = *player.awaiting_teleport.lock().unwrap();
    if let Some((id, _)) = awaiting {
        // Release the guard before the real acknowledgement handler re-locks it.
        let confirm = SConfirmTeleport {
            teleport_id: id,
            position: player.position(),
            yaw: player.get_entity().yaw.load(),
            pitch: player.get_entity().pitch.load(),
        };
        client.client().handle_confirm_teleport(player, &confirm);
    }
    player.set_client_loaded(true);
    let position = player.position() + Vector3::new(1.0, 0.0, 0.0);
    client.client().handle_position(
        player,
        server,
        &SPlayerPosition {
            position,
            collision: 1,
        },
    );
    assert_eq!(
        player.position(),
        position,
        "ordinary position packet was ignored"
    );
}

#[tokio::test]
async fn lifecycle_removal_reasons_preserve_parent_only_for_nondestructive_removal() {
    tokio::time::timeout(Duration::from_secs(60), async {
        use super::spawn_test_support::Fixture;
        use crate::entity::spawn_mount;
        let fixture = Fixture::new();
        for reason in [
            RemovalReason::Killed,
            RemovalReason::Discarded,
            RemovalReason::UnloadedToChunk,
            RemovalReason::UnloadedWithPlayer,
            RemovalReason::ChangedDimension,
        ] {
            let world = &fixture.world;
            let parent = from_type(
                &EntityType::CHICKEN,
                Vector3::new(8.5, 64.0, 8.5),
                world,
                uuid::Uuid::new_v4(),
            );
            let removed = from_type(
                &EntityType::CHICKEN,
                Vector3::new(8.5, 65.0, 8.5),
                world,
                uuid::Uuid::new_v4(),
            );
            let rider = from_type(
                &EntityType::ZOMBIE,
                Vector3::new(8.5, 66.0, 8.5),
                world,
                uuid::Uuid::new_v4(),
            );
            spawn_mount::attach_unpublished(&parent, removed.clone());
            spawn_mount::attach_unpublished(&removed, rider.clone());
            assert!(world.spawn_entity_with_passengers(&parent));
            world.remove_entity_with_reason(removed.as_ref(), reason);
            assert!(removed.get_entity().removal_reason.load() == Some(reason));
            assert_eq!(removed.get_entity().has_vehicle(), !reason.should_destroy());
            assert!(!removed.get_entity().has_passengers());
            assert!(!rider.get_entity().has_vehicle());
            // A repeated discard must not replace the reason (including shouldSave semantics).
            world.remove_entity(removed.as_ref());
            assert!(removed.get_entity().removal_reason.load() == Some(reason));
            world.remove_entity(parent.as_ref());
            world.remove_entity(rider.as_ref());
        }
        fixture.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_spectator_dismounts_and_accepts_position() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let client = TestPlayer::new(&world);
        let player = &client.player;
        player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
        let mount = fixture.mob(&EntityType::MINECART);
        mount
            .get_entity()
            .add_passenger(mount.clone(), player.clone());
        assert!(player.set_gamemode(GameMode::Spectator));
        assert!(!player.get_entity().has_vehicle());
        assert!(!mount.get_entity().has_passengers());
        confirm_teleport_and_move(&client, player, &fixture.server);
        fixture.server.shutdown().await;
    })
    .await
    .unwrap();
}
