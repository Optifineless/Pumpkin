// Optional native enrichment of ServerWatchdog.createWatchdogCrashReport.
// Called only after the independent halt is armed and the phase report is saved.

fn capture_with_fallback(mut capture: impl FnMut(&str) -> Result<String, String>) -> String {
    let first = match capture("eu-stack") {
        Ok(output) => return output,
        Err(error) => error,
    };
    match capture("gdb") {
        Ok(output) => format!("eu-stack capture failed: {first}\n{output}"),
        Err(error) => format!("eu-stack capture failed: {first}\ngdb capture failed: {error}\n"),
    }
}

fn checked_output(success: bool, status: &str, output: &str) -> Result<String, String> {
    // Both debuggers print numbered frames. A banner or an attach error is no dump,
    // even when the debugger (notably gdb) exits with status zero.
    let has_frame = output.lines().any(|line| {
        let Some(frame) = line.trim_start().strip_prefix('#') else {
            return false;
        };
        let digits = frame.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0
            && frame[digits..].starts_with(char::is_whitespace)
            && !frame[digits..].trim().is_empty()
    });
    if success && has_frame {
        Ok(format!("{status}\n{output}"))
    } else {
        // Leave room for the second failure in the bounded report even if the
        // first debugger printed megabytes of partial frames before failing.
        const MAX_FAILURE_BYTES: usize = 4 * 1024;
        let mut end = output.len().min(MAX_FAILURE_BYTES);
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = if end < output.len() {
            "\nDebugger output truncated.\n"
        } else {
            ""
        };
        Err(format!(
            "{status}; no successful usable stack capture\n{}{truncated}",
            &output[..end]
        ))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn thread_dump() -> String {
    // Two attempts each take at most 2s + 1s kill grace, below the 10s halt fallback.
    let Some(timeout) = find_on_path("timeout") else {
        return "Stack capture skipped: external timeout is unavailable on PATH.\n".into();
    };
    capture_with_fallback(|name| {
        let debugger = find_on_path(name).ok_or_else(|| "unavailable on PATH".to_owned())?;
        capture_stacks(&timeout, &debugger).map_err(|error| error.to_string())
    })
}

#[cfg(target_os = "linux")]
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|path| {
                path.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
    })
}

#[cfg(target_os = "linux")]
fn gated_command(program: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new("/bin/sh");
    // exec preserves the declared launcher PID; timeout's debugger is its descendant.
    // Paths and debugger arguments are positional parameters, never shell source.
    command.args([
        "-c",
        "IFS= read -r ready || exit 1; exec \"$@\"",
        "watchdog",
    ]);
    command.arg(program);
    command
}

#[cfg(target_os = "linux")]
fn authorized_status(
    mut command: std::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = command.stdin(Stdio::piped()).spawn()?;
    let unused: libc::c_ulong = 0;
    // Yama mode 1 permits only this launcher and its descendants, never every PID.
    // SAFETY: PR_SET_PTRACER takes an integer PID, not a pointer. child is alive and
    // blocked on stdin until permission is established, preventing an attach race.
    let allowed = unsafe {
        libc::prctl(
            libc::PR_SET_PTRACER,
            child.id() as libc::c_ulong,
            unused,
            unused,
            unused,
        )
    };
    let release = if allowed == 0 {
        child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("Debugger launcher has no input"))
            .and_then(|mut stdin| stdin.write_all(b"go\n"))
    } else {
        Err(std::io::Error::last_os_error())
    };
    let status = match release {
        Ok(()) => child.wait(),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    };
    // SAFETY: zero revokes this process's temporary tracer declaration.
    let revoked = unsafe { libc::prctl(libc::PR_SET_PTRACER, unused, unused, unused, unused) };
    if revoked != 0 {
        return Err(std::io::Error::last_os_error());
    }
    status
}

