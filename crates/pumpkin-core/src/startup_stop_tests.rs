//! Startup stops through the real plugin lifecycle, management RPC and shutdown owner.
use std::{
    any::Any,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpStream, sync::Notify};
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

use crate::{
    PumpkinServer, SHOULD_STOP, STOP_INTERRUPT,
    net::management::{ManagementServer, shutdown_child::run_isolated_child_test},
    plugin::{
        BoxFuture, Context, EventHandler, EventPriority, Plugin, PluginFuture, PluginMetadata,
        loader::{PluginLoadFuture, PluginLoader, PluginUnloadFuture},
        server::server_load::ServerLoadEvent,
    },
    server::{Server, combat_test_support},
};

const STAGE_TIMEOUT: Duration = Duration::from_secs(10);
const SECRET: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
type Client = WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

#[derive(Default)]
struct StartupState {
    entered: Notify,
    release: Notify,
    loaded: AtomicBool,
    unloaded: AtomicBool,
    load_events: AtomicUsize,
}

struct StartupPlugin(Arc<StartupState>);

impl Plugin for StartupPlugin {
    fn on_load(&self, context: Arc<Context>) -> PluginFuture<'_, Result<(), String>> {
        Box::pin(async move {
            context.plugin_manager.register::<ServerLoadEvent, _>(
                Arc::new(LoadCounter(self.0.clone())),
                EventPriority::Normal,
                true,
            );
            self.0.entered.notify_one();
            self.0.release.notified().await;
            self.0.loaded.store(true, Ordering::Release);
            Ok(())
        })
    }

    fn on_unload(&self, _context: Arc<Context>) -> PluginFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.0.unloaded.store(true, Ordering::Release);
            Ok(())
        })
    }
}

struct LoadCounter(Arc<StartupState>);

impl EventHandler<ServerLoadEvent> for LoadCounter {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut ServerLoadEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.load_events.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

struct StartupLoader(Arc<StartupState>);

impl PluginLoader for StartupLoader {
    fn can_load(&self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|extension| extension == "startup-test")
    }

    fn load<'a>(&'a self, _path: &'a Path) -> PluginLoadFuture<'a> {
        Box::pin(async move {
            let plugin: Arc<dyn Plugin> = Arc::new(StartupPlugin(self.0.clone()));
            Ok((
                plugin,
                PluginMetadata {
                    name: "blocked-startup".into(),
                    version: "1".into(),
                    authors: Vec::new(),
                    description: String::new(),
                    dependencies: Vec::new(),
                    permissions: Vec::new(),
                },
                Box::new(()) as Box<dyn Any + Send + Sync>,
            ))
        })
    }

