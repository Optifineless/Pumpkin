//! Management RPC and pending-upgrade shutdown regressions.
use std::{
    collections::HashSet,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    process::Command,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::HeaderMap,
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tokio_util::sync::CancellationToken;

use super::{
    AppState, STOP_RPC_REPLY_FLUSH_TIMEOUT, handlers, ws_handler, ws_handler_with_dispatch,
};
use crate::server::combat_test_support;
struct StalledWriteListener {
    listener: TcpListener,
    stall_writes: CancellationToken,
    release_writes: CancellationToken,
    write_stalled: mpsc::UnboundedSender<()>,
}

struct StalledWriteConnection {
    stream: TcpStream,
    stall_writes: CancellationToken,
    release_writes: CancellationToken,
    release_future: Pin<Box<dyn Future<Output = ()> + Send>>,
    write_stalled: mpsc::UnboundedSender<()>,
    reported_stall: bool,
}

impl axum::serve::Listener for StalledWriteListener {
    type Io = StalledWriteConnection;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, address) = self.listener.accept().await.unwrap();
        (
            StalledWriteConnection {
                stream,
                stall_writes: self.stall_writes.clone(),
                release_writes: self.release_writes.clone(),
                release_future: Box::pin(self.release_writes.clone().cancelled_owned()),
                write_stalled: self.write_stalled.clone(),
                reported_stall: false,
            },
            address,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl AsyncRead for StalledWriteConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for StalledWriteConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let connection = self.get_mut();
        if connection.stall_writes.is_cancelled() && !connection.release_writes.is_cancelled() {
            if !connection.reported_stall {
                let _ = connection.write_stalled.send(());
                connection.reported_stall = true;
            }
            if connection
                .release_future
                .as_mut()
                .poll(context)
                .is_pending()
            {
                return Poll::Pending;
            }
        }
        Pin::new(&mut connection.stream).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(context)
    }
}

type WsClient = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

struct StalledWriteHarness {
    address: SocketAddr,
    stall_writes: CancellationToken,
    release_writes: CancellationToken,
    write_stalled: mpsc::UnboundedReceiver<()>,
    shutdown: oneshot::Sender<()>,
    serve: tokio::task::JoinHandle<()>,
}

fn management_server_with_secret(
    directory: &tempfile::TempDir,
) -> (Arc<crate::server::Server>, String) {
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
    (server, secret)
}

async fn start_stalled_write_server(
    server: Arc<crate::server::Server>,
    secret: String,
) -> StalledWriteHarness {
    let state = AppState {
        server,
        secret,
        allowed_origins: Arc::new(HashSet::new()),
    };
    let app = Router::new().route("/", get(ws_handler)).with_state(state);
    let tcp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp_listener.local_addr().unwrap();
    let (write_stalled_tx, write_stalled) = mpsc::unbounded_channel();
    let stall_writes = CancellationToken::new();
    let release_writes = CancellationToken::new();
    let listener = StalledWriteListener {
        listener: tcp_listener,
        stall_writes: stall_writes.clone(),
        release_writes: release_writes.clone(),
        write_stalled: write_stalled_tx,
    };
    let (shutdown, shutdown_rx) = oneshot::channel();
    let serve = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });
    StalledWriteHarness {
        address,
        stall_writes,
        release_writes,
        write_stalled,
        shutdown,
        serve,
    }
}

