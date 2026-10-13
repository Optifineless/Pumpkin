pub mod auth;
pub mod dto;
pub mod handlers;
pub mod hub;
pub mod rpc;

#[cfg(test)]
mod shutdown_child;
#[cfg(test)]
mod shutdown_rpc_tests;
#[cfg(test)]
mod shutdown_tests;

use std::{
    collections::HashSet,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, atomic::Ordering},
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    serve::Listener,
};
use futures::{SinkExt, StreamExt};
use pumpkin_config::networking::management::ManagementServerConfig;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::server::Server;

use self::{
    auth::SecurityCheckResult,
    dto::{PlayerDto, ServerState, VersionDto},
    rpc::{RpcError, RpcRequest, RpcResponse, is_valid_request_id},
};

pub struct ManagementServer;

// Bound management HTTP and WebSocket drainage when a peer stops reading or an RPC stalls.
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(5);
const STOP_RPC_REPLY_FLUSH_TIMEOUT: Duration = Duration::from_millis(250);
const SERVER_STOPPING_ERROR_CODE: i32 = -32000;

struct ForceCloseListener {
    listener: TcpListener,
    force_close: CancellationToken,
}

// Axum spawns each connection, so the deadline wakes tasks through their I/O wrapper.
struct ForceCloseConnection {
    stream: TcpStream,
    force_close: CancellationToken,
    force_close_future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl Listener for ForceCloseListener {
    type Io = ForceCloseConnection;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, address) = match self.listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    warn!("Management listener accept failed: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let force_close = self.force_close.clone();
            let force_close_future = Box::pin(force_close.clone().cancelled_owned());
            return (
                ForceCloseConnection {
                    stream,
                    force_close,
                    force_close_future,
                },
                address,
            );
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl ForceCloseConnection {
    fn force_close_ready(&mut self, context: &mut Context<'_>) -> bool {
        self.force_close.is_cancelled() || self.force_close_future.as_mut().poll(context).is_ready()
    }
}

fn force_close_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "management connection closed at shutdown deadline",
    )
}

impl AsyncRead for ForceCloseConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let connection = self.get_mut();
        if connection.force_close_ready(context) {
            return Poll::Ready(Err(force_close_error()));
        }
        Pin::new(&mut connection.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for ForceCloseConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let connection = self.get_mut();
        if connection.force_close_ready(context) {
            return Poll::Ready(Err(force_close_error()));
        }
        Pin::new(&mut connection.stream).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let connection = self.get_mut();
        if connection.force_close_ready(context) {
            return Poll::Ready(Err(force_close_error()));
        }
        Pin::new(&mut connection.stream).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let connection = self.get_mut();
        if connection.force_close_ready(context) {
            return Poll::Ready(Err(force_close_error()));
        }
        Pin::new(&mut connection.stream).poll_shutdown(context)
    }
}

#[derive(Clone)]
struct AppState {
    server: Arc<Server>,
    secret: String,
    allowed_origins: Arc<HashSet<String>>,
}

impl ManagementServer {
    /// Binds the enabled management listener before its tracked serve task starts.
    pub async fn bind(config: &ManagementServerConfig) -> Option<TcpListener> {
        if !config.enabled {
            return None;
        }
        let address = config.address;
        match TcpListener::bind(address).await {
            Ok(listener) => {
                info!(
                    "Management server is listening on {}",
                    listener.local_addr().unwrap_or(address)
                );
                Some(listener)
            }
            Err(e) => {
                error!("Failed to bind management server on {address}: {e}");
                None
            }
        }
    }

