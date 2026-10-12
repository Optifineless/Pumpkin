use super::scan_source;

fn one(source: &str, kind: &str, enforced: bool) {
    let sites = scan_source("src/fixture.rs", source);
    assert_eq!(sites.len(), 1, "{source}");
    assert_eq!(sites[0].kind, kind, "{source}");
    assert_eq!(sites[0].enforced, enforced, "{source}");
}

#[test]
fn type_alias_constructor() {
    one(
        "type Lock<T> = std::sync::Mutex<T>; fn f() { Lock::new(1); }",
        "std::sync::Mutex::new",
        true,
    );
}
#[test]
fn chained_forward_type_alias() {
    one(
        "type Lock = Inner; type Inner = M<u8>; use std::sync::Mutex as M; fn f() { Lock::new(1); }",
        "std::sync::Mutex::new",
        true,
    );
}
#[test]
fn module_alias() {
    one(
        "use std::sync as sync; fn f() { sync::Mutex::new(1); }",
        "std::sync::Mutex::new",
        true,
    );
}
#[test]
fn imported_through_module_alias() {
    one(
        "use std::sync as sync; use sync::Mutex as Lock; fn f() { Lock::new(1); }",
        "std::sync::Mutex::new",
        true,
    );
}
#[test]
fn ambiguous_raw_and_instrumented_alias() {
    one(
        "type Lock = std::sync::Mutex<u8>; mod m { type Lock = pumpkin_util::sync::Mutex<u8>; } fn f() { Lock::new(1); }",
        "unknown::Mutex::new",
        true,
    );
}
#[test]
fn unresolved_named_default() {
    one(
        "fn f() { let lock: other::Mutex<u8> = Default::default(); }",
        "unknown::Default::default",
        true,
    );
}
#[test]
fn mutex_default() {
    one(
        "use std::sync::Mutex; fn f() { Mutex::default(); }",
        "std::sync::Mutex::default",
        true,
    );
}
#[test]
fn rwlock_default() {
    one(
        "use tokio::sync::RwLock as Lock; fn f() { Lock::default(); }",
        "tokio::sync::RwLock::default",
        true,
    );
}
#[test]
fn const_new() {
    one(
        "fn f() { tokio::sync::Mutex::const_new(1); }",
        "tokio::sync::Mutex::const_new",
        true,
    );
}
#[test]
fn const_mutex_macro() {
    one(
        "fn f() { parking_lot::const_mutex!(1); }",
        "parking_lot::Mutex::const_mutex!",
        true,
    );
}
#[test]
fn const_rwlock_macro_alias() {
    one(
        "use parking_lot::const_rwlock as lock; fn f() { lock!(1); }",
        "parking_lot::RwLock::const_rwlock!",
        true,
    );
}
#[test]
fn typed_default() {
    one(
        "use std::sync::Mutex; fn f() { let lock: Mutex<u8> = Default::default(); }",
        "std::sync::Mutex::default",
        true,
    );
}
#[test]
fn typed_alias_default() {
    one(
        "type Lock = std::sync::RwLock<u8>; fn f() { let lock: Lock = Default::default(); }",
        "std::sync::RwLock::default",
        true,
    );
}
#[test]
fn field_default() {
    one(
        "struct S { lock: std::sync::Mutex<u8> } fn f() { S { lock: Default::default() } }",
        "std::sync::Mutex::default",
        true,
    );
}
#[test]
fn unresolved_nested_default() {
    one(
        "fn f() { let lock: Arc<ExternalMutex<u8>> = Default::default(); }",
        "unknown::Default::default",
        true,
    );
}
#[test]
fn unrelated_default() {
    assert!(scan_source("src/a.rs", "fn f() { let n: u8 = Default::default(); }").is_empty());
}
#[test]
fn imported_block_on() {
    one(
        "use futures::executor::block_on; fn f() { block_on(async {}); }",
        "futures::executor::block_on",
        true,
    );
}
#[test]
fn aliased_block_on() {
    one(
        "use futures::executor::block_on as wait; fn f() { wait(async {}); }",
        "futures::executor::block_on",
        true,
    );
}
#[test]
fn aliased_block_in_place() {
    one(
        "use tokio::task::block_in_place as bip; fn f() { bip(|| {}); }",
        "block_in_place",
        true,
    );
}
#[test]
fn handle_ufcs() {
    one(
        "use tokio::runtime::Handle; fn f() { Handle::block_on(&h, async {}); }",
        "block_on",
        true,
    );
}
#[test]
fn runtime_ufcs() {
    one(
        "use tokio::runtime::Runtime as Rt; fn f() { Rt::block_on(&rt, async {}); }",
        "block_on",
        true,
    );
}
#[test]
fn instrumented_mutex() {
    one(
        "fn f() { pumpkin_util::sync::Mutex::new(1); }",
        "pumpkin_util::sync::Mutex::new",
        false,
    );
}
#[test]
fn instrumented_rwlock_alias() {
    one(
        "use pumpkin_util::sync::RwLock as Lock; fn f() { Lock::new(1); }",
        "pumpkin_util::sync::RwLock::new",
        false,
    );
}
#[test]
fn screen_mutex() {
    one("fn f() { ScreenMutex::new(1); }", "ScreenMutex::new", false);
}
#[test]
fn negative_string_constructor() {
    assert!(scan_source("src/a.rs", "fn f() { let s = \"Mutex::new(1)\"; }").is_empty());
}
#[test]
fn negative_comment_bridge() {
    assert!(scan_source("src/a.rs", "fn f() { /* block_on(1) */ } // block_on(2)").is_empty());
}
