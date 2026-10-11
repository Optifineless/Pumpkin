use super::startup_tests::{callback, fixture};
use super::*;
use crate::plugin::loader::{PluginLoadFuture, PluginLoader, PluginUnloadFuture};
use crate::plugin::server::server_tick_start::ServerTickStartEvent;
use crate::plugin::{Context, Plugin, PluginFuture, PluginMetadata};
use std::path::Path;
use tokio::sync::Notify;

struct TickPlugin {
    loaded: Arc<AtomicBool>,
}

impl Plugin for TickPlugin {
    fn on_load(&self, context: Arc<Context>) -> PluginFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let tick = Arc::new(Notify::new());
            let arrived = tick.clone();
            callback::<ServerTickStartEvent>(&context.server, move |server| {
                assert!(server.watchdog.enabled.load(Ordering::Acquire));
                assert_ne!(server.watchdog.tick_start.load(Ordering::Acquire), 0);
                arrived.notify_one();
            });
            tick.notified().await;
            self.loaded.store(true, Ordering::Release);
            Ok(())
        })
    }
}

struct TickLoader(Arc<AtomicBool>);

impl PluginLoader for TickLoader {
    fn can_load(&self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|extension| extension == "tick-test")
    }

    fn load<'a>(&'a self, _path: &'a Path) -> PluginLoadFuture<'a> {
        Box::pin(async move {
            let plugin: Arc<dyn Plugin> = Arc::new(TickPlugin {
                loaded: self.0.clone(),
            });
            Ok((
                plugin,
                PluginMetadata {
                    name: "await-startup-tick".into(),
                    version: "1".into(),
                    authors: Vec::new(),
                    description: String::new(),
                    dependencies: Vec::new(),
                    permissions: Vec::new(),
                },
                Box::new(()) as Box<dyn std::any::Any + Send + Sync>,
            ))
        })
    }

    fn unload(&self, _data: Box<dyn std::any::Any + Send + Sync>) -> PluginUnloadFuture<'_> {
        Box::pin(async { Ok(()) })
    }

    fn can_unload(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_on_load_can_await_a_monitored_tick() {
    const CHILD: &str = "PUMPKIN_WATCHDOG_PLUGIN_STARTUP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["plugin_on_load_can_await_a_monitored_tick", "--nocapture"])
            .env(CHILD, "1")
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // Isolate the real plugin directory and process-wide ticker cancellation.
    std::fs::create_dir("plugins").unwrap();
    std::fs::write("plugins/await.tick-test", b"test plugin").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    let loaded = Arc::new(AtomicBool::new(false));
    server
        .server
        .plugin_manager
        .add_loader(&server.server, Arc::new(TickLoader(loaded.clone())))
        .await;
    let result = tokio::time::timeout(Duration::from_secs(30), server.init_plugins()).await;
    let enabled = server.server.watchdog.enabled.load(Ordering::Acquire);
    // start() calls this again; it must reuse the already monitored ticker.
    server.start_ticker().unwrap();
    server.stop_ticker().await;
    assert!(
        result.is_ok(),
        "Plugin on_load could not await its first tick"
    );
    assert!(loaded.load(Ordering::Acquire));
    assert!(
        enabled,
        "Detection ended when plugin initialization returned"
    );
    assert!(!server.server.watchdog.enabled.load(Ordering::Acquire));
    server.unload_plugins().await;
    server.server.shutdown().await;
}
