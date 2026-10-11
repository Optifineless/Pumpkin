#![expect(
    clippy::unwrap_used,
    reason = "Chunk teardown regression fixtures must be valid"
)]

use super::Player;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::Notify;

type Pause = (Arc<Notify>, Arc<Notify>);
static PAUSES: OnceLock<Mutex<HashMap<i32, Pause>>> = OnceLock::new();
static UPDATE_PAUSES: OnceLock<Mutex<HashMap<i32, Pause>>> = OnceLock::new();

pub(super) async fn pause_update(entity_id: i32) {
    let pause = UPDATE_PAUSES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .remove(&entity_id);
    if let Some((entered, resume)) = pause {
        entered.notify_one();
        resume.notified().await;
    }
}

pub(super) async fn pause_unload(entity_id: i32) {
    let pause = PAUSES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .remove(&entity_id);
    if let Some((entered, resume)) = pause {
        entered.notify_one();
        resume.notified().await;
    }
}
pub(super) fn pause(player: &Player) -> Pause {
    let pause = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    PAUSES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .insert(player.entity_id(), pause.clone());
    pause
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_disconnect_during_chunk_unload_decrements_watchers_once() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let chunks: Vec<_> = player.watched_section.load().all_chunks_within().collect();
    world.level.mark_chunks_as_newly_watched(&chunks).await;
    world.level.mark_chunks_as_newly_watched(&chunks).await;
    let (entered, resume) = pause(&player);
    fixture
        .collect_packets_during(async {
            tokio::join!(player.unload_watched_chunks(&world), async {
                entered.notified().await;
                assert_eq!(player.watched_section.load().all_chunks_within().len(), 0);
                player.remove().await;
                resume.notify_one();
            });
        })
        .await;
    for chunk in chunks {
        assert!(world.level.is_chunk_watched(&chunk));
        assert!(
            world.level.mark_chunk_as_not_watched(chunk).await,
            "source watcher decremented twice"
        );
    }
    player.unload_watched_chunks(&world).await;
    assert_eq!(player.watched_section.load().all_chunks_within().len(), 0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup4_movement_during_chunk_teardown_cannot_rewatch_the_empty_view() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let chunks: Vec<_> = player.watched_section.load().all_chunks_within().collect();
    world.level.mark_chunks_as_newly_watched(&chunks).await;
    world.level.mark_chunks_as_newly_watched(&chunks).await;
    let (entered, resume) = pause(&player);
    fixture
        .collect_packets_during(async {
            tokio::join!(player.unload_watched_chunks(&world), async {
                entered.notified().await;
                crate::world::chunker::update_position(&player);
                assert_eq!(player.watched_section.load().all_chunks_within().len(), 0);
                resume.notify_one();
            });
        })
        .await;
    player.await_chunk_watch_updates().await;
    for chunk in chunks {
        assert!(
            world.level.mark_chunk_as_not_watched(chunk).await,
            "movement leaked a watcher"
        );
        assert!(!world.level.is_chunk_watched(&chunk));
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup4_chunk_teardown_waits_for_pending_watcher_additions() {
    use crate::entity::EntityBase;
    use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
    use pumpkin_world::cylindrical_chunk_iterator::Cylindrical;
    use std::{collections::HashSet, num::NonZeroU8};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let mut config = (**player.config.load()).clone();
    config.view_distance = NonZeroU8::new(2).unwrap();
    player.config.store(Arc::new(config));
    let old_view = player.watched_section.load();
    let new_view = Cylindrical::new(Vector2::new(2, 0), NonZeroU8::new(2).unwrap());
    let probe = Cylindrical::changed_chunks(old_view, new_view)
        .0
        .next()
        .unwrap();
    let all_chunks: HashSet<_> = old_view
        .all_chunks_within()
        .chain(new_view.all_chunks_within())
        .collect();
    let all_chunks: Vec<_> = all_chunks.into_iter().collect();
    world.level.mark_chunks_as_newly_watched(&all_chunks).await;
    let old_chunks: Vec<_> = old_view.all_chunks_within().collect();
    world.level.mark_chunks_as_newly_watched(&old_chunks).await;
    let (entered, resume) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    UPDATE_PAUSES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .insert(player.entity_id(), (entered.clone(), resume.clone()));
    player.get_entity().set_pos(Vector3::new(40.5, 64.0, 4.5));
    crate::world::chunker::update_position(&player);
    entered.notified().await;
    fixture
        .collect_packets_during(async {
            let teardown = player.unload_watched_chunks(&world);
            tokio::pin!(teardown);
            assert!(
                futures::poll!(teardown.as_mut()).is_pending(),
                "teardown overtook watcher additions"
            );
            assert!(
                world.level.is_chunk_watched(&probe),
                "teardown removed another viewer before its own watcher was added"
            );
            // A concurrently empty claim must not steal the first teardown's pending completion.
            player.unload_watched_chunks(&world).await;
            resume.notify_one();
            teardown.await;
        })
        .await;
    for chunk in all_chunks {
        assert!(
            world.level.mark_chunk_as_not_watched(chunk).await,
            "watcher delta leaked at {chunk:?}"
        );
        assert!(!world.level.is_chunk_watched(&chunk));
    }
    crate::server::fixture_lifecycle::finish().await;
}
