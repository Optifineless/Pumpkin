use super::discovery::scan_file;
use super::{HEADER, Record, compare, discover_workspace, parse_manifest, workspace_root};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Self> {
        let id = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pumpkin-inventory-scan-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn source_file(&self, relative: &str, contents: &str) -> std::io::Result<PathBuf> {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap_or(&self.0))?;
        fs::write(&path, contents)?;
        Ok(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn link_rust_sources(
    directory: &Path,
    source_root: &Path,
    scratch_root: &Path,
) -> std::io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            link_rust_sources(&path, source_root, scratch_root)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let relative = path
                .strip_prefix(source_root)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
            let destination = scratch_root.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            if fs::hard_link(&path, &destination).is_err() {
                fs::copy(path, destination)?;
            }
        }
    }
    Ok(())
}

fn mirror_workspace_sources(source_root: &Path, scratch_root: &Path) -> std::io::Result<()> {
    for workspace_dir in ["crates", "tools"] {
        let mut members =
            fs::read_dir(source_root.join(workspace_dir))?.collect::<Result<Vec<_>, _>>()?;
        members.sort_by_key(std::fs::DirEntry::file_name);
        for entry in members {
            let member_root = entry.path();
            if member_root.is_dir() {
                link_rust_sources(&member_root, source_root, scratch_root)?;
            }
        }
    }
    Ok(())
}

fn first_finding(root: &Path, actual: &[Record], expected: &[Record], needle: &str) -> String {
    compare(root, actual, expected)
        .into_iter()
        .find(|finding| finding.contains(needle))
        .unwrap_or_else(|| format!("no finding contained {needle}"))
}

fn serialize_manifest(records: &[Record]) -> String {
    let mut out = format!("{HEADER}\n");
    for row in records {
        out.push_str(
            &[
                row.site_key.as_str(),
                row.workspace_relative_path.as_str(),
                row.enclosing_item.as_str(),
                row.site_kind.as_str(),
                row.field_or_method.as_str(),
                row.state_family.as_str(),
                row.value_class.as_str(),
                row.actor_role.as_str(),
                row.authority_status.as_str(),
                row.boundary_kind.as_str(),
                row.lifecycle_context.as_str(),
                row.alias_or_dependency.as_str(),
                row.migration_item.as_str(),
                row.source_fingerprint.as_str(),
                row.review_note.as_str(),
            ]
            .join("\t"),
        );
        out.push('\n');
    }
    out
}

