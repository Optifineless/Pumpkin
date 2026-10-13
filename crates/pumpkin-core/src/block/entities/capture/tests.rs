use super::*;
use pumpkin_util::{
    math::position::BlockPos,
    sync::{Mutex, ScreenSelection},
};
use std::sync::Arc;

pub type CaptureAction = Arc<dyn Fn(&mut NbtCompound) + Send + Sync>;
pub struct TestCapture(pub CaptureAction);
impl BlockEntity for TestCapture {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        (self.0)(nbt);
    }
    fn resource_location(&self) -> &'static str {
        "minecraft:comparator"
    }
    fn get_position(&self) -> BlockPos {
        BlockPos::new(8, 64, 8)
    }
    fn from_nbt(_: &NbtCompound, _: BlockPos) -> Self {
        Self(Arc::new(|_| {}))
    }
}

pub fn rejected(operation: impl FnOnce(), boundary: &str) {
    let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).unwrap_err();
    let message = error
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        message.contains(boundary),
        "expected {boundary}, got {message}"
    );
}

#[test]
#[cfg(debug_assertions)]
fn snapshot_rejects_real_instrumented_guard_before_serialization() {
    let lock = Mutex::<_, ScreenSelection>::new(());
    let capture = TestCapture(Arc::new(|_| panic!("serialization crossed guarded entry")));
    let guard = lock.lock().unwrap();
    rejected(
        || {
            let _ = capture_legacy(&capture);
        },
        "BlockEntity::capture_legacy",
    );
    drop(guard);
}

#[test]
fn snapshot_envelope_and_status_stay_explicitly_legacy() {
    let normal = TestCapture(Arc::new(|nbt| nbt.put_int("OutputSignal", 7)));
    let capture = capture_legacy(&normal);
    assert_eq!(capture.status(), CaptureStatus::BoundaryCheckedLegacy);
    let nbt = capture.into_nbt();
    assert_eq!(nbt.get_string("id"), Some("minecraft:comparator"));
    assert_eq!(
        (nbt.get_int("x"), nbt.get_int("y"), nbt.get_int("z")),
        (Some(8), Some(64), Some(8))
    );
    assert_eq!(nbt.get_int("OutputSignal"), Some(7));
}
