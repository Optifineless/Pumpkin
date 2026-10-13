#[cfg(debug_assertions)]
use super::*;
#[cfg(debug_assertions)]
extern crate self as pumpkin_util;

#[test]
#[cfg(debug_assertions)]
fn snapshot_checks_only_registered_own_locks_and_restores_on_unwind() {
    let own = pumpkin_util::sync::Mutex::<_, ScreenSelection>::new(());
    let foreign = pumpkin_util::sync::Mutex::<_, Cursor>::new(());
    let identity = CaptureIdentity {
        kind: "audited test adapter",
        instance: 1,
        position: Some([0; 3]),
    };
    let descriptor =
        OwnLock::new::<ScreenSelection>(std::ptr::from_ref(&own).cast::<()>() as usize);
    let error = std::panic::catch_unwind(|| {
        let _scope = SnapshotScope::with_own_locks(identity, &[descriptor]);
        let _guard = own.lock().unwrap();
        let _foreign = foreign.try_lock();
    })
    .unwrap_err();
    assert!(message(error.as_ref()).contains("snapshot foreign lock"));
    assert_no_guards_held("after snapshot unwind");
    assert_not_in_snapshot(SnapshotBoundary::WorldRead, "ordinary lookup after unwind");
    let _foreign = foreign.lock().unwrap();
}

#[test]
#[cfg(debug_assertions)]
fn snapshot_rejects_nested_capture_and_bounded_lock_descriptor() {
    let identity = CaptureIdentity {
        kind: "test capture",
        instance: 1,
        position: Some([0; 3]),
    };
    let descriptor = OwnLock::new::<ScreenSelection>(1);
    let error = std::panic::catch_unwind(|| {
        let _scope = SnapshotScope::with_own_locks(identity, &[descriptor; 9]);
    })
    .unwrap_err();
    assert!(message(error.as_ref()).contains("exceeds 8 entries"));
    let error = std::panic::catch_unwind(|| {
        let _scope = SnapshotScope::legacy(identity);
        let _nested = SnapshotScope::legacy(identity);
    })
    .unwrap_err();
    assert!(message(error.as_ref()).contains("SnapshotScope::enter"));
    assert_not_in_snapshot(SnapshotBoundary::Capture, "after nested unwind");
}

#[test]
#[cfg(not(debug_assertions))]
fn release_scope_and_boundaries_are_inert() {
    use super::*;
    let _scope = SnapshotScope::legacy(CaptureIdentity {
        kind: "release",
        instance: 0,
        position: Some([0; 3]),
    });
    assert_not_in_snapshot(SnapshotBoundary::Mutation, "release boundary");
}

#[cfg(debug_assertions)]
fn message(error: &(dyn std::any::Any + Send)) -> &str {
    error
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied())
        .unwrap_or("")
}
