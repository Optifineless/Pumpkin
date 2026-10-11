use super::startup_tests::{callback, fixture_with_limit};
use crate::plugin::server::server_tick_start::ServerTickStartEvent;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_real_tick_exits_fatally_with_phase_report() {
    const CHILD: &str = "PUMPKIN_WATCHDOG_FATAL_TICK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let directory = std::env::current_dir().unwrap();
        let server = fixture_with_limit(&directory, 20);
        callback::<ServerTickStartEvent>(&server.server, |_| {
            loop {
                std::thread::park();
            }
        });
        server.start_ticker().unwrap();
        loop {
            std::thread::park();
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let output_path = directory.path().join("child.log");
    let output = std::fs::File::create(&output_path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "server::watchdog::fatal_tests::stalled_real_tick_exits_fatally_with_phase_report",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("PATH", "")
        .current_dir(directory.path())
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "A stalled real tick did not terminate: {}",
                std::fs::read_to_string(&output_path).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(1));
    let reports: Vec<_> = std::fs::read_dir(directory.path().join("crash-reports"))
        .into_iter()
        .flatten()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(reports.len(), 1, "Fatal shutdown omitted its crash report");
    let report = std::fs::read_to_string(&reports[0]).unwrap();
    assert!(report.contains("limit: 20 ms"), "{report}");
    assert!(report.contains("Server: tick start event"), "{report}");
    let elapsed: u64 = report
        .split("Tick elapsed: ")
        .nth(1)
        .unwrap()
        .split(" ms;")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(elapsed > 20, "{report}");
}