#[test]
#[ignore = "prints review material only; never writes or accepts manifest rows"]
#[expect(
    clippy::print_stdout,
    reason = "candidate identities and fingerprints aid manual review"
)]
fn print_source_inventory_for_manual_manifest_review() {
    let records = discover_workspace(&workspace_root()).unwrap_or_default();
    println!("{}", serialize_manifest(&records));
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "negative control diagnostics are recorded in the implementation report"
)]
fn unlisted_direct_itemstack_writer_fails_in_scratch_source() {
    let scratch = Scratch::new();
    assert!(
        scratch.is_ok(),
        "could not create source-scan scratch: {:?}",
        scratch.as_ref().err()
    );
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let written = scratch.source_file(
        relative,
        "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn inject(&mut self, stack: ItemStack) { self.main_inventory.push(stack); } }\n",
    );
    assert!(
        written.is_ok(),
        "could not write direct-writer fixture: {:?}",
        written.as_ref().err()
    );
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(&scratch.0, &actual, &[], "stock-mutator::push");
    eprintln!("negative control (unlisted direct ItemStack writer): {finding}");
    assert!(finding.contains("player_inventory.rs:"), "{finding}");
    assert!(
        finding.contains("add an audited manifest record"),
        "{finding}"
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "negative control diagnostics are recorded in the implementation report"
)]
fn changed_fingerprint_requires_audit() {
    let scratch = Scratch::new();
    assert!(
        scratch.is_ok(),
        "could not create source-scan scratch: {:?}",
        scratch.as_ref().err()
    );
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let written = scratch.source_file(
        relative,
        "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn set_stack(&mut self, stack: ItemStack) { self.main_inventory.push(stack); } }\n",
    );
    assert!(
        written.is_ok(),
        "could not write fingerprint fixture: {:?}",
        written.as_ref().err()
    );
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let mut expected = actual.clone();
    let row = expected
        .iter_mut()
        .find(|row| row.site_kind == "stock-mutator");
    assert!(row.is_some(), "fixture did not discover a stock mutator");
    if let Some(row) = row {
        row.source_fingerprint = "0".repeat(64);
    }
    let finding = first_finding(&scratch.0, &actual, &expected, "source fingerprint changed");
    eprintln!("negative control (changed item fingerprint): {finding}");
    assert!(
        finding.contains("audit readers, writers, and lifecycle"),
        "{finding}"
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "source-body fingerprint diagnostics are recorded in the implementation report"
)]
fn changed_helper_body_fails_even_when_setter_token_remains() {
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let before = "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn set_stack(&mut self, stack: ItemStack) { self.main_inventory.push(stack); } }\n";
    assert!(scratch.source_file(relative, before).is_ok());
    let expected = discover_workspace(&scratch.0).unwrap_or_default();
    let after = "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn set_stack(&mut self, stack: ItemStack) { let _unrelated = 17; self.main_inventory.push(stack); } }\n";
    assert!(scratch.source_file(relative, after).is_ok());
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(
        &scratch.0,
        &actual,
        &expected,
        "source fingerprint changed for crates/pumpkin-inventory/src/player/player_inventory.rs::impl TestInventory::set_stack::stock-mutator::push#1",
    );
    eprintln!("negative control (changed helper body, setter retained): {finding}");
    assert!(
        finding.contains("audit readers, writers, and lifecycle"),
        "{finding}"
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "string-literal fingerprint diagnostics are recorded in the implementation report"
)]
fn changed_string_literal_also_requires_audit() {
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let before = "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn set_stack(&mut self, stack: ItemStack) { let _key = \"before\"; self.main_inventory.push(stack); } }\n";
    assert!(scratch.source_file(relative, before).is_ok());
    let expected = discover_workspace(&scratch.0).unwrap_or_default();
    let after = "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn set_stack(&mut self, stack: ItemStack) { let _key = \"after\"; self.main_inventory.push(stack); } }\n";
    assert!(scratch.source_file(relative, after).is_ok());
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(
        &scratch.0,
        &actual,
        &expected,
        "source fingerprint changed for crates/pumpkin-inventory/src/player/player_inventory.rs::impl TestInventory::set_stack::stock-mutator::push#1",
    );
    eprintln!("negative control (changed string literal): {finding}");
    assert!(
        finding.contains("audit readers, writers, and lifecycle"),
        "{finding}"
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "negative control diagnostics are recorded in the implementation report"
)]
fn removed_source_site_makes_manifest_row_stale() {
    let scratch = Scratch::new();
    assert!(
        scratch.is_ok(),
        "could not create source-scan scratch: {:?}",
        scratch.as_ref().err()
    );
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let first = "struct TestInventory { main_inventory: Vec<ItemStack> }\nimpl TestInventory { fn set_stack(&mut self, stack: ItemStack) { self.main_inventory.push(stack); } }\n";
    let written = scratch.source_file(relative, first);
    assert!(
        written.is_ok(),
        "could not write stale-row fixture: {:?}",
        written.as_ref().err()
    );
    let expected = discover_workspace(&scratch.0).unwrap_or_default();
    let replaced = scratch.source_file(relative, "struct TestInventory {}\n");
    assert!(
        replaced.is_ok(),
        "could not replace stale-row fixture: {:?}",
        replaced.as_ref().err()
    );
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(&scratch.0, &actual, &expected, "stale manifest site");
    eprintln!("negative control (stale site): {finding}");
    assert!(finding.contains("player_inventory.rs:"), "{finding}");
    assert!(finding.contains("remove the row"), "{finding}");
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "negative control diagnostics are recorded in the implementation report"
)]
fn removed_live_data_entity_row_fails_on_full_scratch_source_mirror() {
    let root = workspace_root();
    let relative = "crates/pumpkin-core/src/command/commands/data.rs";
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/inventory_ownership_scan/manifest.tsv");
    let contents = fs::read_to_string(&manifest_path);
    assert!(
        contents.is_ok(),
        "could not read complete ownership manifest"
    );
    let parsed = parse_manifest(&contents.unwrap_or_default());
    assert!(
        parsed.is_ok(),
        "could not parse complete ownership manifest"
    );
    let complete = parsed.unwrap_or_default();
    let mut expected = complete.clone();
    let target = expected.iter().position(|row| {
        row.workspace_relative_path == relative
            && row.enclosing_item.ends_with("EntityDataAccessor::set_data")
            && row.field_or_method == "read_nbt_non_mut"
    });
    assert!(
        target.is_some(),
        "complete manifest omitted live /data entity site"
    );
    if let Some(target) = target {
        expected.remove(target);
    }
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let mirrored = mirror_workspace_sources(&root, &scratch.0);
    assert!(
        mirrored.is_ok(),
        "could not mirror workspace Rust sources: {mirrored:?}"
    );
    let scratch_manifest = "crates/pumpkin-core/tests/inventory_ownership_scan/manifest.tsv";
    assert!(
        scratch
            .source_file(scratch_manifest, &serialize_manifest(&expected))
            .is_ok()
    );
    let scratch_contents = fs::read_to_string(scratch.0.join(scratch_manifest));
    assert!(scratch_contents.is_ok(), "could not read scratch manifest");
    let scratch_expected = parse_manifest(&scratch_contents.unwrap_or_default());
    assert!(scratch_expected.is_ok(), "could not parse scratch manifest");
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(
        &scratch.0,
        &actual,
        &scratch_expected.unwrap_or_default(),
        "EntityDataAccessor::set_data::nbt-component-site::read_nbt_non_mut",
    );
    eprintln!("negative control (live /data entity omitted from scratch manifest): {finding}");
    assert!(finding.contains("data.rs:"), "{finding}");
    assert!(
        finding.contains("add an audited manifest record"),
        "{finding}"
    );
    let restored = scratch.source_file(scratch_manifest, &serialize_manifest(&complete));
    assert!(
        restored.is_ok(),
        "could not restore complete scratch manifest"
    );
    let restored_contents = fs::read_to_string(restored.unwrap_or_default());
    assert!(
        restored_contents.is_ok(),
        "could not read restored manifest"
    );
    let restored_rows = parse_manifest(&restored_contents.unwrap_or_default());
    assert!(restored_rows.is_ok(), "could not parse restored manifest");
    let restored_findings = compare(&scratch.0, &actual, &restored_rows.unwrap_or_default());
    assert!(
        restored_findings.is_empty(),
        "restoring the live /data entity row did not restore the scan:\n{}",
        restored_findings.join("\n")
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "atomic writer diagnostics are recorded in the implementation report"
)]
fn unlisted_atomic_xp_writer_fails_in_scratch_source() {
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-core/src/entity/player/experience.rs";
    let written = scratch.source_file(
        relative,
        "use std::sync::atomic::{AtomicI32, Ordering};\nstruct PlayerState { experience_level: AtomicI32 }\nimpl PlayerState { fn pay_xp(&self) { self.experience_level.fetch_sub(1, Ordering::Relaxed); } }\n",
    );
    assert!(written.is_ok(), "could not write atomic-writer fixture");
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(&scratch.0, &actual, &[], "stock-mutator::fetch_sub");
    eprintln!("negative control (unlisted atomic XP writer): {finding}");
    assert!(finding.contains("experience.rs:"), "{finding}");
    assert!(
        finding.contains("add an audited manifest record"),
        "{finding}"
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "held-guard await diagnostics are recorded in the implementation report"
)]
fn unlisted_non_send_guard_await_is_discovered() {
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let source = "use std::sync::Mutex;\nstruct Inventory { main_inventory: Mutex<Vec<ItemStack>> }\nimpl Inventory { async fn guarded(&self) { let guard = self.main_inventory.lock().unwrap(); std::future::pending::<()>().await; drop(guard); } }\n";
    assert!(scratch.source_file(relative, source).is_ok());
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let finding = first_finding(&scratch.0, &actual, &[], "boundary::await#1");
    eprintln!("negative control (unlisted held-guard await): {finding}");
    assert!(finding.contains("player_inventory.rs:"), "{finding}");
    assert!(
        finding.contains("add an audited manifest record"),
        "{finding}"
    );
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "callback and packet boundary diagnostics are recorded in the implementation report"
)]
fn unlisted_callback_and_send_under_mapped_scope_are_discovered() {
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/player/player_inventory.rs";
    let source = "struct Inventory { main_inventory: Mutex<Vec<ItemStack>> }\nimpl Inventory { fn notify(&self) { let guard = self.main_inventory.lock().unwrap(); fire(); send_packet(); drop(guard); } }\n";
    assert!(scratch.source_file(relative, source).is_ok());
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    let findings = compare(&scratch.0, &actual, &[]);
    for site in ["boundary::fire#1", "boundary::send_packet#1"] {
        let finding = findings
            .iter()
            .find(|finding| finding.contains(site))
            .cloned()
            .unwrap_or_else(|| format!("no finding contained {site}"));
        eprintln!("negative control ({site}): {finding}");
        assert!(
            finding.contains("add an audited manifest record"),
            "{finding}"
        );
    }
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "merchant callback diagnostics are recorded in the implementation report"
)]
fn unlisted_merchant_callback_field_and_invocation_are_discovered() {
    let scratch = Scratch::new();
    assert!(scratch.is_ok(), "could not create source-scan scratch");
    let Ok(scratch) = scratch else { return };
    let relative = "crates/pumpkin-inventory/src/merchant/merchant_screen_handler.rs";
    let source = "struct MerchantScreenHandler { on_trade: Option<Arc<dyn Fn()>> }\nimpl MerchantScreenHandler { fn trade(&self) { if let Some(callback) = &self.on_trade { callback(); } } }\n";
    assert!(scratch.source_file(relative, source).is_ok());
    let actual = discover_workspace(&scratch.0).unwrap_or_default();
    for site in ["boundary::on_trade#1", "boundary::callback#1"] {
        let finding = first_finding(&scratch.0, &actual, &[], site);
        eprintln!("negative control ({site}): {finding}");
        assert!(
            finding.contains("add an audited manifest record"),
            "{finding}"
        );
    }
}

