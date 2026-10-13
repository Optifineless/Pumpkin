pub mod auth;
pub mod dto;
pub mod handlers;
pub mod hub;
pub mod rpc;

#[cfg(test)]
mod shutdown_tests;

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
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
};
use futures::{SinkExt, StreamExt};
use pumpkin_config::networking::management::ManagementServerConfig;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::server::Server;

use self::{
    auth::SecurityCheckResult,
    dto::{PlayerDto, ServerState, VersionDto},
    rpc::{RpcError, RpcRequest, RpcResponse},
};

pub struct ManagementServer;

// Bound shutdown drainage when a management peer stops reading or an RPC stalls.
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(5);

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

        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(server.management_shutdown.clone().cancelled_owned())
        .await
        {
            error!("Management server error: {e}");
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
    if state.server.management_shutdown.is_cancelled()
        || state.server.management_hub.is_server_stopping()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match auth::check_auth(&headers, &state.secret, &state.allowed_origins) {
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
            // Capture the receiver before the upgrade response can race the stopping broadcast.
            let mut response = ws.on_upgrade(move |socket| {
                // Axum upgrades leave the HTTP serve loop; include sessions in server drainage.
                let session = server.spawn_task(handle_socket(socket, server.clone(), notif_rx));
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

async fn handle_socket(
    socket: WebSocket,
    server: Arc<Server>,
    notif_rx: tokio::sync::broadcast::Receiver<String>,
) {
    debug!("Management WebSocket connection opened");
    tokio::select! {
        () = handle_socket_messages(socket, &server, notif_rx) => {}
        () = async {
            server.management_shutdown.cancelled().await;
            tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;
        } => {
            debug!("Management WebSocket shutdown grace period expired");
        }
    }
    debug!("Management WebSocket connection closed");
}

async fn handle_socket_messages(
    socket: WebSocket,
    server: &Arc<Server>,
    mut notif_rx: tokio::sync::broadcast::Receiver<String>,
) {
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
                        let (response, stop_requested) = handle_json_text(text_str, server).await;
                        let send_failed = if let Some(response) = response {
                            sender.send(Message::from(response)).await.is_err()
                        } else {
                            false
                        };
                        // ServerStateService.stop submits halt(false) after its acknowledgement.
                        if stop_requested {
                            crate::stop_server();
                        }
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

async fn handle_json_text(text: &str, server: &Arc<Server>) -> (Option<String>, bool) {
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
            for item in items {
                if let Ok(req) = serde_json::from_value::<RpcRequest>(item) {
                    let (response, should_stop) = handlers::dispatch(req, server).await;
                    stop_requested |= should_stop;
                    if let Some(resp) = response {
                        responses.push(resp);
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
            let (response, stop_requested) = handlers::dispatch(req, server).await;
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
