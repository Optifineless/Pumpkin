use super::*;
use crate::net::java::combat_test_support::{Completion, OutgoingPacket};
use pumpkin_util::{
    GameMode,
    math::{vector2::Vector2, vector3::Vector3},
};
use std::{future::Future, io::Write, sync::Mutex};
use tokio::sync::mpsc::UnboundedReceiver;

pub(super) async fn collect<F: Future>(
    operation: F,
    receive: &mut UnboundedReceiver<OutgoingPacket>,
) -> (F::Output, Vec<bytes::Bytes>) {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut packets = Vec::new();
        tokio::pin!(operation);
        let result = loop {
            tokio::select! {
                result = &mut operation => break result,
                Some(packet) = receive.recv() => capture(packet, &mut packets),
            }
        };
        while let Ok(packet) = receive.try_recv() {
            capture(packet, &mut packets);
        }
        (result, packets)
    })
    .await
    .expect("Real player lifecycle operation timed out")
}

pub(super) fn capture(packet: OutgoingPacket, packets: &mut Vec<bytes::Bytes>) {
    if let OutgoingPacket::Data { data, completion } = packet {
        packets.push(data);
        if let Some(Completion::Framed(done) | Completion::Flushed(done)) = completion {
            let _ = done.send(());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_real_join_and_dimension_change_keep_one_shared_state() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        populate_base(&server, &worlds[1]);
        let mut observer = TestPlayer::new(&worlds[0]);
        observer.client().set_player(observer.player.clone());
        observer
            .client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_2);
        observer.player.send_scoreboard();
        observer.take_packets();
        let (player, mut receive) = super::login::join_player(&server, &worlds).await;
        player.set_client_loaded(true);
        player.set_gamemode(GameMode::Creative);
        let gap = super::reentry::mutate_during_transfer(&server, player.clone());
        combat_test_support::publish_empty_chunk(&worlds[2], Vector2::new(0, 0));
        let ((), packets) = collect(
            player.teleport_world(
                worlds[2].clone(),
                Vector3::new(0.5, 64.0, 0.5),
                Some(0.0),
                Some(0.0),
            ),
            &mut receive,
        )
        .await;
        assert!(Arc::ptr_eq(&player.world(), &worlds[2]));
        assert!(
            gap.load(std::sync::atomic::Ordering::Relaxed),
            "Transfer-gap callback did not run"
        );
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
            ),
            1,
            "The moving player missed the score update while absent from both world player lists"
        );
        assert_eq!(
            command(&server, &worlds[2], "scoreboard players get Gap tally"),
            99
        );
        // ClientPacketListener.handleRespawn keeps its scoreboard across dimensions.
        // Re-sending CREATE here duplicates the team already sent during login.
        for id in [
            pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0,
            pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0,
        ] {
            assert_eq!(
                packet_count(&packets, id),
                0,
                "Dimension transfer duplicated persistent scoreboard state"
            );
        }
        command(&server, &worlds[1], "team modify red color red");
        let mut packets = Vec::new();
        while let Ok(packet) = receive.try_recv() {
            capture(packet, &mut packets);
        }
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0
            ),
            1,
            "Changed-dimension player missed or duplicated its next team update"
        );
        // Real login attaches a strong self-reference to its inventory sync handler.
        // End fixture ownership before with_worlds checks that every level was released.
        player
            .screen_handler_sync_handler
            .store_player(observer.player.clone());
    })
    .await;
}

#[derive(Clone)]
pub(super) struct LogBuffer(pub(super) Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_corrupt_load_logs_error_and_preserves_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data/minecraft");
    fs::create_dir_all(&data_dir).unwrap();
    let original = b"a player's damaged scoreboard must never become an empty replacement";
    fs::write(data_dir.join("scoreboard.dat"), original).unwrap();
    let logs = LogBuffer(Arc::default());
    let captured = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::ERROR)
        .with_writer(move || captured.clone())
        .finish();
    let server = tracing::subscriber::with_default(subscriber, || {
        combat_test_support::server(directory.path())
    });
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world]));
    command(
        &server,
        &server.worlds.load()[0],
        "scoreboard objectives add fresh dummy",
    );
    let _ = tokio::time::timeout(Duration::from_secs(30), server.save_all())
        .await
        .expect("Real scoreboard save timed out");
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    let preserved = fs::read_dir(&data_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("scoreboard.dat")
        })
        .any(|entry| fs::read(entry.path()).is_ok_and(|bytes| bytes == original));
    server.shutdown().await;
    assert!(
        preserved,
        "Saving destroyed the original corrupt scoreboard bytes"
    );
    assert!(
        log.to_ascii_lowercase().contains("scoreboard"),
        "Corrupt scoreboard load did not emit an error: {log}"
    );
}
