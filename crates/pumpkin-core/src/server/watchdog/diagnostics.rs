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
        }
    }

    pub(super) fn save(&self) {
        let result = std::fs::create_dir_all(&self.directory)
            .and_then(|()| std::fs::write(&self.path, self.buffer.as_str()));
        if let Err(error) = result {
            tracing::error!("Unable to save watchdog crash report: {error}");
        }
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
    let Some(debugger) = find_on_path("eu-stack").or_else(|| find_on_path("gdb")) else {
        return "Neither eu-stack nor gdb is available on PATH.\n".into();
    };
    // A debugger can stop every thread, including us. Its deadline must be enforced
    // by another process rather than by a Rust thread inside the stopped server.
    let Some(timeout) = find_on_path("timeout") else {
        return "Stack capture skipped: external timeout is unavailable on PATH.\n".into();
    };
    match capture_stacks(&timeout, &debugger) {
        Ok(output) => output,
        Err(error) => format!("Stack capture failed: {error}\n"),
    }
}

#[cfg(target_os = "linux")]
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|path| path.is_file())
    })
}

#[cfg(target_os = "linux")]
fn capture_stacks(
    timeout: &std::path::Path,
    debugger: &std::path::Path,
) -> std::io::Result<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    const MAX_STACK_BYTES: u64 = 4 * 1024 * 1024;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "pumpkin-watchdog-{}-{timestamp}.txt",
        std::process::id()
    ));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    let result = (|| {
        let mut command = Command::new(timeout);
        command.args(["--kill-after=1s", "5s"]).arg(debugger);
        let pid = std::process::id().to_string();
        if debugger.file_name().is_some_and(|name| name == "eu-stack") {
            command.args(["-p", &pid]);
        } else {
            command.args([
                "--batch",
                "--nx",
                "-p",
                &pid,
                "-ex",
                "set pagination off",
                "-ex",
                "thread apply all bt",
                "-ex",
                "detach",
            ]);
        }
        let status = command
            .stdout(Stdio::from(file.try_clone()?))
            .stderr(Stdio::from(file))
            .status()?;
        let mut output = format!(
            "{} exited with {status} (5s timeout; ptrace permissions may restrict attachment)\n",
            debugger.display()
        );
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_STACK_BYTES)
            .read_to_end(&mut bytes)?;
        output.push_str(&String::from_utf8_lossy(&bytes));
        if std::fs::metadata(&path)?.len() > MAX_STACK_BYTES {
            output.push_str("\nThread dump truncated at 4 MiB.\n");
        }
        Ok(output)
    })();
    let _ = std::fs::remove_file(path);
    result
}
