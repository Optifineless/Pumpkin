mod diagnostics;
pub(crate) mod markers;
mod observations;
pub(crate) mod slots;

#[cfg(test)]
mod benchmark;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use markers::{Phase, Record, elapsed_ms};
use slots::Slots;
use tokio::runtime::RuntimeMetrics;

use super::tick_rate_manager::ServerTickRateManager;

// ServerWatchdog.MAX_SHUTDOWN_TIME and SHUTDOWN_STATUS.
const MAX_SHUTDOWN_TIME: Duration = Duration::from_secs(10);
const SHUTDOWN_STATUS: i32 = 1;
const LOG_FLUSH_DELAY: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const TICK_CLOCK_SHIFT: u32 = 2;
const ACTIVE_TICK: u64 = 1;
const EXEMPT_TICK: u64 = 2;

pub(crate) struct WorldStats {
    pub(crate) id: uuid::Uuid,
    pub(crate) name: String,
    pub(crate) dimension: &'static str,
    pub(crate) players: AtomicUsize,
    pub(crate) chunks: AtomicUsize,
    phase: Arc<Record>,
}

pub(crate) struct Watchdog {
    epoch: Instant,
    max_tick_ms: i64,
    enabled: AtomicBool,
    // Low bits: active and exempt; upper bits: tick start in monotonic milliseconds.
    tick_start: AtomicU64,
    main_phase: Record,
    slots: Arc<Slots>,
    worlds: ArcSwap<Vec<WorldStats>>,
}

impl Watchdog {
    pub(crate) fn new(max_tick_ms: i64) -> Self {
        let epoch = Instant::now();
        Self {
            epoch,
            max_tick_ms,
            enabled: AtomicBool::new(false),
            tick_start: AtomicU64::new(0),
            main_phase: Record::new(),
            slots: Arc::new(Slots::new(epoch)),
            worlds: ArcSwap::from_pointee(Vec::new()),
        }
    }

    pub(crate) fn begin_tick(&self, suspended: bool) {
        if self.enabled.load(Ordering::Relaxed) {
            let now = elapsed_ms(self.epoch);
            let exemption = if suspended { EXEMPT_TICK } else { 0 };
            self.tick_start.store(
                (now << TICK_CLOCK_SHIFT) | ACTIVE_TICK | exemption,
                Ordering::Release,
            );
            self.phase(Phase::ServerTick);
        }
    }

    pub(crate) fn end_tick(&self) {
        self.tick_start.store(0, Ordering::Release);
    }

    pub(crate) fn phase(&self, phase: Phase) {
        if self.enabled.load(Ordering::Relaxed) {
            self.main_phase.enter(phase, elapsed_ms(self.epoch));
        }
    }

    pub(crate) fn stop(&self) {
        self.enabled.store(false, Ordering::Release);
        self.end_tick();
    }

    fn run(&self, manager: &ServerTickRateManager, metrics: &RuntimeMetrics) {
        let poll = POLL_INTERVAL.min(Duration::from_millis(self.max_tick_ms as u64));
        while self.enabled.load(Ordering::Acquire) {
            let start = self.tick_start.load(Ordering::Acquire);
            if stalled(
                self.max_tick_ms,
                start,
                manager.is_frozen() || manager.is_sprinting(),
                || elapsed_ms(self.epoch),
            ) && self.enabled.load(Ordering::Acquire)
                && self.tick_start.load(Ordering::Acquire) == start
            {
                self.crash(metrics, start >> TICK_CLOCK_SHIFT);
            }
            std::thread::park_timeout(poll);
        }
    }