fn batch_dispatch_app(
    state: AppState,
    save_started: mpsc::UnboundedSender<()>,
    release_save: watch::Receiver<bool>,
) -> Router {
    Router::new()
        .route(
            "/",
            get(
                move |State(state): State<AppState>, headers: HeaderMap, ws| {
                    let save_started = save_started.clone();
                    let release_save = release_save.clone();
                    async move {
                        ws_handler_with_dispatch(&state, &headers, ws, move |request, server| {
                            let save_started = save_started.clone();
                            let mut release_save = release_save.clone();
                            async move {
                                if request.method.as_deref() == Some("minecraft:server/save") {
                                    let _ = save_started.send(());
                                    while !*release_save.borrow() {
                                        if release_save.changed().await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                handlers::dispatch(request, &server).await
                            }
                        })
                    }
                },
            ),
        )
        .with_state(state)
}

async fn connect_management_client(address: SocketAddr, secret: &str) -> WsClient {
    let mut request = format!("ws://{address}/").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {secret}").parse().unwrap());
    connect_async(request).await.unwrap().0
}

fn valid_stopping_batch_response(response: &serde_json::Value) -> bool {
    let Some(responses) = response.as_array() else {
        return false;
    };
    let response_ids = responses
        .iter()
        .map(|response| response["id"].clone())
        .collect::<Vec<_>>();
    let stop_response = responses.iter().find(|response| response["id"] == 1);
    let save_response = responses.iter().find(|response| response["id"] == 2);
    response_ids == [json!(1), json!(2)]
        && stop_response.is_some_and(|response| {
            response["result"].as_bool() == Some(true) && response.get("error").is_none()
        })
        && save_response.is_some_and(|response| {
            response["error"]["code"].as_i64() == Some(-32000)
                && response["error"]["message"].as_str() == Some("Server is stopping")
        })
}

async fn receive_stopping_notification(client: &mut WsClient) {
    loop {
        let message = client.next().await.unwrap().unwrap();
        let notification: serde_json::Value =
            serde_json::from_str(message.to_text().unwrap()).unwrap();
        if notification["method"] == "minecraft:notification/server/stopping" {
            return;
        }
    }
}

fn run_isolated_child_test(test_name: &str, child_env: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .arg(test_name)
        .arg("--nocapture")
        .env(child_env, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child test failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("test result: ok."),
        "child test exited before its assertions completed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn management_stop_rpc_signals_when_reply_flush_stalls() {
    const CHILD: &str = "PUMPKIN_MANAGEMENT_STOP_RPC_STALLED_REPLY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_isolated_child_test("management_stop_rpc_signals_when_reply_flush_stalls", CHILD);
        return;
    }
    stalled_reply_child().await;
}

async fn stalled_reply_child() {
    tokio::time::timeout(Duration::from_secs(30), stalled_reply_scenario())
        .await
        .expect("stalled stop reply regression exceeded the 30-second crash guard");
}

async fn stalled_reply_scenario() {
    let directory = tempfile::tempdir().unwrap();
    let (server, secret) = management_server_with_secret(&directory);
    let mut harness = start_stalled_write_server(server.clone(), secret.clone()).await;
    let mut client = connect_management_client(harness.address, &secret).await;
    harness.stall_writes.cancel();

    let mut batch = (0..1024)
        .map(|id| json!({"jsonrpc":"2.0", "id":id, "method":"rpc.discover"}))
        .collect::<Vec<_>>();
    batch.push(json!({
        "jsonrpc":"2.0",
        "id":1024,
        "method":"minecraft:server/stop"
    }));
    client
        .send(Message::Text(serde_json::to_string(&batch).unwrap().into()))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(30), harness.write_stalled.recv())
        .await
        .expect("the stop reply never reached the controlled stalled-write boundary")
        .unwrap();
    let stop_signaled_while_reply_was_stalled = crate::STOP_INTERRUPT.is_cancelled();

    server.begin_shutdown();
    let drain_finished_within_reply_deadline = tokio::time::timeout(
        STOP_RPC_REPLY_FLUSH_TIMEOUT + Duration::from_secs(2),
        server.shutdown(),
    )
    .await
    .is_ok();
    harness.release_writes.cancel();
    drop(client);
    if !stop_signaled_while_reply_was_stalled {
        tokio::time::timeout(Duration::from_secs(30), crate::STOP_INTERRUPT.cancelled())
            .await
            .expect("releasing the stalled reply did not let the stop request signal");
    }
    if !drain_finished_within_reply_deadline {
        server.shutdown().await;
    }
    let _ = harness.shutdown.send(());
    harness.serve.await.unwrap();

    assert!(
        stop_signaled_while_reply_was_stalled,
        "the stop signal waited for the acknowledgement flush"
    );
    assert!(
        drain_finished_within_reply_deadline,
        "the stalled acknowledgement held the session past its flush deadline"
    );
}

#[tokio::test]
async fn management_stop_rpc_batch_preempts_a_blocked_later_dispatch() {
    const CHILD: &str = "PUMPKIN_MANAGEMENT_STOP_RPC_BATCH_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_isolated_child_test(
            "management_stop_rpc_batch_preempts_a_blocked_later_dispatch",
            CHILD,
        );
        return;
    }
    batch_stop_child().await;
}

async fn batch_stop_child() {
    tokio::time::timeout(Duration::from_secs(30), batch_stop_scenario())
        .await
        .expect("stop batch regression exceeded the 30-second crash guard");
}

