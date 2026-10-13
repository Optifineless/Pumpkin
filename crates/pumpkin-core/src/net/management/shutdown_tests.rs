//! Shutdown regressions through the real management listener and RPC entry points.
use std::{collections::HashSet, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::State,
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use futures::{SinkExt, StreamExt};
use tokio::sync::oneshot;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

use super::{AppState, ManagementServer, SHUTDOWN_GRACE_PERIOD, ws_handler};
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

#[tokio::test]
async fn management_shutdown_bounds_partial_http_request_drain() {
    tokio::time::timeout(Duration::from_secs(30), async {
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
        config.status_heartbeat_interval = 0;
        let config = config.clone();
        let listener = ManagementServer::bind(&config).await.unwrap();
        let address = listener.local_addr().unwrap();
        let management_server = server.clone();
        let management = server.spawn_task(async move {
            ManagementServer::run(&config, management_server, listener).await;
        });

        let level_dat = directory.path().join("level.dat");
        assert!(!level_dat.exists());
        let mut peer = TcpStream::connect(address).await.unwrap();
        peer.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n")
            .await
            .unwrap();

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

        server.begin_shutdown();
        let shutdown = tokio::time::timeout(
            SHUTDOWN_GRACE_PERIOD + Duration::from_secs(2),
            server.shutdown(),
        )
        .await;
        if shutdown.is_err() {
            management.abort();
            let _ = management.await;
            panic!("management HTTP drain exceeded its grace deadline");
        }
        management.await.unwrap();

        assert!(level_dat.exists(), "shutdown did not save level.dat");
        assert!(server.tasks.is_empty());
        let notification = client.next().await.unwrap().unwrap();
        let notification: serde_json::Value =
            serde_json::from_str(notification.to_text().unwrap()).unwrap();
        assert_eq!(
            notification["method"],
            "minecraft:notification/server/stopping"
        );
        drop(peer);
        let _rebound = TcpListener::bind(address).await.unwrap();
    })
    .await
    .expect("partial-header shutdown regression exceeded the 30-second crash guard");
}

#[tokio::test]
async fn concurrent_shutdown_claim_enqueues_before_cancellation() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let mut first_session = server.management_hub.subscribe();
        let mut second_session = server.management_hub.subscribe();
        let shutdown = server.management_shutdown.clone();
        let (monitor_ready_tx, monitor_ready_rx) = oneshot::channel();
        let observed_shutdown = tokio::spawn(async move {
            let _ = monitor_ready_tx.send(());
            shutdown.cancelled().await;
            (
                first_session.try_recv(),
                first_session.try_recv(),
                second_session.try_recv(),
                second_session.try_recv(),
            )
        });
        monitor_ready_rx.await.unwrap();

        let (after_claim_tx, after_claim_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = oneshot::channel();
        let first_server = server.clone();
        let first_shutdown = tokio::task::spawn_blocking(move || {
            first_server.begin_shutdown_with_adapter(move || {
                let _ = after_claim_tx.send(());
                let _ = resume_rx.blocking_recv();
            });
        });
        after_claim_rx.await.unwrap();
        assert!(server.management_hub.is_server_stopping());

        let second_server = server.clone();
        tokio::task::spawn_blocking(move || second_server.begin_shutdown())
            .await
            .unwrap();
        let cancellation_preceded_enqueue = server.management_shutdown.is_cancelled();
        let _ = resume_tx.send(());
        first_shutdown.await.unwrap();

        let (first, first_extra, second, second_extra) = observed_shutdown.await.unwrap();
        assert!(
            !cancellation_preceded_enqueue,
            "a losing shutdown caller cancelled management before the stopping notification was enqueued"
        );
        for event in [first, second] {
            let notification: serde_json::Value = serde_json::from_str(&event.unwrap()).unwrap();
            assert_eq!(
                notification["method"],
                "minecraft:notification/server/stopping"
            );
        }
        assert!(first_extra.is_err(), "first session received a duplicate stop notification");
        assert!(second_extra.is_err(), "second session received a duplicate stop notification");
    })
    .await
    .expect("concurrent shutdown claim regression exceeded its 30-second crash guard");
}

#[tokio::test]
async fn management_stop_rpc_requests_shutdown_owner_after_reply() {
    const CHILD: &str = "PUMPKIN_MANAGEMENT_STOP_RPC_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        assert_eq!(
            super::shutdown_child::run_isolated_child_test(
                "net::management::shutdown_tests::management_stop_rpc_requests_shutdown_owner_after_reply",
                CHILD,
            )
            .await,
            Ok(()),
            "parent process could not verify the stop RPC child"
        );
        return;
    }

    management_stop_rpc_child().await;
}

