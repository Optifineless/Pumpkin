#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#[path = "lifecycle_coverage_scan/fixtures.rs"]
mod fixtures;
#[path = "lifecycle_coverage_scan/scan.rs"]
mod scan;
#[path = "sync_guardrail_scan.rs"]
mod syntax;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::{collections::BTreeMap, fs, path::Path};

// Roots are deliberately explicit. A manifest row cannot reduce the scanned surface.
const ROOTS: &[&str] = &[
    "pumpkin-core/src/block/entities",
    "pumpkin-core/src/world",
    "pumpkin-core/src/entity",
    "pumpkin-core/src/data/player_server.rs",
    "pumpkin-core/src/command",
    "pumpkin-world/src/level.rs",
    "pumpkin-world/src/entity_unload.rs",
    "pumpkin-world/src/chunk",
    "pumpkin-world/src/chunk_system",
    "pumpkin-world/src/data/player_data.rs",
    "pumpkin-inventory/src",
    "pumpkin-data/src/item_stack",
    "pumpkin-data/src/data_component_impl",
    "pumpkin-wasm-host-v0_1/src/entity.rs",
    "pumpkin-wasm-host-v0_1/src/living_entity.rs",
    "pumpkin-wasm-host-v0_1/src/player.rs",
    "pumpkin-wasm-host-v0_1/src/block_entity.rs",
    "pumpkin-wasm-host-v0_1/src/world.rs",
    "pumpkin-wasm-host-v0_2/src/entity.rs",
    "pumpkin-wasm-host-v0_2/src/living_entity.rs",
    "pumpkin-wasm-host-v0_2/src/player.rs",
    "pumpkin-wasm-host-v0_2/src/block_entity.rs",
    "pumpkin-wasm-host-v0_2/src/world.rs",
    "pumpkin-core/src/plugin/mod.rs",
    "pumpkin-core/src/net/mod.rs",
    "pumpkin-core/src/net/java/mod.rs",
    "pumpkin-core/src/net/bedrock/mod.rs",
];

fn walk(path: &Path, files: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            walk(&entry?.path(), files)?;
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") {
        files.push(path.into());
    }
    Ok(())
}

fn compare(actual: &BTreeMap<String, String>, manifest: &str) -> Vec<String> {
    let expected: BTreeMap<_, _> = manifest
        .lines()
        .filter(|s| !s.starts_with('#') && !s.is_empty())
        .map(|s| {
            let fields: Vec<_> = s.split('|').collect();
            (
                fields[0].to_owned(),
                fields.get(1).unwrap_or(&"").to_string(),
            )
        })
        .collect();
    let mut errors = Vec::new();
    for row in manifest
        .lines()
        .filter(|s| !s.starts_with('#') && !s.is_empty())
    {
        let fields: Vec<_> = row.split('|').collect();
        if fields.len() != 4
            || !matches!(fields[2], "Legacy" | "BoundaryCheckedLegacy")
            || fields[3].is_empty()
        {
            errors.push(format!(
                "invalid or prematurely certified lifecycle manifest row: {row}"
            ));
        }
        if fields.get(2)
            != Some(
                &["Legacy", "BoundaryCheckedLegacy"]
                    [usize::from(fields[0].starts_with("boundary:"))],
            )
        {
            errors.push(format!("lifecycle status mismatch: {}", fields[0]));
        }
    }
    for (key, fingerprint) in actual {
        match expected.get(key) {
            None => errors.push(format!("unmanifested lifecycle family/site: {key}")),
            Some(old) if old != fingerprint => {
                errors.push(format!("changed lifecycle fingerprint: {key}"));
            }
            _ => {}
        }
    }
    for key in expected.keys().filter(|key| !actual.contains_key(*key)) {
        errors.push(format!("stale lifecycle manifest row: {key}"));
    }
    errors
}

fn scan_file(relative_path: &Path, source: &str) -> (Vec<String>, Vec<(String, String)>) {
    let path = relative_path.to_string_lossy().replace('\\', "/");
    (scan::sites(&path, source), scan::boundaries(&path, source))
}

#[test]
fn lifecycle_sites_match_reviewed_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut actual = BTreeMap::new();
    for relative in ROOTS {
        let mut files = Vec::new();
        walk(&root.join("crates").join(relative), &mut files).unwrap();
        files.sort();
        let mut sites = Vec::new();
        for file in files {
            let source = fs::read_to_string(&file).unwrap();
            let (file_sites, boundaries) = scan_file(file.strip_prefix(root).unwrap(), &source);
            for (key, call) in boundaries {
                actual.insert(key, hex::encode(Sha256::digest(call.as_bytes())));
            }
            sites.extend(file_sites);
        }
        let normalized = sites.join("\n");
        actual.insert(
            (*relative).into(),
            hex::encode(Sha256::digest(normalized.as_bytes())),
        );
    }
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lifecycle_coverage_manifest.txt");
    if std::env::var("LIFECYCLE_MANIFEST_WRITE").as_deref() == Ok("1") {
        let mut text = String::from(
            "# Reviewed lifecycle surface; SHA-256 of exact normalized sites (including enclosing symbol).\n# Legacy raw guards, public fields and native/indirect writes remain uncertified.\n# BoundaryCheckedLegacy means debug reentry detection, never writer exclusion.\n",
        );
        for (key, hash) in &actual {
            let status = if key.starts_with("boundary:") {
                "BoundaryCheckedLegacy"
            } else {
                "Legacy"
            };
            writeln!(
                text,
                "{key}|{hash}|{status}|writer audit; own locks; transitive bounds; owner activation"
            )
            .unwrap();
        }
        fs::write(&manifest, text).unwrap();
    }
    let loaded = fs::read_to_string(manifest).unwrap();
    let errors = compare(&actual, &loaded);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