    fn unload(&self, _data: Box<dyn Any + Send + Sync>) -> PluginUnloadFuture<'_> {
        Box::pin(async { Ok(()) })
    }

    fn can_unload(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn management_stop_during_plugin_initialization_gates_startup() {
    isolated_scenario(
        "startup_stop_tests::management_stop_during_plugin_initialization_gates_startup",
        "PUMPKIN_MANAGEMENT_PLUGIN_STARTUP_STOP_CHILD",
        true,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_initialization_without_stop_starts_normally() {
    isolated_scenario(
        "startup_stop_tests::plugin_initialization_without_stop_starts_normally",
        "PUMPKIN_MANAGEMENT_PLUGIN_STARTUP_CONTROL_CHILD",
        false,
    )
    .await;
}

async fn isolated_scenario(test_name: &str, child_env: &str, stop_before_release: bool) {
    if std::env::var_os(child_env).is_none() {
        assert_eq!(run_isolated_child_test(test_name, child_env).await, Ok(()));
        return;
    }
    // Plugin paths, logger initialization and stop state are process-wide.
    let directory = tempfile::tempdir().unwrap();
    std::env::set_current_dir(directory.path()).unwrap();
    std::fs::create_dir("plugins").unwrap();
    std::fs::write("plugins/blocked.startup-test", b"test plugin").unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        startup_scenario(directory.path(), stop_before_release),
    )
    .await
    .expect("startup regression exceeded its child crash guard");
}

async fn fixture(path: &Path, state: Arc<StartupState>) -> (Arc<PumpkinServer>, SocketAddr) {
    let mut server = combat_test_support::server(path);
    let configuration = Arc::get_mut(&mut server).unwrap();
    configuration.advanced_config.logging.enabled = false;
    configuration.advanced_config.commands.use_console = false;
    configuration.advanced_config.plugins.hot_reload = true;
    configuration.telemetry_config.enabled = false;
    let management = &mut configuration.advanced_config.networking.management;
    management.enabled = true;
    management.address = "127.0.0.1:0".parse().unwrap();
    management.secret = SECRET.into();
    crate::init_logger(&server.advanced_config);
    server
        .management_hub
        .settings
        .status_heartbeat_interval
        .store(0, Ordering::Relaxed);
    let world = combat_test_support::world(&server, path);
    server.worlds.store(Arc::new(vec![world]));
    server.watchdog_prepare_workers();
    server
        .plugin_manager
        .add_loader(&server, Arc::new(StartupLoader(state)))
        .await;
    let config = server.advanced_config.networking.management.clone();
    let listener = ManagementServer::bind(&config).await.unwrap();
    let address = listener.local_addr().unwrap();
    let management_server = server.clone();
    server.spawn_task(async move {
        ManagementServer::run(&config, management_server, listener).await;
    });
    (
        Arc::new(PumpkinServer {
            server,
            tcp_listener: None,
            bedrock_status: None,
            nethernet_listener: None,
            ticker: Mutex::new(None),
        }),
        address,
    )
}

async fn connect_client(address: SocketAddr) -> Client {
    let mut request = format!("ws://{address}/").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {SECRET}").parse().unwrap());
    connect_async(request).await.unwrap().0
}

async fn send_stop(client: &mut Client) {
    client
        .send(Message::Text(
            json!({"jsonrpc":"2.0", "id":1, "method":"minecraft:server/stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

async fn receive_json(client: &mut Client) -> Value {
    let message = tokio::time::timeout(STAGE_TIMEOUT, client.next())
        .await
        .expect("management reply did not arrive")
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn startup_scenario(path: &Path, stop_before_release: bool) {
    let state = Arc::new(StartupState::default());
    let (server, address) = fixture(path, state.clone()).await;
    let mut client = connect_client(address).await;
    let (initialized, wait_initialized) = tokio::sync::oneshot::channel();
    let startup = server.clone();
    let owner = tokio::spawn(async move {
        startup.init_plugins().await;
        initialized
            .send(startup.server.plugin_manager.is_hot_reload_enabled())
            .unwrap();
        startup.start().await;
    });
    tokio::time::timeout(STAGE_TIMEOUT, state.entered.notified())
        .await
        .expect("plugin initialization did not reach its blocked boundary");
    assert!(!state.loaded.load(Ordering::Acquire));
    assert_eq!(state.load_events.load(Ordering::Relaxed), 0);

    if stop_before_release {
        send_stop(&mut client).await;
        let reply = receive_json(&mut client).await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"], true);
        assert!(SHOULD_STOP.load(Ordering::Relaxed));
        assert!(STOP_INTERRUPT.is_cancelled());
        assert!(!state.loaded.load(Ordering::Acquire));
        assert!(!owner.is_finished());
    }

    // Keep on_load owned until it completes, then let the ordinary owner save and stop.
    state.release.notify_one();
    let watching = tokio::time::timeout(STAGE_TIMEOUT, wait_initialized)
        .await
        .expect("plugin initialization did not finish after release")
        .unwrap();
    assert_eq!(watching, !stop_before_release);
    if !stop_before_release {
        let started = receive_json(&mut client).await;
        assert_eq!(started["method"], "minecraft:notification/server/started");
        assert_eq!(state.load_events.load(Ordering::Relaxed), 1);
        send_stop(&mut client).await;
    }
    tokio::time::timeout(STAGE_TIMEOUT, owner)
        .await
        .expect("the normal shutdown owner did not finish promptly")
        .unwrap();

    let mut stopping = 0;
    while let Some(message) = tokio::time::timeout(STAGE_TIMEOUT, client.next())
        .await
        .expect("management session did not close after shutdown")
    {
        let message = message.unwrap();
        if message.is_close() {
            break;
        }
        let notification: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_ne!(
            notification["method"],
            "minecraft:notification/server/started"
        );
        if notification["method"] == "minecraft:notification/server/stopping" {
            stopping += 1;
        }
    }
    assert_eq!(stopping, 1);
    assert_eq!(
        state.load_events.load(Ordering::Relaxed),
        usize::from(!stop_before_release)
    );
    assert!(state.loaded.load(Ordering::Acquire));
    assert!(state.unloaded.load(Ordering::Acquire));
    assert!(server.server.tasks.is_empty());
    assert!(path.join("level.dat").exists());
    assert!(crate::SERVER_IS_STOPPING.load(Ordering::Acquire));
    assert!(server.ticker.lock().unwrap().is_none());
    let _rebound = tokio::net::TcpListener::bind(address).await.unwrap();
}
