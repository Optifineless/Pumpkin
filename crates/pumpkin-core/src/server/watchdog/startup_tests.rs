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

fn callback<E: Payload + Send + Sync + 'static>(
    server: &Server,
    action: impl Fn(&Arc<Server>) + Send + Sync + 'static,
) {
    server
        .plugin_manager
        .register::<E, _>(Arc::new(Callback(action)), EventPriority::Normal, true);
}

fn fixture(path: &std::path::Path) -> PumpkinServer {
    let mut server = combat_test_support::server(path);
    Arc::get_mut(&mut server).unwrap().watchdog = Arc::new(Watchdog::new(i64::MAX));
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
    let watchdog = server.start_ticker().unwrap();
    let (enabled, start) = observed.recv_timeout(Duration::from_secs(30)).unwrap();
    join_ticker(&server);
    assert!(enabled);
    assert_ne!(start & ACTIVE_TICK, 0);
    drop(watchdog);
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
    let watchdog = server.start_ticker().unwrap();
    let (ids, report) = observed.recv_timeout(Duration::from_secs(30)).unwrap();
    join_ticker(&server);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[1], retained_id);
    assert!(report.contains(&format!("World #1 [{retained_id}]: end")));
    drop(watchdog);
}

// `/tick freeze` or `/tick sprint` from the console while an ordinary tick is
// stuck must not exempt that tick; the next tick boundary picks it up.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn freeze_or_sprint_during_a_stuck_tick_keeps_it_eligible() {
    let directory = tempfile::tempdir().unwrap();
    let mut server = combat_test_support::server(directory.path());
    Arc::get_mut(&mut server).unwrap().watchdog = Arc::new(Watchdog::new(1));
    let watchdog = &server.watchdog;
    let past_limit = |start: u64| {
        while elapsed_ms(watchdog.epoch) <= (start >> TICK_CLOCK_SHIFT) + 1 {
            std::thread::yield_now();
        }
    };

    server.watchdog_tick_start();
    let start = watchdog.tick_start.load(Ordering::Acquire);
    server.tick_rate_manager.set_frozen(&server, true);
    past_limit(start);
    assert_eq!(
        watchdog.stalled_tick(),
        Some(start),
        "Freezing during a stuck tick exempted it"
    );
    server
        .tick_rate_manager
        .request_game_to_sprint(&server, 100);
    assert!(server.tick_rate_manager.is_sprinting());
    assert_eq!(
        watchdog.stalled_tick(),
        Some(start),
        "Sprinting during a stuck tick exempted it"
    );
    watchdog.end_tick();

    server.watchdog_tick_start();
    let start = watchdog.tick_start.load(Ordering::Acquire);
    past_limit(start);
    assert_eq!(
        watchdog.stalled_tick(),
        None,
        "A sprint tick was not exempt"
    );
}
