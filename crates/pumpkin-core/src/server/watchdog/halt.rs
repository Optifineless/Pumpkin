use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

use super::{MAX_SHUTDOWN_TIME, hard_exit, markers::elapsed_ms};

struct State {
    epoch: Instant,
    armed_at: AtomicU64,
    stopped: AtomicBool,
}

// ServerWatchdog.exit's independent timer, prepared before any gameplay can stall.
pub(super) struct HaltGuard {
    state: Arc<State>,
    thread: Option<JoinHandle<()>>,
}

impl HaltGuard {
    pub(super) fn start() -> std::io::Result<Self> {
        let state = Arc::new(State {
            epoch: Instant::now(),
            armed_at: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        });
        let worker = state.clone();
        let (ready, wait_ready) = std::sync::mpsc::sync_channel(0);
        let thread = std::thread::Builder::new()
            .name("Watchdog-Halt".into())
            .spawn(move || {
                // Initialize the parker before readiness, rather than on a failed tick.
                std::thread::park_timeout(std::time::Duration::ZERO);
                let _ = ready.send(());
                worker.run();
            })?;
        let guard = Self {
            state,
            thread: Some(thread),
        };
        wait_ready
            .recv()
            .map_err(|_| std::io::Error::other("Halt helper failed during startup"))?;
        Ok(guard)
    }

    pub(super) fn arm(&self) {
        self.state
            .armed_at
            .store(elapsed_ms(self.state.epoch) + 1, Ordering::Release);
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
}

impl State {
    fn run(&self) {
        loop {
            let armed_at = self.armed_at.load(Ordering::Acquire);
            if armed_at != 0 {
                let deadline = (armed_at - 1) + MAX_SHUTDOWN_TIME.as_millis() as u64;
                while elapsed_ms(self.epoch) < deadline {
                    std::thread::park_timeout(std::time::Duration::from_millis(
                        deadline.saturating_sub(elapsed_ms(self.epoch)),
                    ));
                }
                hard_exit();
            }
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            std::thread::park();
        }
    }
}

impl Drop for HaltGuard {
    fn drop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            if thread.join().is_err() {
                super::hard_exit();
            }
        }
    }
}