    // ServerWatchdog.run / exit: bypass normal shutdown and all world saves.
    fn crash(&self, metrics: &RuntimeMetrics, tick_start_ms: u64) -> ! {
        // Arm before logging: even a stuck subscriber or debugger cannot stop the halt.
        if std::thread::Builder::new()
            .name("Watchdog-Halt".into())
            .spawn(|| {
                std::thread::sleep(MAX_SHUTDOWN_TIME);
                hard_exit();
            })
            .is_err()
        {
            hard_exit();
        }
        let report = self.report(metrics, tick_start_ms);
        // Persist diagnostics before touching a subscriber that might itself be stuck.
        diagnostics::save_report(&report);
        tracing::error!(
            "A single server tick exceeded {} ms; forcing shutdown without saving",
            self.max_tick_ms
        );
        tracing::error!("Watchdog crash report:\n{report}");
        std::thread::sleep(LOG_FLUSH_DELAY);
        hard_exit();
    }
}

// _exit, unlike normal shutdown or C exit hooks, cannot acquire a game lock.
fn hard_exit() -> ! {
    // SAFETY: _exit terminates the process without reading any Rust data.
    unsafe { libc::_exit(SHUTDOWN_STATUS) }
}

/// Owns an explicitly started watchdog thread and disarms it on drop.
pub struct WatchdogGuard {
    state: Arc<Watchdog>,
    thread: Option<JoinHandle<()>>,
}

impl WatchdogGuard {
    pub(crate) fn start(
        state: Arc<Watchdog>,
        manager: Arc<ServerTickRateManager>,
        metrics: RuntimeMetrics,
    ) -> std::io::Result<Option<Self>> {
        // DedicatedServer.initServer starts the daemon only for positive max-tick-time.
        if state.max_tick_ms <= 0 {
            return Ok(None);
        }
        state
            .enabled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "Watchdog is already running",
                )
            })?;
        rayon::broadcast(|_| state.slots.register());
        let worker = state.clone();
        match std::thread::Builder::new()
            .name("Server-Watchdog".into())
            .spawn(move || worker.run(&manager, &metrics))
        {
            Ok(thread) => Ok(Some(Self {
                state,
                thread: Some(thread),
            })),
            Err(error) => {
                state.stop();
                Err(error)
            }
        }
    }
}

impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.state.stop();
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            if thread.join().is_err() {
                tracing::error!("Server watchdog thread panicked");
            }
        }
    }
}

// ServerWatchdog.run's strict > comparison, with a clock injected by the caller.
fn stalled(max_tick_ms: i64, start: u64, suspended: bool, clock: impl FnOnce() -> u64) -> bool {
    max_tick_ms > 0
        && start != 0
        && start & EXEMPT_TICK == 0
        && !suspended
        && clock().saturating_sub(start >> TICK_CLOCK_SHIFT) > max_tick_ms as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_clock_obeys_strict_deadline_and_lifecycle() {
        let watchdog = Watchdog::new(60_000);
        watchdog.enabled.store(true, Ordering::Relaxed);
        // Startup and the intentional inter-tick wait are unarmed.
        assert!(!stalled(
            60_000,
            watchdog.tick_start.load(Ordering::Relaxed),
            false,
            || u64::MAX
        ));
        watchdog.begin_tick(false);
        let start = watchdog.tick_start.load(Ordering::Relaxed);
        assert!(!stalled(60_000, start, false, || (start
            >> TICK_CLOCK_SHIFT)
            + 60_000));
        assert!(stalled(60_000, start, false, || (start
            >> TICK_CLOCK_SHIFT)
            + 60_001));
        assert!(!stalled(60_000, start, true, || u64::MAX));
        watchdog.end_tick();
        assert!(!stalled(
            60_000,
            watchdog.tick_start.load(Ordering::Relaxed),
            false,
            || u64::MAX
        ));
        watchdog.begin_tick(true);
        assert!(!stalled(
            60_000,
            watchdog.tick_start.load(Ordering::Relaxed),
            false,
            || u64::MAX
        ));
        watchdog.stop();
        assert!(!watchdog.enabled.load(Ordering::Relaxed));
        assert_eq!(watchdog.tick_start.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn disabled_configuration_never_fires() {
        for limit in [-1, 0, i64::MIN] {
            assert!(!stalled(limit, 1, false, || u64::MAX));
        }
    }
}