#[cfg(target_os = "linux")]
fn capture_stacks(
    timeout: &std::path::Path,
    debugger: &std::path::Path,
) -> std::io::Result<String> {
    use std::io::Read;
    use std::process::Stdio;

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
        let mut command = gated_command(timeout);
        command.args(["--kill-after=1s", "2s"]).arg(debugger);
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
        command
            .stdout(Stdio::from(file.try_clone()?))
            .stderr(Stdio::from(file));
        let status = authorized_status(command)?;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_STACK_BYTES)
            .read_to_end(&mut bytes)?;
        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        if std::fs::metadata(&path)?.len() > MAX_STACK_BYTES {
            output.push_str("\nThread dump truncated at 4 MiB.\n");
        }
        checked_output(
            status.success(),
            &format!("{} exited with {status} (2s timeout)", debugger.display()),
            &output,
        )
        .map_err(std::io::Error::other)
    })();
    let _ = std::fs::remove_file(path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_or_empty_eu_stack_retries_gdb_and_preserves_reasons() {
        for (success, output) in [
            (false, "ptrace: Operation not permitted"),
            (false, "#0 0x123 tick"),
            (true, ""),
            (true, "GNU gdb banner\nptrace: Operation not permitted"),
        ] {
            let mut calls = Vec::new();
            let result = capture_with_fallback(|name| {
                calls.push(name.to_owned());
                if name == "eu-stack" {
                    checked_output(success, "eu-stack exited", output)
                } else {
                    checked_output(true, "gdb exited", "Thread 1\n#0  tick ()\n#1  main ()")
                }
            });
            assert_eq!(calls, ["eu-stack", "gdb"]);
            assert!(result.contains("eu-stack capture failed:"));
            assert!(result.contains(output));
            assert!(result.contains("#0  tick"));
        }
        let result = capture_with_fallback(|name| Err(format!("{name}: permission denied")));
        assert!(result.contains("eu-stack: permission denied"));
        assert!(result.contains("gdb: permission denied"));
        let large_failure = "#0  partial frame\n".repeat(16_000);
        let result = capture_with_fallback(|name| {
            if name == "eu-stack" {
                checked_output(false, "eu-stack exited 1", &large_failure)
            } else {
                Err("gdb: permission denied".into())
            }
        });
        assert!(result.len() < 8 * 1024);
        assert!(result.contains("eu-stack exited 1"));
        assert!(result.contains("gdb: permission denied"));
        let result = capture_with_fallback(|name| {
            assert_eq!(name, "eu-stack");
            checked_output(true, "eu-stack exited", "PID 123\nTID 123:\n#0  0x123 tick")
        });
        assert!(result.contains("#0  0x123 tick"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gated_debugger_child_can_trace_its_parent() {
        const CHILD: &str = "PUMPKIN_WATCHDOG_PTRACE_PARENT";
        if let Some(parent) = std::env::var_os(CHILD) {
            let pid: libc::pid_t = parent.to_str().unwrap().parse().unwrap();
            // SAFETY: attach/wait/detach operate on the live test parent PID.
            unsafe {
                let unused = std::ptr::null_mut::<libc::c_void>();
                assert_eq!(libc::ptrace(libc::PTRACE_ATTACH, pid, unused, unused), 0);
                let mut status = 0;
                let waited = libc::waitpid(pid, &mut status, 0);
                let detached = libc::ptrace(libc::PTRACE_DETACH, pid, unused, unused);
                assert_eq!(waited, pid);
                assert_eq!(detached, 0);
            }
            return;
        }
        // Stricter Yama modes intentionally forbid this unprivileged attachment.
        let scope =
            std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope").unwrap_or_default();
        if matches!(scope.trim(), "2" | "3") {
            return;
        }
        let mut command = gated_command(&std::env::current_exe().unwrap());
        command
            .args(["gated_debugger_child_can_trace_its_parent", "--nocapture"])
            .env(CHILD, std::process::id().to_string());
        assert!(authorized_status(command).unwrap().success());
    }
}
