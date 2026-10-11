use super::*;
use std::io::Write as _;

fn report(directory: &std::path::Path) -> PreparedReport {
    let mut report = PreparedReport::new();
    report.directory = directory.to_path_buf();
    report.path = directory.join("watchdog.txt");
    report
        .buffer
        .write_str("Watching Server\nWorld: active entity ticks\n")
        .unwrap();
    report
}

#[test]
fn failed_enrichment_preserves_published_phase_report() {
    let directory = tempfile::tempdir().unwrap();
    let mut report = report(directory.path());
    report.save();
    let baseline = std::fs::read(&report.path).unwrap();
    report
        .buffer
        .write_str("Thread dump:\n#0 tick\n#1 main\n")
        .unwrap();
    let result = report.save_with(|file, text| {
        file.write_all(&text[..7])?;
        Err(std::io::Error::other("injected partial enrichment failure"))
    });
    assert!(result.is_err());
    let saved = std::fs::read(&report.path).unwrap();
    assert!(
        saved.starts_with(&baseline),
        "Enrichment destroyed the phase report"
    );
    assert_eq!(saved.len(), baseline.len() + 7);
}

#[test]
fn successful_enrichment_appends_without_repeating_phase_report() {
    let directory = tempfile::tempdir().unwrap();
    let mut report = report(directory.path());
    report.save();
    report
        .buffer
        .write_str("Thread dump:\n#0 tick\n#1 main\n")
        .unwrap();
    report.save();
    assert_eq!(
        std::fs::read_to_string(&report.path).unwrap(),
        report.buffer.as_str()
    );
}

#[cfg(unix)]
#[test]
fn published_report_is_owner_only_with_permissive_umask() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "PUMPKIN_WATCHDOG_REPORT_PERMISSIONS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "published_report_is_owner_only_with_permissive_umask",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // SAFETY: this isolated child runs one test; no other test creates files.
    unsafe { libc::umask(0) };
    let directory = tempfile::tempdir().unwrap();
    let mut report = report(directory.path());
    report.save();
    assert_eq!(
        std::fs::metadata(&report.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "The published report exposes diagnostics to other users"
    );
}
