#[path = "inventory_ownership_scan/discovery.rs"]
mod discovery;
#[path = "inventory_ownership_scan/fixtures.rs"]
mod fixtures;
#[path = "sync_guardrail_scan/source.rs"]
mod source;

use discovery::discover_workspace;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const HEADER: &str = "site_key\tworkspace_relative_path\tenclosing_item\tsite_kind\tfield_or_method\tstate_family\tvalue_class\tactor_role\tauthority_status\tboundary_kind\tlifecycle_context\talias_or_dependency\tmigration_item\tsource_fingerprint\treview_note";
const REQUIRED_FAMILIES: &[&str] = &[
    "player_stock_payment_use",
    "menu_identity_cursor_payment",
    "menu_specific_stock_payment",
    "world_container_stock_metadata",
    "stored_itemstack_classification",
    "packet_local_state",
    "execution_actor_boundary",
    "retained_entity_stock_payload",
    "lifecycle_snapshot_retirement",
];
const REQUIRED_ROLES: &[&str] = &[
    "java-packet",
    "bedrock-packet",
    "command-dispatch",
    "block-tick-or-use",
    "entity-tick-or-collision",
    "native-plugin-api",
    "wasm-host-v0_1",
    "wasm-host-v0_2",
];
#[rustfmt::skip]
const ACTOR_ROLES: &[&str] = &["java-packet", "bedrock-packet", "command-dispatch", "block-tick-or-use", "entity-tick-or-collision", "native-plugin-api", "wasm-host-common", "wasm-host-v0_1", "wasm-host-v0_2", "world-lifecycle-or-automation", "inventory-api", "item-data-api", "protocol-boundary", "core-gameplay", "workspace-api"];
const VALUES: &[&str] = &[
    "physical stock",
    "recoverable projectile stock",
    "XP budget",
    "eligibility/payment state",
    "derived preview/cache",
    "effect/display payload",
    "retained observation",
    "unresolved alias",
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct Record {
    site_key: String,
    workspace_relative_path: String,
    enclosing_item: String,
    site_kind: String,
    field_or_method: String,
    state_family: String,
    value_class: String,
    actor_role: String,
    authority_status: String,
    boundary_kind: String,
    lifecycle_context: String,
    alias_or_dependency: String,
    migration_item: String,
    source_fingerprint: String,
    review_note: String,
    line: usize,
}

fn digest(tokens: &[source::Token<'_>]) -> String {
    let mut hash = Sha256::new();
    for token in tokens {
        if let Some(literal) = token.literal_before {
            hash.update(literal.as_bytes());
            hash.update([0]);
        }
        hash.update(token.text.as_bytes());
        hash.update([0]);
    }
    let bytes = hash.finalize();
    let hex = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(hex[usize::from(byte >> 4)]));
        output.push(char::from(hex[usize::from(byte & 0x0f)]));
    }
    output
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

fn record(
    path: &str,
    item: &str,
    kind: &str,
    field: &str,
    occurrence: usize,
    line: usize,
    tokens: &[source::Token<'_>],
) -> Record {
    let macro_site = matches!(kind, "macro-definition" | "macro-invocation");
    Record {
        site_key: format!("{path}::{item}::{kind}::{field}#{occurrence}"),
        workspace_relative_path: path.into(),
        enclosing_item: clean(item),
        site_kind: kind.into(),
        field_or_method: clean(field),
        state_family: "menu_specific_stock_payment".into(),
        value_class: "unresolved alias".into(),
        actor_role: "workspace-api".into(),
        authority_status: "legacy".into(),
        boundary_kind: "storage-local".into(),
        lifecycle_context: "context-dependent".into(),
        alias_or_dependency: if macro_site {
            "unanalysed-macro-expansion: expansion coverage remains a named debt"
        } else {
            "review placeholder: assign a source-backed owner and dependency"
        }
        .into(),
        migration_item: if macro_site {
            "debt: macro expansion is not scanned; inventory expansion sites before cohort activation"
        } else {
            "debt: source classification requires review; touching cohort remains blocked"
        }
        .into(),
        source_fingerprint: digest(tokens),
        review_note: if macro_site {
            "E1 token source is covered; macro expansion is not expanded and remains a named audit debt"
        } else {
            "E1 review placeholder only; committed manifest classifications are authoritative"
        }
        .into(),
        line,
    }
}

#[derive(Clone, Copy)]
struct Candidate<'a, 'source> {
    item: &'a str,
    kind: &'a str,
    field: &'a str,
    line: usize,
    fingerprint: &'a [source::Token<'source>],
}

struct RecordSink<'path> {
    path: &'path str,
    ordinals: BTreeMap<(String, String, String), usize>,
}

