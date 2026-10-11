#[cfg(any(target_os = "linux", test))]
mod debugger;
mod diagnostics;
mod halt;
pub(crate) mod markers;
mod observations;
pub(crate) mod slots;
mod startup;

#[cfg(test)]
mod allocation_counter;
#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod benchmark;
#[cfg(test)]
mod fatal_tests;
#[cfg(test)]
mod startup_plugin_tests;
#[cfg(test)]
mod startup_tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use diagnostics::PreparedReport;
use halt::HaltGuard;
use markers::{Phase, Record, elapsed_ms};
use slots::Slots;
use tokio::runtime::RuntimeMetrics;

// ServerWatchdog.MAX_SHUTDOWN_TIME and SHUTDOWN_STATUS.
const MAX_SHUTDOWN_TIME: Duration = Duration::from_secs(10);
const SHUTDOWN_STATUS: i32 = 1;
const LOG_FLUSH_DELAY: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const TICK_CLOCK_SHIFT: u32 = 1;
const ACTIVE_TICK: u64 = 1;

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
    // Low bit: active; upper bits: tick start in monotonic milliseconds.
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

    pub(crate) fn begin_tick(&self) {
        if self.max_tick_ms > 0 {
            let now = elapsed_ms(self.epoch);
            self.tick_start
                .store((now << TICK_CLOCK_SHIFT) | ACTIVE_TICK, Ordering::Release);
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

    // ServerWatchdog.run monitors executing ticks regardless of freeze/step/sprint.
    fn stalled_tick(&self, clock: impl FnOnce() -> u64) -> Option<u64> {
        let start = self.tick_start.load(Ordering::Acquire);
        stalled(self.max_tick_ms, start, clock).then_some(start)
    }

    fn detect_and_arm(&self, halt: &HaltGuard) -> Option<u64> {
        let start = self.stalled_tick(|| elapsed_ms(self.epoch))?;
        (self.enabled.load(Ordering::Acquire) && self.tick_start.load(Ordering::Acquire) == start)
            .then(|| {
                // No allocation, logger, catalog read or thread creation before arming.
                halt.arm();
                start >> TICK_CLOCK_SHIFT
            })
    }

    fn run(&self, metrics: &RuntimeMetrics, halt: &HaltGuard, report: &mut PreparedReport) {
        let poll = POLL_INTERVAL.min(Duration::from_millis(self.max_tick_ms as u64));
        while self.enabled.load(Ordering::Acquire) {
            if let Some(start) = self.detect_and_arm(halt) {
                self.crash(metrics, start, report);
            }
            std::thread::park_timeout(poll);
        }
    }

    // ServerWatchdog.run / exit: bypass normal shutdown and all world saves.
    fn crash(
        &self,
        metrics: &RuntimeMetrics,
        tick_start_ms: u64,
        report: &mut PreparedReport,
    ) -> ! {
        self.report(metrics, tick_start_ms, &mut report.buffer);
        // Persist diagnostics before touching a subscriber that might itself be stuck.
        report.save();
        // Debugger setup and filesystem/logging internals may allocate, after the
        // independent fallback is armed and the bounded phase report is persisted.
        report.enrich();
        report.save();
        tracing::error!(
            "A single server tick exceeded {} ms; forcing shutdown without saving",
            self.max_tick_ms
        );
        tracing::error!("Watchdog crash report:\n{}", report.buffer.as_str());
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
        let mut report = PreparedReport::new();
        let halt = match HaltGuard::start() {
            Ok(halt) => halt,
            Err(error) => {
                state.stop();
                return Err(error);
            }
        };
        let (ready, wait_ready) = std::sync::mpsc::sync_channel(0);
        let worker = state.clone();
        match std::thread::Builder::new()
            .name("Server-Watchdog".into())
            .spawn(move || {
                // ArcSwap's reader bookkeeping is warmed on this OS thread.
                drop(worker.worlds.load());
                worker.slots.warm_reader();
                std::thread::park_timeout(Duration::ZERO);
                let _ = ready.send(());
                worker.run(&metrics, &halt, &mut report);
            }) {
            Ok(thread) => {
                let guard = Self {
                    state,
                    thread: Some(thread),
                };
                wait_ready
                    .recv()
                    .map_err(|_| std::io::Error::other("Watchdog failed during startup"))?;
                Ok(Some(guard))
            }
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
fn stalled(max_tick_ms: i64, start: u64, clock: impl FnOnce() -> u64) -> bool {
    max_tick_ms > 0
        && start != 0
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
            || u64::MAX
        ));
        watchdog.begin_tick();
        let start = watchdog.tick_start.load(Ordering::Relaxed);
        assert!(!stalled(60_000, start, || (start >> TICK_CLOCK_SHIFT) + 60_000));
        assert!(stalled(60_000, start, || (start >> TICK_CLOCK_SHIFT) + 60_001));
        watchdog.end_tick();
        assert!(!stalled(
            60_000,
            watchdog.tick_start.load(Ordering::Relaxed),
            || u64::MAX
        ));
        watchdog.stop();
        assert!(!watchdog.enabled.load(Ordering::Relaxed));
        assert_eq!(watchdog.tick_start.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn disabled_configuration_never_fires() {
        for limit in [-1, 0, i64::MIN] {
            assert!(!stalled(limit, 1, || u64::MAX));
        }
    }
}
