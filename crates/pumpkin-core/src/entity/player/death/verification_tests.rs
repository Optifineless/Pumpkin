#![expect(
    clippy::unwrap_used,
    reason = "Respawn regression fixtures must be valid"
)]

use super::external_review_tests::{nether, prepare};
use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, player::player_respawn::PlayerRespawnEvent},
    server::{
        Server,
        combat_test_support::{server, world},
    },
    world::World,
};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::level::Level;
use std::sync::Arc;

// Probe the real watcher map by consuming and restoring exactly the expected references.
pub(super) async fn assert_watcher_count(level: &Level, chunk: Vector2<i32>, count: usize) {
    assert!(level.is_chunk_watched(&chunk));
    for remaining in (0..count).rev() {
        assert_eq!(
            level.mark_chunk_as_not_watched(chunk).await,
            remaining == 0,
            "wrong watcher count at {chunk:?}"
        );
    }
    assert!(!level.is_chunk_watched(&chunk));
    for _ in 0..count {
        level.mark_chunks_as_newly_watched(&[chunk]).await;
    }
}

struct Teleport(Arc<World>);
impl EventHandler<PlayerRespawnEvent> for Teleport {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert!(event.player.living_entity.is_respawning());
            event
                .player
                .teleport_world(self.0.clone(), event.position, None, None)
                .await;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_respawn_callback_teleport_keeps_exactly_one_world_membership() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let target = world(&server, dir.path());
    let other = nether(&server, &dir.path().join("nether"));
    server
        .worlds
        .store(Arc::new(vec![target.clone(), other.clone()]));
    let mut fixture = TestPlayer::new(&target);
    let player = fixture.player.clone();
    prepare(&target, &player);
    prepare(&other, &player);
    server.plugin_manager.register::<PlayerRespawnEvent, _>(
        Arc::new(Teleport(other.clone())),
        EventPriority::Normal,
        true,
    );
    fixture
        .collect_packets_during(target.respawn_player(&player, false))
        .await;
    assert!(!player.living_entity.is_respawning());
    assert_eq!(player.world().uuid, target.uuid);
    assert_eq!(
        target
            .players
            .load()
            .iter()
            .filter(|p| p.entity_id() == player.entity_id())
            .count(),
        1
    );
    assert!(
        other
            .players
            .load()
            .iter()
            .all(|p| p.entity_id() != player.entity_id())
    );
    assert!(!other.entity_tracker.has_entity_with_id(player.entity_id()));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_respawn_publication_rejects_a_changed_target_world() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let target = world(&server, dir.path());
    let other = nether(&server, &dir.path().join("nether"));
    let fixture = TestPlayer::new(&target);
    let player = &fixture.player;
    assert!(player.living_entity.begin_respawn().is_some());
    player.get_entity().set_world(other.clone());
    assert!(!player.living_entity.publish_respawn(
        player,
        &target,
        &target,
        Vector3::new(4.5, 64.0, 4.5),
        0.0,
        0.0
    ));
    assert!(target.players.load().is_empty());
    assert!(other.players.load().is_empty());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_ordinary_disconnect_does_not_abort_a_non_respawning_life() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let lifecycle = player.living_entity.damage_lifecycle();
    assert!(!player.living_entity.abort_respawn());
    assert_eq!(player.living_entity.damage_lifecycle(), lifecycle);
    assert!(!player.get_entity().is_removed());
    assert!(player.get_entity().removal_reason.load().is_none());
    crate::server::fixture_lifecycle::finish().await;
}