impl RecordSink<'_> {
    const fn new(path: &str) -> RecordSink<'_> {
        RecordSink {
            path,
            ordinals: BTreeMap::new(),
        }
    }

    fn push(&mut self, out: &mut Vec<Record>, candidate: Candidate<'_, '_>) {
        let key = (
            candidate.item.to_owned(),
            candidate.kind.to_owned(),
            candidate.field.to_owned(),
        );
        let occurrence = self.ordinals.entry(key).or_default();
        *occurrence += 1;
        out.push(record(
            self.path,
            candidate.item,
            candidate.kind,
            candidate.field,
            *occurrence,
            candidate.line,
            candidate.fingerprint,
        ));
    }

    fn push_token(
        &mut self,
        out: &mut Vec<Record>,
        item: &str,
        kind: &str,
        fingerprint: &[source::Token<'_>],
        token: &source::Token<'_>,
    ) {
        self.push(
            out,
            Candidate {
                item,
                kind,
                field: token.text,
                line: token.line,
                fingerprint,
            },
        );
    }
}

fn parse_manifest(contents: &str) -> Result<Vec<Record>, String> {
    let mut lines = contents.lines();
    if lines.next() != Some(HEADER) {
        return Err("manifest header does not match the required 15-column schema".into());
    }
    let mut out = Vec::new();
    for (index, line) in lines.enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = line.split('\t').collect();
        if columns.len() != 15 {
            return Err(format!(
                "manifest.tsv:{} has {} columns; expected 15",
                index + 2,
                columns.len()
            ));
        }
        out.push(Record {
            site_key: columns[0].into(),
            workspace_relative_path: columns[1].into(),
            enclosing_item: columns[2].into(),
            site_kind: columns[3].into(),
            field_or_method: columns[4].into(),
            state_family: columns[5].into(),
            value_class: columns[6].into(),
            actor_role: columns[7].into(),
            authority_status: columns[8].into(),
            boundary_kind: columns[9].into(),
            lifecycle_context: columns[10].into(),
            alias_or_dependency: columns[11].into(),
            migration_item: columns[12].into(),
            source_fingerprint: columns[13].into(),
            review_note: columns[14].into(),
            line: 0,
        });
    }
    Ok(out)
}

#[rustfmt::skip]
fn classified(row: &Record) -> bool {
    let authority = ["legacy", "detached-init", "admitted", "retirement", "snapshot", "detection/publication"].contains(&row.authority_status.as_str()) && row.authority_status != "admitted";
    let boundary = ["storage-local", "arbitrary policy", "callback", "packet send", "await", "synchronous wait/bridge", "lifecycle publication"].contains(&row.boundary_kind.as_str());
    let lifecycle = ["detached construction/load", "live mutation", "promotion", "replacement", "retirement", "capture", "context-dependent"].contains(&row.lifecycle_context.as_str());
    let migration = if row.migration_item.starts_with("debt:") { row.migration_item.contains("cohort") } else { let mut parts = row.migration_item.split(';'); matches!(parts.next(), Some(p) if p.starts_with('P')) && matches!(parts.next(), Some(e) if e.starts_with('E')) && parts.next().is_none() };
    VALUES.contains(&row.value_class.as_str()) && REQUIRED_FAMILIES.contains(&row.state_family.as_str()) && ACTOR_ROLES.contains(&row.actor_role.as_str()) && authority && boundary && lifecycle
        && !row.alias_or_dependency.is_empty() && !row.review_note.is_empty() && migration && row.source_fingerprint.len() == 64
        && row.source_fingerprint.chars().all(|ch| ch.is_ascii_hexdigit())
        && (!matches!(row.site_kind.as_str(), "macro-definition" | "macro-invocation") || row.review_note.contains("expansion") && row.alias_or_dependency.contains("macro") && row.migration_item.starts_with("debt:"))
}

fn identity_matches(actual: &Record, expected: &Record) -> bool {
    actual.workspace_relative_path == expected.workspace_relative_path
        && actual.enclosing_item == expected.enclosing_item
        && actual.site_kind == expected.site_kind
        && actual.field_or_method == expected.field_or_method
}

