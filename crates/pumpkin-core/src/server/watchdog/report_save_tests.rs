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