#[test]
fn aliases_comments_strings_nested_functions_and_macros_are_scanned() {
    let source = r#"
use crate::item_stack::ItemStack as Stack;
type Stored = Vec<Stack>;
struct Inventory { main_inventory: Stored }
// ItemStack and fake_inventory.push(ItemStack::EMPTY) are ignored here.
const TEXT: &str = "struct Ghost { inventory: ItemStack }";
impl Inventory {
    fn outer(&mut self, stack: Stack) {
        fn nested(inventory: &mut Inventory, stack: Stack) {
            inventory.main_inventory.push(stack);
        }
    }
    fn create(&self) { generated_inventory!(); }
}
#[cfg(test)] mod test_only { struct Fake { inventory: ItemStack } }
#[cfg(not(test))] struct ProductionOnly { inventory: ItemStack }
macro_rules! generated_inventory { () => { ItemStack::EMPTY }; }
"#;
    let records = scan_file(
        "crates/pumpkin-inventory/src/player/player_inventory.rs",
        source,
    );
    assert!(records.iter().any(|row| {
        row.site_kind == "field-declaration" && row.field_or_method == "main_inventory"
    }));
    assert!(records.iter().any(|row| {
        row.enclosing_item.ends_with("outer::nested")
            && row.site_kind == "stock-mutator"
            && row.field_or_method == "push"
    }));
    assert_eq!(
        records
            .iter()
            .filter(|row| {
                row.enclosing_item.ends_with("outer::nested")
                    && row.site_kind == "stock-mutator"
                    && row.field_or_method == "push"
            })
            .count(),
        1
    );
    assert!(!records.iter().any(|row| {
        row.enclosing_item.ends_with("outer")
            && row.site_kind == "stock-mutator"
            && row.field_or_method == "push"
    }));
    assert!(records.iter().any(|row| {
        row.site_kind == "macro-definition" && row.field_or_method == "generated_inventory"
    }));
    assert!(records.iter().any(|row| {
        row.site_kind == "macro-invocation" && row.field_or_method == "generated_inventory"
    }));
    assert!(
        !records
            .iter()
            .any(|row| row.enclosing_item.contains("test_only"))
    );
    assert!(
        records
            .iter()
            .any(|row| row.enclosing_item == "ProductionOnly")
    );
    assert!(!records.iter().any(|row| row.field_or_method == "Ghost"));
}

#[test]
fn classification_and_macro_fixtures_are_not_silently_accepted() {
    let source =
        "struct Holder { stock: ItemStack }\nmacro_rules! holder { () => { ItemStack::EMPTY }; }\n";
    let actual = scan_file(
        "crates/pumpkin-inventory/src/player/player_inventory.rs",
        source,
    );
    let macro_row = actual
        .iter()
        .find(|row| row.site_kind == "macro-definition");
    assert!(
        macro_row.is_some(),
        "mapped macro definition was not inventoried"
    );
    if let Some(row) = macro_row {
        assert!(row.review_note.contains("expansion"));
    }
    let mut invalid: Vec<_> = actual.clone();
    let candidate = invalid
        .iter_mut()
        .find(|row| row.site_kind == "field-declaration");
    assert!(candidate.is_some(), "ItemStack field was not inventoried");
    if let Some(row) = candidate {
        row.value_class = "unclassified".into();
    }
    let finding = first_finding(
        Path::new("."),
        &actual,
        &invalid,
        "unclassified manifest site",
    );
    assert!(finding.contains("complete every required classification column"));
}
