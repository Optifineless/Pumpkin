use std::fmt::Write;

use tokio::runtime::RuntimeMetrics;

use super::Watchdog;
use super::markers::elapsed_ms;
use std::sync::atomic::Ordering;

impl Watchdog {
    // ServerWatchdog.createWatchdogCrashReport and ServerLevel.getWatchdogStats.
    pub(super) fn report(&self, metrics: &RuntimeMetrics, tick_start_ms: u64) -> String {
        let now = elapsed_ms(self.epoch);
        let mut report = format!(
            "Watching Server\nPID: {}\nTick elapsed: {} ms; limit: {} ms\n",
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
        let _ = writeln!(report, "Online players (tick-start snapshot): {players}");
        for (index, world) in worlds.iter().enumerate() {
            let _ = writeln!(
                report,
                "World #{index}: {} ({}), {} loaded chunks, {} players (tick-start snapshot)",
                world.name,
                world.dimension,
                world.chunks.load(Ordering::Relaxed),
                world.players.load(Ordering::Relaxed)
            );
            if let Some(phase) = world.phase.describe(now) {
                let _ = writeln!(report, "World #{index}: {phase}");
            }
        }
        let _ = writeln!(
            report,
            "Tokio: {} workers, {} alive tasks, {} global queued tasks",
            metrics.num_workers(),
            metrics.num_alive_tasks(),
            metrics.global_queue_depth()
        );
        report.push_str(
            "Active worker subjects (best-effort atomic samples; phase age is per world):\n",
        );
        self.slots.append(&mut report);
        report.push_str("Thread dump:\n");
        report.push_str(&thread_dump());
        report
    }
}

pub(super) fn save_report(report: &str) {
    let directory = std::path::Path::new("crash-reports");
    let result = std::fs::create_dir_all(directory).and_then(|()| {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = directory.join(format!("crash-{timestamp}-watchdog.txt"));
        std::fs::write(&path, report)?;
        tracing::error!("Watchdog crash report saved to {}", path.display());
        Ok(())
    });
    if let Err(error) = result {
        tracing::error!("Unable to save watchdog crash report: {error}");
    }
}

#[cfg(not(target_os = "linux"))]
fn thread_dump() -> String {
    "Cross-thread stack capture is unavailable on this platform; use the phase markers.\n".into()
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
