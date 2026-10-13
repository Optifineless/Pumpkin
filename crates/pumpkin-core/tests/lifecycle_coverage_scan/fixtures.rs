use super::{compare, scan};
use std::collections::BTreeMap;

#[test]
fn rejects_new_capture_live_nbt_write_and_aliased_host_api() {
    for source in [
        "fn write_nbt() {}",
        "fn mob_write_nbt() {}",
        "fn arbitrary_mutable_api(&self) -> Arc<dyn Inventory> { self.inventory.clone() }",
        "struct Entity { pub health: AtomicCell<f32>, }",
        "fn read_nbt() {}",
        "fn apply_nbt() {}",
        "fn tick() { entity.health.store(0); }",
        "use crate::entity::set_health as mutate; fn host() { mutate(0); }",
        "use crate::be::write_nbt as save; fn host() { save(); }",
        "use crate::be::write_nbt as inner; use inner as outer; fn host() { outer(); }",
        "use crate::world as w; use w::set_block_state as mutate; fn host() { mutate(); }",
        "chest_like_block_entity!(); furnace_like_block_entity!();",
    ] {
        let found = scan::sites("host_v0_2/entity.rs", source);
        assert!(!found.is_empty(), "unobserved fixture: {source}");
        let actual = BTreeMap::from([(found[0].clone(), "hash".into())]);
        assert!(compare(&actual, "")[0].contains("unmanifested"));
    }
}

#[test]
fn rejects_stale_row_and_changed_fingerprint() {
    let actual = BTreeMap::from([("current".into(), "new".into())]);
    let errors = compare(
        &actual,
        "gone|old|Legacy|unresolved\ncurrent|old|Legacy|unresolved\n",
    );
    assert!(errors.iter().any(|e| e.contains("stale")));
    assert!(errors.iter().any(|e| e.contains("changed")));
    let old = scan::sites("a.rs", "fn tick() { state.store(1); }");
    let new = scan::sites("a.rs", "fn tick() { state.store(2); }");
    assert_ne!(old, new);
    assert!(scan::sites("a.rs", "// fn write_nbt() {}\n \"state.store(1)\";").is_empty());
}

#[test]
fn stale_manifest_negative_control() {
    assert!(
        compare(&BTreeMap::new(), "gone|old|Legacy|unresolved\n")
            .iter()
            .any(|e| e.contains("stale"))
    );
}
#[test]
fn fingerprint_negative_control() {
    let actual = BTreeMap::from([("current".into(), "new".into())]);
    assert!(
        compare(&actual, "current|old|Legacy|unresolved\n")
            .iter()
            .any(|e| e.contains("changed"))
    );
}

#[test]
fn inventories_both_qualified_host_mutable_surfaces() {
    for path in [
        "crates/pumpkin-wasm-host-v0_1/src/player.rs",
        "crates/pumpkin-wasm-host-v0_2/src/player.rs",
    ] {
        let found = scan::sites(path, "impl Host { fn arbitrary_mutable_api(&mut self) {} }");
        assert!(!found.is_empty(), "missing host inventory: {path}");
        assert!(found[0].contains("arbitrary_mutable_api"));
    }
}

#[test]
fn aliases_and_capture_body_changes_have_their_own_records() {
    let alias = scan::sites(
        "a.rs",
        "use crate::be::write_nbt as inner; use inner as outer; fn host() { outer(); }",
    );
    assert!(alias.iter().any(|s| s.contains("::outer::")));
    let old = scan::sites(
        "a.rs",
        "fn write_nbt(value: [u8; 3]) { nbt.put_int(\"value\", 1); }",
    );
    let new = scan::sites(
        "a.rs",
        "fn write_nbt(value: [u8; 3]) { nbt.put_int(\"value\", 2); }",
    );
    assert_ne!(old, new);
}
