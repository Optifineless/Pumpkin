//! Exercise independently stalled writes and shutdown with a producer still active.
use std::{sync::atomic::Ordering, time::Duration};

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::{
    StalledWriteHarness, WsClient, connect_management_client, management_server_with_secret,
    start_stalled_write_server,
};
use crate::{net::management::hub::NOTIFICATION_CAPACITY, server::Server};

const DEADLINE: Duration = Duration::from_secs(3);

async fn stalled_connection(
    server: std::sync::Arc<Server>,
    secret: String,
) -> (StalledWriteHarness, WsClient) {
    let mut harness = start_stalled_write_server(server.clone(), secret.clone()).await;
    let mut client = connect_management_client(harness.address, &secret).await;
    send_rpc(&mut client, 0, "minecraft:server/status").await;
    let response = tokio::time::timeout(DEADLINE, client.next())
        .await
        .expect("status control timed out")
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
    assert_eq!(response["id"], 0);
    assert!(response["result"].is_object());
    harness.stall_writes.cancel();
    server.management_hub.broadcast_server_activity();
    tokio::time::timeout(DEADLINE, harness.write_stalled.recv())
        .await
        .expect("notification never reached the stalled-write boundary")
        .unwrap();
    (harness, client)
}

async fn send_rpc(client: &mut WsClient, id: u32, method: &str) {
    tokio::time::timeout(
        DEADLINE,
        client.send(Message::Text(
            json!({"jsonrpc":"2.0", "id":id, "method":method})
                .to_string()
                .into(),
        )),
    )
    .await
    .expect("client RPC send stalled")
    .unwrap();
}

#[tokio::test]
async fn stalled_notification_writer_keeps_incoming_stop_live() {
    const CHILD: &str = "PUMPKIN_MANAGEMENT_STALLED_NOTIFICATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        assert_eq!(super::super::shutdown_child::run_isolated_child_test(
            "net::management::shutdown_rpc_tests::backpressure_tests::stalled_notification_writer_keeps_incoming_stop_live",
            CHILD,
        ).await, Ok(()));
        return;
    }
    tokio::time::timeout(Duration::from_secs(20), stalled_reader_scenario())
        .await
        .expect("stalled-reader scenario exceeded its crash guard");
}

async fn stalled_reader_scenario() {
    let directory = tempfile::tempdir().unwrap();
    let (server, secret) = management_server_with_secret(&directory);
    let (harness, mut client) = stalled_connection(server.clone(), secret).await;
    // The peer never reads again, but continues sending through the real RPC entry.
    for _ in 0..NOTIFICATION_CAPACITY * 2 {
        server.management_hub.broadcast_server_activity();
    }
    send_rpc(&mut client, 1, "minecraft:server/status").await;
    tokio::time::timeout(
        DEADLINE,
        client.send(Message::Ping(b"still-sending".to_vec().into())),
    )
    .await
    .expect("client ping send stalled")
    .unwrap();
    send_rpc(&mut client, 2, "minecraft:server/stop").await;
    tokio::time::timeout(DEADLINE, crate::STOP_INTERRUPT.cancelled())
        .await
        .expect("stalled notification write blocked incoming stop dispatch");
    server.begin_shutdown();
    tokio::time::timeout(DEADLINE, server.shutdown())
        .await
        .expect("stalled writer held shutdown beyond the stop acknowledgement deadline");
    assert!(server.tasks.is_empty());
    assert!(directory.path().join("level.dat").exists());
    harness.release_writes.cancel();
    drop(client);
    let _ = harness.shutdown.send(());
    tokio::time::timeout(DEADLINE, harness.serve)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn shutdown_notification_drain_stays_bounded_with_active_producer() {
    const CHILD: &str = "PUMPKIN_MANAGEMENT_ACTIVE_PRODUCER_SHUTDOWN_CHILD";
    if std::env::var_os(CHILD).is_none() {
        assert_eq!(super::super::shutdown_child::run_isolated_child_test(
            "net::management::shutdown_rpc_tests::backpressure_tests::shutdown_notification_drain_stays_bounded_with_active_producer",
            CHILD,
        ).await, Ok(()));
        return;
    }
    tokio::time::timeout(Duration::from_secs(20), active_producer_scenario())
        .await
        .expect("active-producer scenario exceeded its crash guard");
}

async fn active_producer_scenario() {
    let directory = tempfile::tempdir().unwrap();
    let (server, secret) = management_server_with_secret(&directory);
    let (harness, mut client) = stalled_connection(server.clone(), secret).await;
    for _ in 0..NOTIFICATION_CAPACITY * 8 {
        server.management_hub.broadcast_server_activity();
    }
    harness.flood.enabled.store(true, Ordering::Relaxed);
    server.begin_shutdown();
    harness.release_writes.cancel();
    let received = tokio::time::timeout(DEADLINE, async {
        let read_notifications = async {
            let (mut notifications, mut stopping) = (0, 0);
            while let Some(message) = client.next().await {
                let message = message.unwrap();
                if message.is_close() {
                    break;
                }
                if let Message::Text(text) = message {
                    let notification: Value = serde_json::from_str(text.as_str()).unwrap();
                    notifications += 1;
                    stopping += usize::from(
                        notification["method"] == "minecraft:notification/server/stopping",
                    );
                }
            }
            (notifications, stopping)
        };
        let ((), received) = tokio::join!(server.shutdown(), read_notifications);
        received
    })
    .await
    .expect("active notification producer delayed session drainage and final saves");
    assert!(harness.flood.enabled.load(Ordering::Relaxed));
    assert!(
        harness.flood.replenished.load(Ordering::Relaxed) > 0,
        "producer did not remain active after shutdown began"
    );
    assert!(
        received.0 <= NOTIFICATION_CAPACITY + 2,
        "shutdown drained {} notifications from a stale Receiver::len snapshot",
        received.0
    );
    assert_eq!(received.1, 1);
    assert!(server.tasks.is_empty());
    assert!(directory.path().join("level.dat").exists());
    harness.flood.enabled.store(false, Ordering::Relaxed);
    let _ = harness.shutdown.send(());
    tokio::time::timeout(DEADLINE, harness.serve)
        .await
        .unwrap()
        .unwrap();
}