    /// Serves a bound management listener until the server requests shutdown.
    pub async fn run(config: &ManagementServerConfig, server: Arc<Server>, listener: TcpListener) {
        if !config.enabled || server.management_shutdown.is_cancelled() {
            return;
        }
        let allowed_origins: HashSet<String> = config
            .allowed_origins
            .iter()
            .flat_map(|s| s.split(',').map(|p| p.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();

        let state = AppState {
            server: server.clone(),
            secret: config.secret.clone(),
            allowed_origins: Arc::new(allowed_origins),
        };

        // ManagementServer.stop also shuts down the event loop owning scheduleHeartbeat.
        let heartbeat_shutdown = server.management_shutdown.child_token();
        let heartbeat = tokio::spawn(run_heartbeat_loop(
            server.clone(),
            heartbeat_shutdown.clone(),
        ));

        let app = Router::new().route("/", get(ws_handler)).with_state(state);

        let force_close = CancellationToken::new();
        let listener = ForceCloseListener {
            listener,
            force_close: force_close.clone(),
        };
        let management_shutdown = server.management_shutdown.clone();
        let serve = async move {
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(management_shutdown.clone().cancelled_owned())
                .await
        };
        tokio::pin!(serve);
        tokio::select! {
            biased;
            result = &mut serve => {
                if let Err(error) = result {
                    error!("Management server error: {error}");
                }
            }
            () = async {
                server.management_shutdown.cancelled().await;
                tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;
            } => {
                // ManagementServer.stop closes its listener and event loop outright.
                warn!("Management HTTP shutdown grace period expired; closing remaining connections");
                force_close.cancel();
                if let Err(error) = serve.await {
                    error!("Management server error: {error}");
                }
            }
        }
        heartbeat_shutdown.cancel();
        if let Err(e) = heartbeat.await {
            error!("Management heartbeat error: {e}");
        }
    }
}

async fn run_heartbeat_loop(server: Arc<Server>, shutdown: CancellationToken) {
    loop {
        let interval = server
            .management_hub
            .settings
            .status_heartbeat_interval
            .load(Ordering::Relaxed);
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(Duration::from_secs(interval.max(1))) => {}
        }
        if interval == 0 {
            continue;
        }

        let status = ServerState {
            started: true,
            players: server
                .get_all_players()
                .into_iter()
                .map(|p| PlayerDto::new(Some(p.gameprofile.id), Some(p.gameprofile.name.clone())))
                .collect(),
            version: VersionDto {
                name: CURRENT_MC_VERSION.to_string(),
                protocol: CURRENT_MC_VERSION.protocol_version(),
            },
        };
        server.management_hub.broadcast_status_heartbeat(&status);
    }
}

async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    ws_handler_with_dispatch(&state, &headers, ws, dispatch_request)
}

