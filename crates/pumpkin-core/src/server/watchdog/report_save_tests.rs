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

#[cfg(unix)]
fn assert_private_report_directory(test: &str, existing: bool) {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "PUMPKIN_WATCHDOG_DIRECTORY_PERMISSIONS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--nocapture", "--test-threads=1"])
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
    // SAFETY: the isolated child runs one test and owns all its fixture files.
    unsafe { libc::umask(0) };
    let parent = tempfile::tempdir().unwrap();
    let directory = parent.path().join("crash-reports");
    if existing {
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    let mut report = report(&directory);
    report.save_with(std::io::Write::write_all).unwrap();
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700,
        "The report directory lets other users tamper with published diagnostics"
    );
    assert_eq!(
        std::fs::metadata(&report.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        std::fs::read_to_string(&report.path)
            .unwrap()
            .contains("Watching Server")
    );
}

#[cfg(unix)]
#[test]
fn absent_report_directory_is_owner_only_with_permissive_umask() {
    assert_private_report_directory(
        "absent_report_directory_is_owner_only_with_permissive_umask",
        false,
    );
}

#[cfg(unix)]
#[test]
fn existing_insecure_report_directory_becomes_owner_only() {
    assert_private_report_directory(
        "existing_insecure_report_directory_becomes_owner_only",
        true,
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires the source-bound driver's real foreign-UID directory"]
fn foreign_owned_writable_root_publishes_phase_and_enrichment() {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let directory = std::path::PathBuf::from(
        std::env::var_os("PUMPKIN_WATCHDOG_FOREIGN_ROOT")
            .expect("The control driver must provide the actual foreign-owned root"),
    );
    // SAFETY: geteuid only reads the effective identity of this process.
    let uid = unsafe { libc::geteuid() };
    let before = std::fs::metadata(&directory).unwrap();
    assert_eq!(before.uid(), 65534);
    assert_ne!(before.uid(), uid);
    assert_eq!(before.permissions().mode() & 0o777, 0o777);
    let probe = directory.join("actual-service-uid-write-probe");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe)
        .unwrap();
    file.write_all(b"The actual test UID can create a report here")
        .unwrap();
    drop(file);
    assert_eq!(std::fs::metadata(&probe).unwrap().uid(), uid);
    std::fs::remove_file(&probe).unwrap();

    let mut report = report(&directory);
    report.save();
    let phase = std::fs::read_to_string(&report.path);
    report
        .buffer
        .write_str("Thread dump:\n#0 actual appended enrichment\n")
        .unwrap();
    report.save();
    let enriched = std::fs::read_to_string(&report.path);
    assert!(
        phase
            .as_ref()
            .is_ok_and(|text| text.contains("Watching Server"))
            && enriched
                .as_ref()
                .is_ok_and(|text| text == report.buffer.as_str()),
        "A writable foreign-owned root lost both phase and enriched watchdog diagnostics: phase={:?}, enriched={:?}",
        phase.as_ref().err(),
        enriched.as_ref().err(),
    );
    let after = std::fs::metadata(&directory).unwrap();
    assert_eq!(after.uid(), before.uid());
    assert_eq!(after.permissions().mode() & 0o777, 0o777);
    let private = std::fs::metadata(&report.directory).unwrap();
    assert_eq!(private.uid(), uid);
    assert_eq!(private.permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(&report.path).unwrap().uid(), uid);
    assert_eq!(
        std::fs::metadata(&report.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn enrichment_retains_the_first_published_file_after_path_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let mut report = report(directory.path());
    report.save_with(std::io::Write::write_all).unwrap();
    let original = directory.path().join("original-phase.txt");
    std::fs::rename(&report.path, &original).unwrap();
    std::fs::write(&report.path, "replacement file must remain untouched").unwrap();
    report
        .buffer
        .write_str("Thread dump:\n#0 retained file enrichment\n")
        .unwrap();
    report.save_with(std::io::Write::write_all).unwrap();
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        report.buffer.as_str(),
        "Enrichment lost the first published phase file after pathname replacement"
    );
    assert_eq!(
        std::fs::read_to_string(&report.path).unwrap(),
        "replacement file must remain untouched"
    );
}

#[cfg(unix)]
#[test]
fn initial_report_refuses_preexisting_link_without_changing_its_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for symbolic in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("protected-target.txt");
        std::fs::write(&target, "original target bytes").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
        let mut report = report(directory.path());
        if symbolic {
            symlink(&target, &report.path).unwrap();
        } else {
            std::fs::hard_link(&target, &report.path).unwrap();
        }
        assert!(report.save_with(std::io::Write::write_all).is_err());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "original target bytes"
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o666
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires the source-bound driver's real foreign-UID directory"]
fn foreign_root_reuses_the_owned_private_child_for_distinct_reports() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = std::path::PathBuf::from(
        std::env::var_os("PUMPKIN_WATCHDOG_FOREIGN_ROOT").expect("real foreign root"),
    );
    let before = std::fs::metadata(&root).unwrap();
    assert_eq!(before.uid(), 65534);
    let mut published = Vec::new();
    for name in ["first-distinct-report.txt", "second-distinct-report.txt"] {
        let mut report = report(&root);
        report.path = root.join(name);
        report.save_with(std::io::Write::write_all).unwrap();
        report.buffer.write_str("completed enrichment\n").unwrap();
        report.save_with(std::io::Write::write_all).unwrap();
        assert_eq!(
            std::fs::read_to_string(&report.path).unwrap(),
            report.buffer.as_str()
        );
        assert_eq!(
            std::fs::metadata(&report.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&report.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        published.push(report.directory);
    }
    assert_eq!(published[0], published[1]);
    let after = std::fs::metadata(&root).unwrap();
    assert_eq!(after.uid(), before.uid());
    assert_eq!(
        after.permissions().mode() & 0o777,
        before.permissions().mode() & 0o777
    );
}
