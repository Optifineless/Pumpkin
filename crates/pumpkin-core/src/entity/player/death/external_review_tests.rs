#![expect(
    clippy::unwrap_used,
    reason = "Respawn regression fixtures must be valid"
)]

use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        player::{player_leave::PlayerLeaveEvent, player_respawn::PlayerRespawnEvent},
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
    world::World,
};
use pumpkin_data::{
    Block, dimension::Dimension, entity::EntityType, packet::clientbound::play::REMOVE_ENTITIES,
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{chunk::ChunkData, level::Level};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};
use tokio::sync::Notify;

pub(super) fn prepare(world: &World, player: &Player) {
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(12, 63, 12, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    player.set_respawn_point(
        Dimension::OVERWORLD,
        BlockPos::new(12, 64, 12),
        0.0,
        0.0,
        true,
    );
}

pub(super) fn nether(server: &Arc<Server>, path: &std::path::Path) -> Arc<World> {
    let world = Arc::new(World::load(
        Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            path.to_owned(),
            0,
            Dimension::THE_NETHER,
        ),
        server.level_info.clone(),
        Dimension::THE_NETHER,
        server.block_registry.clone(),
        Arc::downgrade(server),
    ));
    crate::server::fixture_lifecycle::track_world(&world);
    world
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_cross_world_respawn_removes_source_tracker_and_viewer() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let destination = world(&server, dir.path());
    let source = nether(&server, &dir.path().join("nether"));
    server
        .worlds
        .store(Arc::new(vec![destination.clone(), source.clone()]));
    let mut fixture = TestPlayer::new(&source);
    let player = fixture.player.clone();
    let mut observer = TestPlayer::new(&source);
    source
        .players
        .store(Arc::new(vec![player.clone(), observer.player.clone()]));
    prepare(&destination, &player);
    let cow = crate::entity::r#type::from_type(
        &EntityType::COW,
        player.position(),
        &source,
        uuid::Uuid::new_v4(),
    );
    source.add_entity_silent(cow.clone());
    let cow_tracker = source
        .entity_tracker
        .get_tracked_entity(cow.get_entity().entity_id)
        .unwrap();
    cow_tracker.seen_by.insert(player.gameprofile.id);
    let old_tracker = source
        .entity_tracker
        .get_tracked_entity(player.entity_id())
        .unwrap();
    old_tracker.seen_by.insert(observer.player.gameprofile.id);
    observer.take_packets();
    fixture
        .collect_packets_during(source.respawn_player(&player, false))
        .await;
    assert_eq!(player.world().uuid, destination.uuid);
    assert!(!source.entity_tracker.has_entity_with_id(player.entity_id()));
    assert!(!cow_tracker.seen_by.contains(&player.gameprofile.id));
    assert!(
        destination
            .entity_tracker
            .has_entity_with_id(player.entity_id())
    );
    assert!(
        destination
            .get_player_by_uuid(player.gameprofile.id)
            .is_some()
    );
    let removed = observer.take_packets().into_iter().any(|packet| {
        let mut bytes = packet.as_ref();
        bytes.get_var_int().unwrap().0 == REMOVE_ENTITIES.0
            && bytes.get_var_int().unwrap().0 == 1
            && bytes.get_var_int().unwrap().0 == player.entity_id()
    });
    assert!(
        removed,
        "source-world viewers never received player removal"
    );
    assert!(source.remove_player(&player, false).await.is_none());
    crate::server::fixture_lifecycle::finish().await;
}

struct Pause {
    entered: Arc<Notify>,
    resume: Arc<Notify>,
}
impl EventHandler<PlayerRespawnEvent> for Pause {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.entered.notify_one();
            self.resume.notified().await;
        })
    }
}