fn diagnostic_line(root: &Path, row: &Record) -> usize {
    let path = root.join(&row.workspace_relative_path);
    let Ok(contents) = fs::read_to_string(path) else {
        return 1;
    };
    let name = row
        .enclosing_item
        .rsplit("::")
        .next()
        .unwrap_or(&row.enclosing_item);
    source::tokenize(&contents)
        .iter()
        .find(|token| token.text == name.trim_start_matches("macro_rules!"))
        .map_or(1, |token| token.line)
}

fn compare(root: &Path, actual: &[Record], expected: &[Record]) -> Vec<String> {
    let actual_by_key: BTreeMap<_, _> = actual
        .iter()
        .map(|row| (row.site_key.as_str(), row))
        .collect();
    let expected_by_key: BTreeMap<_, _> = expected
        .iter()
        .map(|row| (row.site_key.as_str(), row))
        .collect();
    let mut findings = Vec::new();
    if actual_by_key.len() != actual.len() {
        findings.push(
            "coverage:1: duplicate discovered site key; disambiguate source occurrence identity"
                .into(),
        );
    }
    if expected_by_key.len() != expected.len() {
        findings.push(
            "manifest.tsv:1: duplicate site_key; keep one audited record per source site".into(),
        );
    }
    for row in actual {
        match expected_by_key.get(row.site_key.as_str()) {
            None => findings.push(format!(
                "{}:{}: unlisted source site {}; add an audited manifest record with all classifications",
                row.workspace_relative_path, row.line, row.site_key
            )),
            Some(old) if !identity_matches(row, old) => findings.push(format!(
                "{}:{}: manifest identity does not match discovered site {}; review and update the source identity",
                row.workspace_relative_path, row.line, row.site_key
            )),
            Some(old) if !classified(old) => findings.push(format!(
                "{}:{}: unclassified manifest site {}; complete every required classification column",
                row.workspace_relative_path, row.line, row.site_key
            )),
            Some(old) if old.source_fingerprint != row.source_fingerprint => findings.push(format!(
                "{}:{}: source fingerprint changed for {}; audit readers, writers, and lifecycle, then update the manifest fingerprint",
                row.workspace_relative_path, row.line, row.site_key
            )),
            _ => {}
        }
    }
    for row in expected {
        if !actual_by_key.contains_key(row.site_key.as_str()) {
            findings.push(format!(
                "{}:{}: stale manifest site {}; remove the row or restore and audit the source site",
                row.workspace_relative_path, diagnostic_line(root, row), row.site_key
            ));
        }
    }
    let present_families: BTreeSet<_> = expected
        .iter()
        .map(|row| row.state_family.as_str())
        .collect();
    for family in REQUIRED_FAMILIES {
        if !present_families.contains(family) {
            findings.push(format!("manifest.tsv:1: state family {family} is missing; inventory its mapped source sites"));
        }
    }
    let present_roles: BTreeSet<_> = expected.iter().map(|row| row.actor_role.as_str()).collect();
    for role in REQUIRED_ROLES {
        if !present_roles.contains(role) {
            findings.push(format!("manifest.tsv:1: actor role {role} is missing; discover and classify its source surfaces"));
        }
    }
    let present_values: BTreeSet<_> = expected
        .iter()
        .map(|row| row.value_class.as_str())
        .collect();
    for class in VALUES {
        if !present_values.contains(class) {
            findings.push(format!("manifest.tsv:1: value class {class} is missing; audit the corresponding retained values"));
        }
    }
    findings.sort();
    findings
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&manifest)
        .to_path_buf()
}

#[test]
fn inventory_ownership_manifest_covers_discovered_source() {
    let root = workspace_root();
    let actual = discover_workspace(&root);
    assert!(
        actual.is_ok(),
        "inventory source discovery failed: {:?}",
        actual.as_ref().err()
    );
    let actual = actual.unwrap_or_default();
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/inventory_ownership_scan/manifest.tsv");
    let contents = fs::read_to_string(&manifest_path);
    assert!(
        contents.is_ok(),
        "could not read {}",
        manifest_path.display()
    );
    let expected = parse_manifest(&contents.unwrap_or_default());
    assert!(
        expected.is_ok(),
        "invalid manifest {}: {:?}",
        manifest_path.display(),
        expected.as_ref().err()
    );
    let expected = expected.unwrap_or_default();
    let findings = compare(&root, &actual, &expected);
    assert!(
        findings.is_empty(),
        "inventory ownership coverage mismatch (see README.md update procedure):\n{}",
        findings
            .iter()
            .take(100)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
