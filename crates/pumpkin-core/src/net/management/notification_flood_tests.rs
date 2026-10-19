//! Incoming RPCs and pings must progress while the real notification receiver stays ready.
use std::{
    collections::HashSet,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use axum::{Router, routing::get};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tokio_util::sync::CancellationToken;

use super::{AppState, hub::ManagementHub, ws_handler};
use crate::server::{Server, combat_test_support};

const PROGRESS_DEADLINE: Duration = Duration::from_secs(5);
type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) struct NotificationFlood {
    pub(super) hub: Arc<ManagementHub>,
    pub(super) enabled: AtomicBool,
    pub(super) replenished: AtomicUsize,
}

impl NotificationFlood {
    pub(super) fn replenish(&self) {
        if self.enabled.load(Ordering::Relaxed) {
            // Every notification flush replaces its queue entry before the next select.
            // Unlike a producer task, this cannot lose a scheduling race with the receiver.
            self.hub.broadcast_server_activity();
            self.replenished.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct FloodListener {
    listener: TcpListener,
    flood: Arc<NotificationFlood>,
}

struct FloodConnection {
    stream: TcpStream,
    flood: Arc<NotificationFlood>,
}

impl axum::serve::Listener for FloodListener {
    type Io = FloodConnection;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, address) = self.listener.accept().await.unwrap();
        (
            FloodConnection {
                stream,
                flood: self.flood.clone(),
            },
            address,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl AsyncRead for FloodConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for FloodConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let connection = self.get_mut();
        let result = Pin::new(&mut connection.stream).poll_write(context, buffer);
        if matches!(result, Poll::Ready(Ok(written)) if written > 0) {
            connection.flood.replenish();
        }
        result
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(context)
    }
}

struct FloodHarness {
    address: SocketAddr,
    flood: Arc<NotificationFlood>,
    shutdown: CancellationToken,
    serve: JoinHandle<()>,
}

async fn start_flood_listener(server: Arc<Server>, secret: String) -> FloodHarness {
    let flood = Arc::new(NotificationFlood {
        hub: server.management_hub.clone(),
        enabled: AtomicBool::new(false),
        replenished: AtomicUsize::new(0),
    });
    let state = AppState {
        server,
        secret,
        allowed_origins: Arc::new(HashSet::new()),
    };
    let app = Router::new().route("/", get(ws_handler)).with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let listener = FloodListener {
        listener,
        flood: flood.clone(),
    };
    let shutdown = CancellationToken::new();
    let shutdown_signal = shutdown.clone();
    let serve = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown_signal.cancelled_owned())
            .await
            .unwrap();
    });
    FloodHarness {
        address,
        flood,
        shutdown,
        serve,
    }
}

async fn send_rpc(client: &mut WsClient, id: u32, method: &str) {
    client
        .send(Message::Text(
            json!({"jsonrpc":"2.0", "id":id, "method":method})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

async fn receive_rpc_response(client: &mut WsClient, id: u32) -> Value {
    loop {
        let message = client.next().await.unwrap().unwrap();
        if let Message::Text(text) = message {
            let response: Value = serde_json::from_str(text.as_str()).unwrap();
            if response["id"] == id {
                return response;
            }
        }
    }
}

async fn receive_status_and_pong(client: &mut WsClient, ping: &[u8]) {
    let (mut status_received, mut pong_received) = (false, false);
    while !status_received || !pong_received {
        match client.next().await.unwrap().unwrap() {
            Message::Text(text) => {
                let response: Value = serde_json::from_str(text.as_str()).unwrap();
                if response["id"] == 1 {
                    assert!(response["result"].is_object());
                    status_received = true;
                }
            }
            Message::Pong(payload) if payload.as_ref() == ping => pong_received = true,
            _ => {}
        }
    }
}

async fn receive_stopping(client: &mut WsClient) {
    loop {
        let message = client.next().await.unwrap().unwrap();
        if let Message::Text(text) = message {
            let notification: Value = serde_json::from_str(text.as_str()).unwrap();
            if notification["method"] == "minecraft:notification/server/stopping" {
                return;
            }
        }
    }
}

#[tokio::test]
async fn notification_flood_keeps_rpc_ping_and_stop_responsive() {
    const CHILD: &str = "PUMPKIN_MANAGEMENT_NOTIFICATION_FLOOD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        assert_eq!(
            super::shutdown_child::run_isolated_child_test(
                "net::management::notification_flood_tests::notification_flood_keeps_rpc_ping_and_stop_responsive",
                CHILD,
            )
            .await,
            Ok(())
        );
        return;
    }
    tokio::time::timeout(Duration::from_secs(30), notification_flood_scenario())
        .await
        .expect("notification flood regression exceeded its 30-second crash guard");
}

async fn notification_flood_scenario() {
    let directory = tempfile::tempdir().unwrap();
    let mut server = combat_test_support::server(directory.path());
    let secret = "a".repeat(40);
    Arc::get_mut(&mut server)
        .unwrap()
        .advanced_config
        .networking
        .management
        .secret = secret.clone();
    let harness = start_flood_listener(server.clone(), secret.clone()).await;
    let mut request = format!("ws://{}/", harness.address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {secret}").parse().unwrap());
    let mut client = connect_async(request).await.unwrap().0;

    // The no-flood control also acknowledges that the real session has subscribed.
    send_rpc(&mut client, 0, "minecraft:server/status").await;
    let response = tokio::time::timeout(PROGRESS_DEADLINE, receive_rpc_response(&mut client, 0))
        .await
        .expect("the no-flood status control did not respond");
    assert!(response["result"].is_object());
    assert_eq!(harness.flood.replenished.load(Ordering::Relaxed), 0);

    harness.flood.enabled.store(true, Ordering::Relaxed);
    server.management_hub.broadcast_server_activity();
    let first_notification = client.next().await.unwrap().unwrap();
    let first_notification: Value =
        serde_json::from_str(first_notification.to_text().unwrap()).unwrap();
    assert_eq!(
        first_notification["method"],
        "minecraft:notification/server/activity"
    );
    send_rpc(&mut client, 1, "minecraft:server/status").await;
    let ping = b"notification-flood";
    client
        .send(Message::Ping(ping.to_vec().into()))
        .await
        .unwrap();
    let ordinary_progress = tokio::time::timeout(
        PROGRESS_DEADLINE,
        receive_status_and_pong(&mut client, ping),
    )
    .await;
    assert!(harness.flood.replenished.load(Ordering::Relaxed) > 0);
    assert!(harness.flood.enabled.load(Ordering::Relaxed));
    if ordinary_progress.is_err() {
        harness.flood.enabled.store(false, Ordering::Relaxed);
    }
    ordinary_progress.expect("notification flood starved an incoming status RPC or ping");

    send_rpc(&mut client, 2, "minecraft:server/stop").await;
    let stop_progress = tokio::time::timeout(PROGRESS_DEADLINE, async {
        loop {
            tokio::select! {
                () = crate::STOP_INTERRUPT.cancelled() => break,
                message = client.next() => { message.unwrap().unwrap(); }
            }
        }
    })
    .await;
    harness.flood.enabled.store(false, Ordering::Relaxed);
    stop_progress.expect("notification flood starved the incoming stop RPC");

    server.begin_shutdown();
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(server.shutdown(), receive_stopping(&mut client));
    })
    .await
    .expect("the stopping flush or final save stalled after the notification flood");
    assert!(directory.path().join("level.dat").exists());
    assert!(server.tasks.is_empty());
    harness.shutdown.cancel();
    harness.serve.await.unwrap();
}
