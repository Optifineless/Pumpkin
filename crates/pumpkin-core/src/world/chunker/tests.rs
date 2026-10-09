#![expect(
    clippy::unwrap_used,
    reason = "Chunk movement race fixtures must be valid"
)]

use super::update_position;
use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_util::math::vector3::Vector3;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};
use tokio::{sync::Notify, time::timeout};

type Hook = Box<dyn FnOnce() + Send>;
type Hooks = OnceLock<Mutex<HashMap<i32, Hook>>>;
static TRACKER_HOOKS: Hooks = OnceLock::new();
static TICKET_HOOKS: Hooks = OnceLock::new();
static CLEANUP_HOOKS: Hooks = OnceLock::new();
const TIMEOUT: Duration = Duration::from_secs(5);

fn run_hook(hooks: &Hooks, entity_id: i32) {
    let hook = hooks
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .remove(&entity_id);
    if let Some(hook) = hook {
        hook();
    }
}

fn install(hooks: &Hooks, entity_id: i32, hook: impl FnOnce() + Send + 'static) {
    hooks
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .insert(entity_id, Box::new(hook));
}

pub(super) fn before_tracker_update(entity_id: i32) {
    run_hook(&TRACKER_HOOKS, entity_id);
}

pub(super) fn between_ticket_locks(entity_id: i32) {
    run_hook(&TICKET_HOOKS, entity_id);
}

pub fn before_ticket_cleanup(entity_id: i32) {
    run_hook(&CLEANUP_HOOKS, entity_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup5_disconnect_ticket_cleanup_racing_movement_completes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    player.get_entity().set_pos(Vector3::new(40.5, 64.0, 4.5));
    let entered = Arc::new(Notify::new());
    let cleanup_entered = Arc::new(Notify::new());
    let (resume, paused) = mpsc::channel();
    let level = world.level.clone();
    install(&TICKET_HOOKS, player.entity_id(), {
        let entered = entered.clone();
        move || {
            entered.notify_one();
            paused.recv_timeout(TIMEOUT).unwrap();
            // Fail before blocking on the second lock, releasing held tickets even on regression.
            assert!(
                level.chunk_loading.try_lock().is_ok(),
                "disconnect took the level lock while movement held the player's tickets"
            );
        }
    });
    install(&CLEANUP_HOOKS, player.entity_id(), {
        let cleanup_entered = cleanup_entered.clone();
        move || cleanup_entered.notify_one()
    });
    fixture
        .collect_packets_during(async {
            let movement = tokio::task::spawn_blocking({
                let player = player.clone();
                move || update_position(&player)
            });
            timeout(TIMEOUT, entered.notified()).await.unwrap();
            let cleanup = tokio::task::spawn_blocking({
                let player = player.clone();
                let level = world.level.clone();
                move || player.clean_up_chunk_tickets(&level)
            });
            timeout(TIMEOUT, cleanup_entered.notified()).await.unwrap();
            resume.send(()).unwrap();
            let (movement, cleanup) = timeout(TIMEOUT, async { tokio::join!(movement, cleanup) })
                .await
                .unwrap();
            movement.unwrap();
            cleanup.unwrap();
            timeout(TIMEOUT, player.remove()).await.unwrap();
        })
        .await;
    assert!(player.held_chunk_tickets.lock().unwrap().is_none());
    assert!(world.players.load().is_empty());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup5_tracker_walk_does_not_hold_combat_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let entered = Arc::new(Notify::new());
    let (resume, paused) = mpsc::channel();
    install(&TRACKER_HOOKS, player.entity_id(), {
        let entered = entered.clone();
        move || {
            entered.notify_one();
            paused.recv_timeout(TIMEOUT).unwrap();
        }
    });
    let movement = tokio::task::spawn_blocking({
        let player = player.clone();
        move || update_position(&player)
    });
    timeout(TIMEOUT, entered.notified()).await.unwrap();
    let (owned, ownership) = tokio::sync::oneshot::channel();
    let combat = tokio::task::spawn_blocking({
        let player = player.clone();
        move || {
            let _owner = player.living_entity.own_damage();
            let _ = owned.send(());
        }
    });
    let acquired = timeout(Duration::from_secs(1), ownership).await;
    if acquired.is_ok() {
        // Teardown wins while the tracker walk is paused; the view commit must recheck it.
        timeout(TIMEOUT, player.unload_watched_chunks(&world))
            .await
            .unwrap();
    }
    resume.send(()).unwrap();
    timeout(TIMEOUT, movement).await.unwrap().unwrap();
    timeout(TIMEOUT, combat).await.unwrap().unwrap();
    assert!(acquired.is_ok(), "tracker walk blocked combat ownership");
    assert_eq!(player.watched_section.load().all_chunks_within().len(), 0);
    assert!(player.held_chunk_tickets.lock().unwrap().is_none());
    crate::server::fixture_lifecycle::finish().await;
}
