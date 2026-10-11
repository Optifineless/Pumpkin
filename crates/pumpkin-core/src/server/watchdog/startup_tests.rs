use super::*;
use crate::PumpkinServer;
use crate::plugin::server::{
    server_tick_end::ServerTickEndEvent, server_tick_start::ServerTickStartEvent,
};
use crate::plugin::{BoxFuture, EventHandler, EventPriority, Payload};
use crate::server::{Server, combat_test_support};
use std::sync::{Mutex, mpsc};

struct Callback<F>(F);

struct Unload(String);
impl EventHandler<ServerTickStartEvent> for Unload {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _event: &'a mut ServerTickStartEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            server.unload_world(&self.0).await.unwrap();
        })
    }
}

impl<E: Payload, F: Fn(&Arc<Server>) + Send + Sync> EventHandler<E> for Callback<F> {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _event: &'a mut E,
    ) -> BoxFuture<'a, ()> {
        (self.0)(server);
        Box::pin(async {})
    }
}

pub(super) fn callback<E: Payload + Send + Sync + 'static>(
    server: &Server,
    action: impl Fn(&Arc<Server>) + Send + Sync + 'static,
) {
    server
        .plugin_manager
        .register::<E, _>(Arc::new(Callback(action)), EventPriority::Normal, true);
}

pub(super) fn fixture(path: &std::path::Path) -> PumpkinServer {
    fixture_with_limit(path, i64::MAX)
}

pub(super) fn fixture_with_limit(path: &std::path::Path, limit: i64) -> PumpkinServer {
    let mut server = combat_test_support::server(path);
    Arc::get_mut(&mut server).unwrap().watchdog = Arc::new(Watchdog::new(limit));
    let world = combat_test_support::world(&server, path);
    server.worlds.store(Arc::new(vec![world]));
    server.watchdog_prepare_workers();
    PumpkinServer {
        server,
        tcp_listener: None,
        bedrock_status: None,
        nethernet_listener: None,
        ticker: Mutex::new(None),
    }
}

