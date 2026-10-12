use std::fmt::Write;

use tokio::runtime::RuntimeMetrics;

use super::Watchdog;
use super::markers::elapsed_ms;
use std::sync::atomic::Ordering;

// Bounded diagnostic output: growth must never need the allocator on a failed tick.
const REPORT_CAPACITY: usize = 64 * 1024;
const TRUNCATED: &str = "\nWatchdog report truncated at 64 KiB.\n";

pub(super) struct ReportBuffer {
    text: String,
    truncated: bool,
}

impl ReportBuffer {
    pub(super) fn new() -> Self {
        Self {
            text: String::with_capacity(REPORT_CAPACITY),
            truncated: false,
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.text
    }

    pub(super) fn finish(&mut self) {
        if self.truncated {
            self.text.push_str(TRUNCATED);
        }
    }
}

impl Write for ReportBuffer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if self.truncated {
            return Err(std::fmt::Error);
        }
        let remaining = REPORT_CAPACITY - TRUNCATED.len() - self.text.len();
        if text.len() <= remaining {
            self.text.push_str(text);
            return Ok(());
        }
        let mut boundary = remaining;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.text.push_str(&text[..boundary]);
        self.truncated = true;
        Err(std::fmt::Error)
    }
}

pub(super) struct PreparedReport {
    pub(super) buffer: ReportBuffer,
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
    saved_len: usize,
    file: Option<std::fs::File>,
}

impl PreparedReport {
    pub(super) fn new() -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::path::PathBuf::from("crash-reports");
        let path = directory.join(format!(
            "crash-{timestamp}-{}-watchdog.txt",
            std::process::id()
        ));
        Self {
            buffer: ReportBuffer::new(),
            directory,
            path,
            saved_len: 0,
            file: None,
        }
    }

    pub(super) fn save(&mut self) {
        if let Err(error) = self.save_with(std::io::Write::write_all) {
            tracing::error!("Unable to save watchdog crash report: {error}");
        }
    }

    fn save_with(
        &mut self,
        write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        if self.file.is_none() {
            self.file = Some(self.open_report()?);
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("report file was not opened"))?;
        // The already-published phase report must survive a failed or interrupted enrichment.
        write(file, &self.buffer.as_str().as_bytes()[self.saved_len..])?;
        self.saved_len = self.buffer.as_str().len();
        Ok(())
    }

    #[cfg(unix)]
    fn open_report(&mut self) -> std::io::Result<std::fs::File> {
        let (file, path) = report_file::open_report(&self.directory, &self.path)?;
        self.directory = path
            .parent()
            .ok_or_else(|| std::io::Error::other("report directory is missing"))?
            .to_path_buf();
        self.path = path;
        Ok(file)
    }

    #[cfg(not(unix))]
    fn open_report(&self) -> std::io::Result<std::fs::File> {
        std::fs::create_dir_all(&self.directory)?;
        std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&self.path)
    }

    pub(super) fn enrich(&mut self) {
        let _ = writeln!(self.buffer, "Thread dump:\n{}", thread_dump());
        self.buffer.finish();
    }
}

impl Watchdog {
    // ServerWatchdog.createWatchdogCrashReport and ServerLevel.getWatchdogStats.
    pub(super) fn report(
        &self,
        metrics: &RuntimeMetrics,
        tick_start_ms: u64,
        report: &mut ReportBuffer,
    ) {
        let now = elapsed_ms(self.epoch);
        let _ = writeln!(
            report,
            "Watching Server\nPID: {}\nTick elapsed: {} ms; limit: {} ms",
            std::process::id(),
            now.saturating_sub(tick_start_ms),
            self.max_tick_ms
        );
        if let Some(phase) = self.main_phase.describe(now) {
            let _ = writeln!(report, "Server: {phase}");
        }
        let worlds = self.worlds.load();
        let players: usize = worlds
            .iter()
            .map(|world| world.players.load(Ordering::Relaxed))
            .sum();
        let _ = writeln!(
            report,
            "Online players (world-dispatch snapshot): {players}"
        );
        for (index, world) in worlds.iter().enumerate() {
            let _ = writeln!(
                report,
                "World #{index} [{}]: {} ({}), {} loaded chunks, {} players (world-dispatch snapshot)",
                world.id,
                world.name,
                world.dimension,
                world.chunks.load(Ordering::Relaxed),
                world.players.load(Ordering::Relaxed)
            );
            if let Some(phase) = world.phase.describe(now) {
                let _ = writeln!(report, "World #{index} [{}]: {phase}", world.id);
            }
        }
        let _ = writeln!(
            report,
            "Tokio: {} workers, {} alive tasks, {} global queued tasks",
            metrics.num_workers(),
            metrics.num_alive_tasks(),
            metrics.global_queue_depth()
        );
        let _ = report.write_str(
            "Active worker subjects (best-effort atomic samples; phase age is per world):\n",
        );
        self.slots
            .append(report, |index| worlds.get(index).map(|world| world.id));
    }
}

#[cfg(not(target_os = "linux"))]
const fn thread_dump() -> &'static str {
    "Cross-thread stack capture is unavailable on this platform; use the phase markers.\n"
}

#[cfg(target_os = "linux")]
fn thread_dump() -> String {
    super::debugger::thread_dump()
}

#[cfg(test)]
#[path = "report_save_tests.rs"]
mod report_save_tests;

#[cfg(unix)]
#[path = "report_file.rs"]
mod report_file;
