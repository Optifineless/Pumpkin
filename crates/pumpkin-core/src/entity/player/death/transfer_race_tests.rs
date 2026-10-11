#![expect(clippy::unwrap_used, reason = "Respawn race fixtures must be valid")]

use super::{
    external_review_tests::{nether, prepare},
    verification_tests::assert_watcher_count,
};
use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        player::{
            player_respawn::PlayerRespawnEvent, player_spawn_location::PlayerSpawnLocationEvent,
        },
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
    world::World,
};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::{num::NonZeroU8, sync::Arc};

struct SpawnTeleport(Arc<World>);
impl EventHandler<PlayerSpawnLocationEvent> for SpawnTeleport {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerSpawnLocationEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            event
                .player
                .teleport_world(self.0.clone(), event.spawn_pos, None, None)
                .await;
            assert_eq!(event.player.world().uuid, self.0.uuid);
        })
    }
}

struct RespawnSource(Arc<World>);
impl EventHandler<PlayerRespawnEvent> for RespawnSource {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.previous_world.uuid, self.0.uuid);
        Box::pin(async {})
    }
}

async fn prepare_views(worlds: &[Arc<World>], player: &Player) -> Vec<Vector2<i32>> {
    let mut config = (**player.config.load()).clone();
    config.view_distance = NonZeroU8::new(2).unwrap();
    player.config.store(Arc::new(config));
    let chunks: Vec<_> = player.watched_section.load().all_chunks_within().collect();
    for world in worlds {
        prepare(world, player);
        for pos in &chunks {
            world
                .level
                .loaded_chunks
                .entry(*pos)
                .or_insert_with(|| ChunkData::empty_sync(pos.x, pos.y));
        }
        // One independent viewer in every world; the player's source gets a second reference.
        world.level.mark_chunks_as_newly_watched(&chunks).await;
    }
    player
        .world()
        .level
        .mark_chunks_as_newly_watched(&chunks)
        .await;
    chunks
}

async fn assert_published(
    player: &Player,
    worlds: &[Arc<World>],
    target: &World,
    chunks: &[Vector2<i32>],
) {
    player.await_chunk_watch_updates().await;
    assert!(!player.living_entity.is_respawning());
    assert_eq!(player.world().uuid, target.uuid);
    let mut memberships = 0;
    for world in worlds {
        let count = world
            .players
            .load()
            .iter()
            .filter(|p| p.entity_id() == player.entity_id())
            .count();
        memberships += count;
        assert_eq!(count, usize::from(world.uuid == target.uuid));
        assert_eq!(
            world.entity_tracker.has_entity_with_id(player.entity_id()),
            world.uuid == target.uuid
        );
        for chunk in chunks {
            assert_watcher_count(
                &world.level,
                *chunk,
                1 + usize::from(world.uuid == target.uuid),
            )
            .await;
        }
    }
    assert_eq!(memberships, 1);
}

async fn spawn_callback_teleport(cross_dimension: bool) {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source = world(&server, dir.path());
    let other_path = dir.path().join("other");
    let other = if cross_dimension {
        nether(&server, &other_path)
    } else {
        world(&server, &other_path)
    };
    let worlds = [source.clone(), other.clone()];
    server.worlds.store(Arc::new(worlds.to_vec()));
    let mut fixture = TestPlayer::new(&source);
    let player = fixture.player.clone();
    let chunks = prepare_views(&worlds, &player).await;
    server
        .plugin_manager
        .register::<PlayerSpawnLocationEvent, _>(
            Arc::new(SpawnTeleport(other.clone())),
            EventPriority::Normal,
            true,
        );
    server.plugin_manager.register::<PlayerRespawnEvent, _>(
        Arc::new(RespawnSource(other.clone())),
        EventPriority::Normal,
        true,
    );
    fixture
        .collect_packets_during(source.respawn_player(&player, false))
        .await;
    assert_published(&player, &worlds, &source, &chunks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup5_spawn_callback_teleport_between_overworlds_retains_respawn_world() {
    spawn_callback_teleport(false).await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup4_spawn_callback_teleport_cross_dimension_balances_watchers() {
    spawn_callback_teleport(true).await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup4_inflight_teleport_cannot_be_detached_by_respawn() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source = world(&server, dir.path());
    let other = nether(&server, &dir.path().join("other"));
    let worlds = [source.clone(), other.clone()];
    server.worlds.store(Arc::new(worlds.to_vec()));
    let mut fixture = TestPlayer::new(&source);
    let player = fixture.player.clone();
    let chunks = prepare_views(&worlds, &player).await;
    let (entered, resume) = crate::entity::player::chunk_tracking_tests::pause(&player);
    fixture
        .collect_packets_during(async {
            tokio::join!(
                player.teleport_world(other.clone(), Vector3::new(4.5, 64.0, 4.5), None, None),
                async {
                    entered.notified().await;
                    assert!(
                        player.living_entity.begin_respawn().is_none(),
                        "respawn detached an in-flight transfer"
                    );
                    let waiting = player.living_entity.notify_when_respawn_waits();
                    let respawn = source.respawn_player(&player, false);
                    tokio::pin!(respawn);
                    tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        tokio::select! {
                            () = &mut respawn => panic!("respawn did not wait for the transfer"),
                            () = waiting.notified() => {}
                        }
                    })
                    .await
                    .unwrap();
                    assert!(!player.living_entity.is_respawning());
                    resume.notify_one();
                    respawn.await;
                }
            );
        })
        .await;
    assert_published(&player, &worlds, &source, &chunks).await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup4_rejected_publication_disconnects_and_finishes_teardown() {
    for changed_world in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let source = world(&server, dir.path());
        let other = nether(&server, &dir.path().join("other"));
        let worlds = [source.clone(), other.clone()];
        let mut fixture = TestPlayer::new(&source);
        let player = fixture.player.clone();
        let chunks = prepare_views(&worlds, &player).await;
        let center = player.get_entity().chunk_pos.load();
        let view_level = pumpkin_world::chunk_system::ChunkLoading::get_level_from_view_distance(
            player.config.load().view_distance.get() + 1,
        );
        for world in &worlds {
            world
                .level
                .chunk_loading
                .lock()
                .unwrap()
                .add_ticket(center, view_level);
        }
        source
            .level
            .chunk_loading
            .lock()
            .unwrap()
            .add_ticket(center, view_level);
        *player.held_chunk_tickets.lock().unwrap() = Some((Some(view_level), None));
        assert!(player.living_entity.begin_respawn().is_some());
        if changed_world {
            player.get_entity().set_world(other.clone());
        }
        let rejected_target = if changed_world { &source } else { &other };
        fixture
            .collect_packets_during(async {
                assert!(
                    !player
                        .living_entity
                        .complete_respawn(
                            &player,
                            rejected_target,
                            &source,
                            Vector3::new(4.5, 64.0, 4.5),
                            0.0,
                            0.0,
                        )
                        .await
                );
            })
            .await;
        assert!(player.client.closed());
        assert!(!player.living_entity.is_respawning());
        assert!(player.get_entity().is_removed());
        for world in &worlds {
            assert_eq!(
                world
                    .level
                    .chunk_loading
                    .lock()
                    .unwrap()
                    .ticket
                    .get(&center)
                    .unwrap()
                    .len(),
                1
            );
            assert!(world.players.load().is_empty());
            assert!(!world.entity_tracker.has_entity_with_id(player.entity_id()));
            for chunk in &chunks {
                assert_watcher_count(&world.level, *chunk, 1).await;
            }
        }
    }
    crate::server::fixture_lifecycle::finish().await;
}
