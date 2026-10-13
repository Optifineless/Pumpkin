//! Shutdown regression through the real management listener and `Server::shutdown`.
use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

use super::ManagementServer;
use crate::server::combat_test_support;

#[tokio::test]
async fn management_shutdown_drains_listener_sessions_and_heartbeat() {
    // A deadline is only a crash guard; RPC acknowledgement establishes subscription.
    tokio::time::timeout(Duration::from_secs(30), async {
        for heartbeat_interval in [0, 3600] {
            let directory = tempfile::tempdir().unwrap();
            let mut server = combat_test_support::server(directory.path());
            let config = &mut Arc::get_mut(&mut server)
                .unwrap()
                .advanced_config
                .networking
                .management;
            config.enabled = true;
            config.address = "127.0.0.1:0".parse().unwrap();
            config.secret = "a".repeat(40);
            config.status_heartbeat_interval = heartbeat_interval;
            let config = config.clone();
            server
                .management_hub
                .settings
                .status_heartbeat_interval
                .store(
                    u64::from(heartbeat_interval),
                    std::sync::atomic::Ordering::Relaxed,
                );
            let listener = ManagementServer::bind(&config).await.unwrap();
            let address = listener.local_addr().unwrap();
            let management_server = server.clone();
            let management = server.spawn_task(async move {
                ManagementServer::run(&config, management_server, listener).await;
            });
            let mut request = format!("ws://{address}/").into_client_request().unwrap();
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {}", "a".repeat(40)).parse().unwrap(),
            );
            let (mut client, _) = connect_async(request).await.unwrap();
            client
                .send(Message::Text(
                    r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:server/status"}"#.into(),
                ))
                .await
                .unwrap();
            let reply = client.next().await.unwrap().unwrap();
            let reply: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
            assert_eq!(reply["id"], 1);
            assert!(reply.get("result").is_some(), "{reply}");

            server.shutdown().await;
            // The peer stays open throughout shutdown. Closing it must not be necessary
            // to make axum's graceful shutdown or the server's tracker finish.
            management.await.unwrap();
            let notification = client.next().await.unwrap().unwrap();
            let notification: serde_json::Value =
                serde_json::from_str(notification.to_text().unwrap()).unwrap();
            assert_eq!(
                notification["method"],
                "minecraft:notification/server/stopping"
            );
            assert!(directory.path().join("level.dat").exists());
            assert!(server.tasks.is_empty());
            assert_eq!(
                Arc::strong_count(&server),
                1,
                "management retained the server after shutdown"
            );
            // The listener has released its port, and the open WebSocket has closed.
            let _rebound = TcpListener::bind(address).await.unwrap();
            assert!(matches!(
                client.next().await,
                None | Some(Ok(Message::Close(_)))
            ));
        }
    })
    .await
    .expect("management shutdown exceeded the 30-second harness budget");
}
