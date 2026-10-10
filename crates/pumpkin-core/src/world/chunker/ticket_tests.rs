use std::{
    collections::HashMap,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

use tokio::sync::oneshot;
use uuid::Uuid;

use super::*;
use crate::entity::death_test_world::DeathTestWorld;

type Gate = (oneshot::Sender<()>, mpsc::Receiver<()>);

#[derive(Default)]
struct Gates {
    cleanup: Option<Gate>,
    movement: Option<Gate>,
}

static GATES: OnceLock<Mutex<HashMap<Uuid, Gates>>> = OnceLock::new();

fn pause(player: &Player, cleanup: bool) {
    let gate = GATES.get().and_then(|gates| {
        let mut gates = gates.lock().unwrap();
        let gates = gates.get_mut(&player.get_entity().entity_uuid)?;
        if cleanup {
            gates.cleanup.take()
        } else {
            gates.movement.take()
        }
    });
    if let Some((ready, resume)) = gate {
        let _ = ready.send(());
        let _ = resume.recv();
    }
}

pub fn pause_cleanup(player: &Player) {
    pause(player, true);
}

pub fn pause_movement(player: &Player) {
    pause(player, false);
}

async fn concurrent_ticket_paths() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("ticket-lock-order");
    player.config.rcu(|config| {
        let mut config = (**config).clone();
        config.view_distance = NonZero::new(2).unwrap();
        config
    });
    player.gamemode.store(pumpkin_util::GameMode::Spectator);
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.spectators_generate_chunks = false;
        info
    });
    // Reproduce both real entry points eight times, at the opposing lock boundaries.
    for x in 1..=8 {
        player
            .get_entity()
            .set_pos(pumpkin_util::math::vector3::Vector3::new(
                f64::from(x * 16),
                64.0,
                0.0,
            ));
        let (cleanup_ready, cleanup_started) = oneshot::channel();
        let (cleanup_resume, cleanup_gate) = mpsc::channel();
        let (movement_ready, movement_started) = oneshot::channel();
        let (movement_resume, movement_gate) = mpsc::channel();
        GATES.get_or_init(Mutex::default).lock().unwrap().insert(
            player.get_entity().entity_uuid,
            Gates {
                cleanup: Some((cleanup_ready, cleanup_gate)),
                movement: Some((movement_ready, movement_gate)),
            },
        );
        let cleaning = player.clone();
        let level = fixture.world().level.clone();
        let cleanup = std::thread::spawn(move || cleaning.clean_up_chunk_tickets(&level));
        cleanup_started.await.unwrap();
        let moving = player.clone();
        let movement_runtime = tokio::runtime::Handle::current();
        let movement = std::thread::spawn(move || {
            let _runtime = movement_runtime.enter();
            update_position(&moving);
        });
        movement_started.await.unwrap();
        // Cleanup is just before held_chunk_tickets; movement already holds it.
        // With the old order cleanup owns chunk_loading here, forcing the cycle.
        cleanup_resume.send(()).unwrap();
        movement_resume.send(()).unwrap();
        movement.join().unwrap();
        cleanup.join().unwrap();
        assert!(
            fixture
                .world()
                .level
                .chunk_loading
                .lock()
                .unwrap()
                .ticket
                .is_empty()
        );
        assert!(player.held_chunk_tickets.lock().unwrap().is_none());
    }
    GATES
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .remove(&player.get_entity().entity_uuid);
    fixture.server.shutdown().await;
}

#[test]
fn movement_and_cleanup_follow_the_same_lock_order() {
    if std::env::var_os("PUMPKIN_TICKET_LOCK_CHILD").is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(concurrent_ticket_paths());
        return;
    }
    // A deadlocked synchronous mutex cannot be cancelled by a Tokio timeout.
    let log = tempfile::NamedTempFile::new().unwrap();
    let output = log.reopen().unwrap();
    let name = concat!(
        module_path!(),
        "::movement_and_cleanup_follow_the_same_lock_order"
    )
    .strip_prefix("pumpkin_core::")
    .unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("PUMPKIN_TICKET_LOCK_CHILD", "1")
        .env("RAYON_NUM_THREADS", "2")
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "movement/cleanup timed out or failed: {status:?}\n{output}"
    );
}