async fn batch_stop_scenario() {
    let directory = tempfile::tempdir().unwrap();
    let (server, secret) = management_server_with_secret(&directory);
    let (save_started_tx, mut save_started_rx) = mpsc::unbounded_channel();
    let (release_save_tx, release_save_rx) = watch::channel(false);
    let state = AppState {
        server: server.clone(),
        secret: secret.clone(),
        allowed_origins: Arc::new(HashSet::new()),
    };
    let app = batch_dispatch_app(state, save_started_tx, release_save_rx);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (http_shutdown_tx, http_shutdown_rx) = oneshot::channel();
    let http_server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = http_shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    let mut client = connect_management_client(address, &secret).await;
    client
        .send(Message::Text(
            r#"[{"jsonrpc":"2.0","id":1,"method":"minecraft:server/stop"},{"jsonrpc":"2.0","id":2,"method":"minecraft:server/save","params":{"flush":true}}]"#.into(),
        ))
        .await
        .unwrap();
    let stop_preempted_save = tokio::select! {
        biased;
        () = crate::STOP_INTERRUPT.cancelled() => true,
        save_started = save_started_rx.recv() => {
            if save_started.is_some() {
                let _ = release_save_tx.send(true);
            }
            false
        }
    };
    if !stop_preempted_save {
        tokio::time::timeout(Duration::from_secs(30), crate::STOP_INTERRUPT.cancelled())
            .await
            .expect("stop signal did not follow the blocked batch save");
    }

    let response = client.next().await.unwrap().unwrap();
    let response: serde_json::Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
    let response_shape_is_valid = valid_stopping_batch_response(&response);
    server.begin_shutdown();
    receive_stopping_notification(&mut client).await;
    client.close(None).await.unwrap();
    server.shutdown().await;
    let _ = http_shutdown_tx.send(());
    http_server.await.unwrap();

    assert!(
        stop_preempted_save,
        "accepted stop was held behind the later server/save dispatch"
    );
    assert!(
        response_shape_is_valid,
        "unexpected JSON-RPC batch response: {response}"
    );
    assert!(save_started_rx.try_recv().is_err());
}

#[derive(Clone)]
struct UpgradeResponseGate {
    response_ready: mpsc::UnboundedSender<()>,
    release: watch::Receiver<bool>,
}

async fn pause_upgrade_response(
    State(mut gate): State<UpgradeResponseGate>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    let _ = gate.response_ready.send(());
    while !*gate.release.borrow() {
        if gate.release.changed().await.is_err() {
            break;
        }
    }
    response
}

#[tokio::test]
async fn management_upgrade_is_tracked_before_response_is_returned() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let directory = tempfile::tempdir().unwrap();
        let (server, secret) = management_server_with_secret(&directory);
        assert!(server.tasks.is_empty());
        let (response_ready_tx, mut response_ready_rx) = mpsc::unbounded_channel();
        let (release_response_tx, release_response_rx) = watch::channel(false);
        let state = AppState {
            server: server.clone(),
            secret: secret.clone(),
            allowed_origins: Arc::new(HashSet::new()),
        };
        let app = Router::new()
            .route("/", get(ws_handler))
            .layer(middleware::from_fn_with_state(
                UpgradeResponseGate {
                    response_ready: response_ready_tx,
                    release: release_response_rx,
                },
                pause_upgrade_response,
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

        let connect_secret = secret.clone();
        let connect =
            tokio::spawn(async move { connect_management_client(address, &connect_secret).await });
        response_ready_rx.recv().await.unwrap();
        let upgrade_reserved_before_response = !server.tasks.is_empty();

        server.begin_shutdown();
        server.tasks.close();
        let tracker = server.tasks.clone();
        let drain = tokio::spawn(async move {
            tracker.wait().await;
        });
        release_response_tx.send(true).unwrap();
        let mut client = connect.await.unwrap();
        let notification = client.next().await.unwrap().unwrap();
        let notification: serde_json::Value =
            serde_json::from_str(notification.to_text().unwrap()).unwrap();
        assert_eq!(
            notification["method"],
            "minecraft:notification/server/stopping"
        );
        client.close(None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), drain)
            .await
            .expect("management tracker drain exceeded the 30-second crash guard")
            .unwrap();
        // A closed TaskTracker still accepts spawns; wait again to catch any late callback.
        server.tasks.wait().await;
        let _ = shutdown_tx.send(());
        serve.await.unwrap();
        server.shutdown().await;

        assert!(
            upgrade_reserved_before_response,
            "pending WebSocket upgrade was not registered with the shutdown tracker"
        );
    })
    .await
    .expect("pending-upgrade shutdown regression exceeded the 30-second crash guard");
}