async fn management_stop_rpc_child() {
    tokio::time::timeout(Duration::from_secs(30), async {
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
        config.status_heartbeat_interval = 0;
        let config = config.clone();
        let listener = ManagementServer::bind(&config).await.unwrap();
        let address = listener.local_addr().unwrap();
        let management_server = server.clone();
        let management = server.spawn_task(async move {
            ManagementServer::run(&config, management_server, listener).await;
        });

        let (release_idle_tx, release_idle_rx) = oneshot::channel();
        let (idle_started_tx, idle_started_rx) = oneshot::channel();
        let (idle_done_tx, mut idle_done_rx) = oneshot::channel();
        server.spawn_task(async move {
            let _ = idle_started_tx.send(());
            let _ = release_idle_rx.await;
            let _ = idle_done_tx.send(());
        });
        idle_started_rx.await.unwrap();

        let mut request = format!("ws://{address}/").into_client_request().unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", "a".repeat(40)).parse().unwrap(),
        );
        let (mut client, _) = connect_async(request).await.unwrap();
        client
            .send(Message::Text(
                r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:server/stop"}"#.into(),
            ))
            .await
            .unwrap();
        let reply = client.next().await.unwrap().unwrap();
        let reply: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
        assert_eq!(reply["id"], 1);
        assert!(reply["result"].as_bool().unwrap());

        crate::STOP_INTERRUPT.cancelled().await;
        assert!(crate::SHOULD_STOP.load(std::sync::atomic::Ordering::Relaxed));
        assert!(
            !server.management_shutdown.is_cancelled(),
            "the RPC must leave management draining to the outer lifecycle"
        );
        assert!(
            idle_done_rx.try_recv().is_err(),
            "tracked idle task ended early"
        );

        client
            .send(Message::Text(
                r#"{"jsonrpc":"2.0","id":2,"method":"minecraft:server/status"}"#.into(),
            ))
            .await
            .unwrap();
        let reply = client.next().await.unwrap().unwrap();
        let reply: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
        assert_eq!(reply["id"], 2);
        assert!(reply.get("result").is_some(), "{reply}");
        assert!(
            idle_done_rx.try_recv().is_err(),
            "tracked idle task ended early"
        );

        release_idle_tx.send(()).unwrap();
        server.begin_shutdown();
        let notification = client.next().await.unwrap().unwrap();
        let notification: serde_json::Value =
            serde_json::from_str(notification.to_text().unwrap()).unwrap();
        assert_eq!(
            notification["method"],
            "minecraft:notification/server/stopping"
        );
        server.shutdown().await;
        management.await.unwrap();
        assert!(idle_done_rx.try_recv().is_ok());
        assert!(server.tasks.is_empty());
    })
    .await
    .expect("management stop RPC exceeded the 30-second harness budget");
}

#[tokio::test]
async fn management_upgrade_subscribes_before_response_is_returned() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let directory = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(directory.path());
        let secret = {
            let config = &mut Arc::get_mut(&mut server)
                .unwrap()
                .advanced_config
                .networking
                .management;
            config.secret = "a".repeat(40);
            config.secret.clone()
        };

        let state = AppState {
            server: server.clone(),
            secret: secret.clone(),
            allowed_origins: Arc::new(HashSet::new()),
        };
        let app = Router::new()
            .route("/", get(ws_handler))
            .layer(middleware::from_fn_with_state(
                server.clone(),
                broadcast_stopping_before_response,
            ))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let serve = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
        });

        let mut request = format!("ws://{address}/").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {secret}").parse().unwrap());
        let (mut client, _) = connect_async(request).await.unwrap();
        let notification = client.next().await.unwrap().unwrap();
        let notification: serde_json::Value =
            serde_json::from_str(notification.to_text().unwrap()).unwrap();
        assert_eq!(
            notification["method"],
            "minecraft:notification/server/stopping"
        );

        client.close(None).await.unwrap();
        let _ = shutdown_tx.send(());
        server.shutdown().await;
        serve.await.unwrap();
    })
    .await
    .expect("management upgrade regression exceeded the 30-second harness budget");
}

async fn broadcast_stopping_before_response(
    State(server): State<Arc<crate::server::Server>>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    server.management_hub.broadcast_server_stopping();
    response
}