fn ws_handler_with_dispatch<D, F>(
    state: &AppState,
    headers: &HeaderMap,
    ws: WebSocketUpgrade,
    dispatch: D,
) -> Response
where
    D: FnMut(RpcRequest, Arc<Server>) -> F + Send + 'static,
    F: Future<Output = (Option<RpcResponse>, bool)> + Send + 'static,
{
    if state.server.management_shutdown.is_cancelled()
        || state.server.management_hub.is_server_stopping()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match auth::check_auth(headers, &state.secret, &state.allowed_origins) {
        SecurityCheckResult::Denied(reason) => {
            let body = format!(r#"{{"error":"Unauthorized","message":"{reason}"}}"#);
            Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(axum::http::header::CONNECTION, "close")
                .body(Body::from(body))
                .unwrap_or_else(|_| StatusCode::UNAUTHORIZED.into_response())
        }
        SecurityCheckResult::Allowed { token_in_sec_ws } => {
            let notif_rx = state.server.management_hub.subscribe();
            if state.server.management_shutdown.is_cancelled()
                || state.server.management_hub.is_server_stopping()
            {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            let server = state.server.clone();
            // ManagementServer.stop closes its listener and event loop; reserve this upgrade first.
            let pending_upgrade = server.tasks.token();
            if server.management_shutdown.is_cancelled()
                || server.management_hub.is_server_stopping()
            {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            // Capture the receiver before the upgrade response can race the stopping broadcast.
            let mut response = ws.on_upgrade(move |socket| {
                // Axum upgrades leave the HTTP serve loop; include sessions in server drainage.
                let session = server.spawn_task(handle_socket_with_dispatch(
                    socket,
                    server.clone(),
                    notif_rx,
                    dispatch,
                ));
                // The session is tracked before the pending-upgrade reservation is released.
                drop(pending_upgrade);
                async move {
                    if let Err(e) = session.await {
                        error!("Management session error: {e}");
                    }
                }
            });
            if token_in_sec_ws {
                response.headers_mut().insert(
                    HeaderName::from_static("sec-websocket-protocol"),
                    HeaderValue::from_static("minecraft-v1"),
                );
            }
            response
        }
    }
}

async fn handle_socket_with_dispatch<D, F>(
    socket: WebSocket,
    server: Arc<Server>,
    notif_rx: tokio::sync::broadcast::Receiver<String>,
    dispatch: D,
) where
    D: FnMut(RpcRequest, Arc<Server>) -> F,
    F: Future<Output = (Option<RpcResponse>, bool)>,
{
    debug!("Management WebSocket connection opened");
    tokio::select! {
        () = handle_socket_messages_with_dispatch(socket, &server, notif_rx, dispatch) => {}
        () = async {
            server.management_shutdown.cancelled().await;
            tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;
        } => {
            debug!("Management WebSocket shutdown grace period expired");
        }
    }
    debug!("Management WebSocket connection closed");
}

async fn dispatch_request(request: RpcRequest, server: Arc<Server>) -> (Option<RpcResponse>, bool) {
    handlers::dispatch(request, &server).await
}

async fn handle_socket_messages_with_dispatch<D, F>(
    socket: WebSocket,
    server: &Arc<Server>,
    mut notif_rx: tokio::sync::broadcast::Receiver<String>,
    mut dispatch: D,
) where
    D: FnMut(RpcRequest, Arc<Server>) -> F,
    F: Future<Output = (Option<RpcResponse>, bool)>,
{
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            // JsonRpcNotificationService.serverShuttingDown queues stopping before closure.
            biased;
            notif = notif_rx.recv() => {
                match notif {
                    Ok(notif_str) => {
                        if sender.send(Message::from(notif_str)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Management client lagged behind by {skipped} messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            () = server.management_shutdown.cancelled() => break,
            incoming = receiver.next() => {
                let Some(msg_res) = incoming else { break };
                let msg = match msg_res {
                    Ok(m) => m,
                    Err(e) => {
                        debug!("Management WebSocket error: {e}");
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => {
                        let text_str = text.as_str();
                        let (response, stop_requested) =
                            handle_json_text(text_str, server.clone(), &mut dispatch).await;
                        if stop_requested {
                            if let Some(response) = response {
                                match tokio::time::timeout(
                                    STOP_RPC_REPLY_FLUSH_TIMEOUT,
                                    sender.send(Message::from(response)),
                                )
                                .await
                                {
                                    Ok(Ok(())) => {}
                                    Ok(Err(_)) => return,
                                    Err(_) => {
                                        debug!("Management stop acknowledgement flush timed out");
                                        return;
                                    }
                                }
                            }
                            continue;
                        }
                        let send_failed = if let Some(response) = response {
                            sender.send(Message::from(response)).await.is_err()
                        } else {
                            false
                        };
                        if send_failed {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(p) => {
                        if sender.send(Message::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = sender.close().await;
}

async fn handle_json_text<D, F>(
    text: &str,
    server: Arc<Server>,
    mut dispatch: D,
) -> (Option<String>, bool)
where
    D: FnMut(RpcRequest, Arc<Server>) -> F,
    F: Future<Output = (Option<RpcResponse>, bool)>,
{
    let parsed: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let err = RpcResponse::error(Value::Null, RpcError::parse_error(Some(e.to_string())));
            return (serde_json::to_string(&err).ok(), false);
        }
    };

    match parsed {
        Value::Array(items) => {
            let mut responses = Vec::new();
            let mut stop_requested = false;
            let mut items = items.into_iter();
            while let Some(item) = items.next() {
                if let Ok(req) = serde_json::from_value::<RpcRequest>(item) {
                    let (response, should_stop) = dispatch(req, server.clone()).await;
                    stop_requested |= should_stop;
                    signal_stop_if_requested(should_stop);
                    if let Some(resp) = response {
                        responses.push(resp);
                    }
                    if should_stop {
                        // Signal before formatting responses for the rest of the batch.
                        responses.extend(items.filter_map(stopping_batch_response));
                        break;
                    }
                } else {
                    responses.push(RpcResponse::error(
                        Value::Null,
                        RpcError::invalid_request(None),
                    ));
                }
            }
            let response = if responses.is_empty() {
                None
            } else {
                serde_json::to_string(&responses).ok()
            };
            (response, stop_requested)
        }
        Value::Object(_) => {
            let req: RpcRequest = match serde_json::from_value(parsed) {
                Ok(r) => r,
                Err(e) => {
                    let err = RpcResponse::error(
                        Value::Null,
                        RpcError::invalid_request(Some(e.to_string())),
                    );
                    return (serde_json::to_string(&err).ok(), false);
                }
            };
            let (response, stop_requested) = dispatch(req, server).await;
            signal_stop_if_requested(stop_requested);
            (
                response.and_then(|response| serde_json::to_string(&response).ok()),
                stop_requested,
            )
        }
        _ => {
            let err = RpcResponse::error(Value::Null, RpcError::invalid_request(None));
            (serde_json::to_string(&err).ok(), false)
        }
    }
}

fn signal_stop_if_requested(stop_requested: bool) {
    if stop_requested {
        // Mirrors ServerStateService.stop, which submits halt(false) before returning true.
        crate::stop_server();
    }
}

fn stopping_batch_response(item: Value) -> Option<RpcResponse> {
    let Ok(request) = serde_json::from_value::<RpcRequest>(item) else {
        return Some(RpcResponse::error(
            Value::Null,
            RpcError::invalid_request(None),
        ));
    };
    let id = request.id?;
    if !is_valid_request_id(&id) {
        return Some(RpcResponse::error(
            Value::Null,
            RpcError::invalid_request(Some(
                "Invalid request id - only String, Number and NULL supported".to_string(),
            )),
        ));
    }
    if request.method.is_none() {
        return Some(RpcResponse::error(
            id,
            RpcError::invalid_request(Some("Missing method name".to_string())),
        ));
    }
    Some(RpcResponse::error(
        id,
        RpcError {
            code: SERVER_STOPPING_ERROR_CODE,
            message: "Server is stopping".to_string(),
            data: None,
        },
    ))
}