fn join_ticker(server: &PumpkinServer) {
    let ticker = server.ticker.lock().unwrap().take().unwrap();
    // Test callbacks end the real loop by unwinding, without cancelling the
    // process-wide STOP_INTERRUPT shared with other tests.
    assert!(ticker.join().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_readies_watchdog_before_first_ticker_callback() {
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    let (seen, observed) = mpsc::channel();
    callback::<ServerTickStartEvent>(&server.server, move |server| {
        seen.send((
            server.watchdog.enabled.load(Ordering::Acquire),
            server.watchdog.tick_start.load(Ordering::Acquire),
        ))
        .unwrap();
        std::panic::resume_unwind(Box::new("end startup test"));
    });
    assert_eq!(server.server.watchdog.tick_start.load(Ordering::Relaxed), 0);
    server.start_ticker().unwrap();
    let (enabled, start) = observed.recv_timeout(Duration::from_secs(30)).unwrap();
    join_ticker(&server);
    assert!(enabled);
    assert_ne!(start & ACTIVE_TICK, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enabling_during_live_tick_does_not_wait_for_gameplay_workers() {
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    let (seen, observed) = mpsc::channel();
    let (release_tick, wait_tick) = mpsc::channel();
    let wait_tick = Mutex::new(wait_tick);
    callback::<ServerTickStartEvent>(&server.server, move |_| {
        seen.send(()).unwrap();
        wait_tick.lock().unwrap().recv().unwrap();
        std::panic::resume_unwind(Box::new("end late activation test"));
    });
    let ticker_server = server.server.clone();
    *server.ticker.lock().unwrap() = Some(std::thread::spawn(move || {
        crate::server::ticker::Ticker::run(&ticker_server);
    }));
    observed.recv_timeout(Duration::from_secs(30)).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let (occupied, wait_occupied) = mpsc::channel();
    let (release_worker, wait_worker) = mpsc::channel();
    pool.spawn(move || {
        occupied.send(()).unwrap();
        wait_worker.recv().unwrap();
    });
    wait_occupied.recv_timeout(Duration::from_secs(30)).unwrap();
    let (started, wait_started) = mpsc::channel();
    let state = server.server.clone();
    let starter = std::thread::spawn(move || {
        pool.install(|| {
            let watchdog = state.start_watchdog().unwrap();
            started.send(watchdog).unwrap();
        });
    });
    let result = wait_started.recv_timeout(Duration::from_secs(30));
    let start = server.server.watchdog.tick_start.load(Ordering::Acquire);
    release_worker.send(()).unwrap();
    starter.join().unwrap();
    release_tick.send(()).unwrap();
    join_ticker(&server);
    assert!(result.is_ok(), "Startup waited behind a gameplay worker");
    assert_ne!(
        start & ACTIVE_TICK,
        0,
        "The in-progress tick was never armed"
    );
    assert!(stalled(60_000, start, || (start >> TICK_CLOCK_SHIFT) + 60_001));
    drop(result);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_start_world_removal_keeps_dispatch_catalog_identity() {
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    let worlds: Vec<_> = ["overworld", "nether", "end"]
        .into_iter()
        .map(|name| combat_test_support::world(&server.server, &directory.path().join(name)))
        .collect();
    let removed = worlds[1].get_world_name().to_owned();
    let retained_id = worlds[2].uuid;
    server.server.worlds.store(Arc::new(worlds));
    // Drive the real plugin unload entry point from the real ticker event.
    server
        .server
        .plugin_manager
        .register::<ServerTickStartEvent, _>(
            Arc::new(Unload(removed)),
            EventPriority::Normal,
            true,
        );
    let (seen, observed) = mpsc::channel();
    callback::<ServerTickEndEvent>(&server.server, move |server| {
        let catalog = server.watchdog.worlds.load();
        let mut report = diagnostics::ReportBuffer::new();
        server
            .watchdog
            .report(&server.runtime.metrics(), 0, &mut report);
        seen.send((
            catalog.iter().map(|world| world.id).collect::<Vec<_>>(),
            report.as_str().to_owned(),
        ))
        .unwrap();
        std::panic::resume_unwind(Box::new("end world removal test"));
    });
    server.start_ticker().unwrap();
    let (ids, report) = observed.recv_timeout(Duration::from_secs(30)).unwrap();
    join_ticker(&server);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[1], retained_id);
    assert!(report.contains(&format!("World #1 [{retained_id}]: end")));
}

fn command(server: &Arc<Server>, input: &str) {
    let source = crate::command::CommandSender::Console
        .into_source(server)
        .with_silent();
    server
        .command_dispatcher
        .load()
        .execute_input(input, &source)
        .unwrap();
}

// Block the real ticker callback, then observe detection with a synthetic clock.
fn stalled_mode_tick(commands: &[&str], runs_normally: bool) {
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    for input in commands {
        command(&server.server, input);
    }
    let (seen, observed) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    callback::<ServerTickStartEvent>(&server.server, move |server| {
        seen.send(server.tick_rate_manager.runs_normally()).unwrap();
        wait.lock().unwrap().recv().unwrap();
        std::panic::resume_unwind(Box::new("end mode test"));
    });
    server.start_ticker().unwrap();
    let runs_game = observed.recv_timeout(Duration::from_secs(30)).unwrap();
    let start = server.server.watchdog.tick_start.load(Ordering::Acquire);
    let detected = server.server.watchdog.stalled_tick(|| u64::MAX);
    release.send(()).unwrap();
    join_ticker(&server);
    assert_eq!(runs_game, runs_normally);
    assert_ne!(start, 0);
    assert_eq!(
        detected,
        Some(start),
        "Executing tick was exempt: {commands:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frozen_tick_callback_stall_is_monitored() {
    stalled_mode_tick(&["tick freeze"], false);
}

struct FrozenPacketPhase(mpsc::Sender<String>);

impl EventHandler<crate::plugin::server::packet::PacketReceivedEvent> for FrozenPacketPhase {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut crate::plugin::server::packet::PacketReceivedEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        self.0
            .send(server.watchdog.main_phase.describe(0).unwrap().to_string())
            .unwrap();
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frozen_packet_dispatch_reports_server_player_phase() {
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    let world = server.server.worlds.load()[0].clone();
    let player = crate::net::java::combat_test_support::TestPlayer::new(&world);
    player
        .player
        .inbound_packets
        .push(pumpkin_protocol::RawPacket {
            id: -1,
            payload: bytes::Bytes::new(),
        });
    command(&server.server, "tick freeze");
    let (seen, observed) = mpsc::channel();
    server.server.plugin_manager.register(
        Arc::new(FrozenPacketPhase(seen)),
        EventPriority::Normal,
        true,
    );
    callback::<ServerTickEndEvent>(&server.server, |_| {
        std::panic::resume_unwind(Box::new("end frozen packet phase test"));
    });
    server.start_ticker().unwrap();
    let phase = observed.recv_timeout(Duration::from_secs(30)).unwrap();
    join_ticker(&server);
    assert!(
        phase.starts_with("player ticks"),
        "Wrong frozen packet phase: {phase}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stepped_tick_stall_is_monitored() {
    stalled_mode_tick(&["tick freeze", "tick step 1"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn freeze_or_sprint_during_a_stuck_tick_keeps_it_eligible() {
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(directory.path());
    let (seen, observed) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    let ticks = AtomicUsize::new(0);
    callback::<ServerTickStartEvent>(&server.server, move |_| {
        let tick = ticks.fetch_add(1, Ordering::Relaxed);
        seen.send(tick).unwrap();
        wait.lock().unwrap().recv().unwrap();
        if tick == 1 {
            std::panic::resume_unwind(Box::new("end live mode test"));
        }
    });
    server.start_ticker().unwrap();
    assert_eq!(observed.recv_timeout(Duration::from_secs(30)).unwrap(), 0);
    let start = server.server.watchdog.tick_start.load(Ordering::Acquire);
    command(&server.server, "tick freeze");
    let frozen = server.server.watchdog.stalled_tick(|| u64::MAX);
    command(&server.server, "tick sprint 100");
    let sprinting = server.server.watchdog.stalled_tick(|| u64::MAX);
    release.send(()).unwrap();
    assert_eq!(observed.recv_timeout(Duration::from_secs(30)).unwrap(), 1);
    let next = server.server.watchdog.tick_start.load(Ordering::Acquire);
    let sprint_tick = server.server.watchdog.stalled_tick(|| u64::MAX);
    let is_sprinting = server.server.tick_rate_manager.is_sprinting();
    release.send(()).unwrap();
    join_ticker(&server);
    assert_eq!(frozen, Some(start), "Freezing exempted a stuck tick");
    assert_eq!(sprinting, Some(start), "Sprinting exempted a stuck tick");
    assert!(is_sprinting);
    assert_eq!(sprint_tick, Some(next), "A sprint tick was exempt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_keeps_a_hung_tick_monitored_until_join() {
    const CHILD: &str = "PUMPKIN_WATCHDOG_SHUTDOWN_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "shutdown_keeps_a_hung_tick_monitored_until_join",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    // Cancellation is process-wide, so exercise the production stop/join in a child.
    let directory = tempfile::tempdir().unwrap();
    let server = Arc::new(fixture(directory.path()));
    let (seen, observed) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    callback::<ServerTickStartEvent>(&server.server, move |_| {
        seen.send(()).unwrap();
        wait.lock().unwrap().recv().unwrap();
    });
    server.start_ticker().unwrap();
    observed.recv_timeout(Duration::from_secs(30)).unwrap();
    let stopping = server.clone();
    let join = tokio::spawn(async move { stopping.stop_ticker().await });
    crate::STOP_INTERRUPT.cancelled().await;
    let start = server.server.watchdog.tick_start.load(Ordering::Acquire);
    let enabled = server.server.watchdog.enabled.load(Ordering::Acquire);
    let detected = server.server.watchdog.stalled_tick(|| u64::MAX);
    release.send(()).unwrap();
    join.await.unwrap();
    assert!(enabled, "Shutdown disabled detection before ticker exit");
    assert_ne!(start, 0);
    assert_eq!(detected, Some(start));
    assert!(!server.server.watchdog.enabled.load(Ordering::Acquire));
    assert_eq!(server.server.watchdog.tick_start.load(Ordering::Acquire), 0);
    server.server.shutdown().await;
}