struct CountLeave(Arc<AtomicUsize>);
impl EventHandler<PlayerLeaveEvent> for CountLeave {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerLeaveEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_disconnect_during_respawn_tears_down_once_and_never_republishes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let observer = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![player.clone(), observer.player.clone()]));
    prepare(&world, &player);
    let observer_tracker = world
        .entity_tracker
        .get_tracked_entity(observer.player.entity_id())
        .unwrap();
    observer_tracker.seen_by.insert(player.gameprofile.id);
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let leaves = Arc::new(AtomicUsize::new(0));
    server.plugin_manager.register::<PlayerRespawnEvent, _>(
        Arc::new(Pause {
            entered: entered.clone(),
            resume: resume.clone(),
        }),
        EventPriority::Normal,
        true,
    );
    server.plugin_manager.register::<PlayerLeaveEvent, _>(
        Arc::new(CountLeave(leaves.clone())),
        EventPriority::Normal,
        true,
    );
    fixture
        .collect_packets_during(async {
            tokio::join!(world.respawn_player(&player, false), async {
                entered.notified().await;
                // Exercise permanent removal independently of the network close token.
                player.remove().await;
                assert!(!world.entity_tracker.has_entity_with_id(player.entity_id()));
                assert!(!observer_tracker.seen_by.contains(&player.gameprofile.id));
                assert!(world.remove_player(&player, true).await.is_none());
                assert_eq!(leaves.load(Relaxed), 1);
                resume.notify_one();
            });
        })
        .await;
    assert!(world.get_player_by_uuid(player.gameprofile.id).is_none());
    assert!(!world.entity_tracker.has_entity_with_id(player.entity_id()));
    assert!(!observer_tracker.seen_by.contains(&player.gameprofile.id));
    assert_eq!(leaves.load(Relaxed), 1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_disconnect_in_unpublished_destination_keeps_one_leave_callback() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let destination = world(&server, dir.path());
    let source = nether(&server, &dir.path().join("nether"));
    server
        .worlds
        .store(Arc::new(vec![destination.clone(), source.clone()]));
    let mut fixture = TestPlayer::new(&source);
    let player = fixture.player.clone();
    prepare(&destination, &player);
    let chunks: Vec<_> = player.watched_section.load().all_chunks_within().collect();
    for level in [&source.level, &destination.level] {
        // Two viewers in each world; only the source includes this player.
        level.mark_chunks_as_newly_watched(&chunks).await;
        level.mark_chunks_as_newly_watched(&chunks).await;
    }
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let leaves = Arc::new(AtomicUsize::new(0));
    server.plugin_manager.register::<PlayerRespawnEvent, _>(
        Arc::new(Pause {
            entered: entered.clone(),
            resume: resume.clone(),
        }),
        EventPriority::Normal,
        true,
    );
    server.plugin_manager.register::<PlayerLeaveEvent, _>(
        Arc::new(CountLeave(leaves.clone())),
        EventPriority::Normal,
        true,
    );
    fixture
        .collect_packets_during(async {
            tokio::join!(source.respawn_player(&player, false), async {
                entered.notified().await;
                assert_eq!(player.world().uuid, destination.uuid);
                assert!(server.get_player_by_uuid(player.gameprofile.id).is_none());
                assert!(
                    server
                        .get_all_players()
                        .iter()
                        .all(|p| p.entity_id() != player.entity_id())
                );
                assert!(!source.entity_tracker.has_entity_with_id(player.entity_id()));
                player.remove().await;
                for chunk in &chunks {
                    super::verification_tests::assert_watcher_count(&source.level, *chunk, 1).await;
                    super::verification_tests::assert_watcher_count(&destination.level, *chunk, 2)
                        .await;
                }
                assert!(destination.remove_player(&player, true).await.is_none());
                assert_eq!(leaves.load(Relaxed), 1);
                resume.notify_one();
            });
        })
        .await;
    for world in [&source, &destination] {
        assert!(world.get_player_by_uuid(player.gameprofile.id).is_none());
        assert!(!world.entity_tracker.has_entity_with_id(player.entity_id()));
    }
    assert_eq!(leaves.load(Relaxed), 1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_disconnect_with_stale_source_world_removes_the_published_destination() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let destination = world(&server, dir.path());
    let source = nether(&server, &dir.path().join("nether"));
    server
        .worlds
        .store(Arc::new(vec![destination.clone(), source.clone()]));
    let mut fixture = TestPlayer::new(&source);
    let player = fixture.player.clone();
    prepare(&destination, &player);
    fixture
        .collect_packets_during(source.respawn_player(&player, false))
        .await;
    assert_eq!(player.world().uuid, destination.uuid);
    assert!(!player.living_entity.is_respawning());
    let observer = TestPlayer::new(&destination);
    destination
        .players
        .store(Arc::new(vec![player.clone(), observer.player.clone()]));
    let observer_tracker = destination
        .entity_tracker
        .get_tracked_entity(observer.player.entity_id())
        .unwrap();
    observer_tracker.seen_by.insert(player.gameprofile.id);
    let leaves = Arc::new(AtomicUsize::new(0));
    server.plugin_manager.register::<PlayerLeaveEvent, _>(
        Arc::new(CountLeave(leaves.clone())),
        EventPriority::Normal,
        true,
    );
    // Player.remove may capture this source world just before the transfer publishes.
    source.remove_player(&player, true).await;
    for world in [&source, &destination] {
        assert!(world.get_player_by_uuid(player.gameprofile.id).is_none());
        assert!(!world.entity_tracker.has_entity_with_id(player.entity_id()));
    }
    assert!(!observer_tracker.seen_by.contains(&player.gameprofile.id));
    assert_eq!(leaves.load(Relaxed), 1);
    assert!(source.remove_player(&player, true).await.is_none());
    assert_eq!(leaves.load(Relaxed), 1);
    crate::server::fixture_lifecycle::finish().await;
}

struct HungerBeforeEvent;
impl EventHandler<PlayerRespawnEvent> for HungerBeforeEvent {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        let hunger = &event.player.hunger_manager;
        assert_eq!(hunger.level.load(), if event.alive { 0 } else { 20 });
        assert_eq!(
            hunger.saturation.load(),
            if event.alive { 0.0 } else { 5.0 }
        );
        hunger.set_level(7);
        hunger.set_saturation(2.0);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_respawn_restores_hunger_before_event_and_preserves_plugin_values() {
    for alive in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let mut fixture = TestPlayer::new(&world);
        let player = fixture.player.clone();
        prepare(&world, &player);
        player.hunger_manager.set_level(0);
        player.hunger_manager.set_saturation(0.0);
        server.plugin_manager.register::<PlayerRespawnEvent, _>(
            Arc::new(HungerBeforeEvent),
            EventPriority::Normal,
            true,
        );
        fixture
            .collect_packets_during(world.respawn_player(&player, alive))
            .await;
        assert_eq!(player.hunger_manager.level.load(), 7);
        assert_eq!(player.hunger_manager.saturation.load(), 2.0);
    }
    crate::server::fixture_lifecycle::finish().await;
}
