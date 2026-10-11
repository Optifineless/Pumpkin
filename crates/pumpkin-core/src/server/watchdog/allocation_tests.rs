use super::allocation_counter::{ALLOCATIONS, COUNT_ALLOCATIONS};
use super::*;

// Run fatal arming in a child: an armed halt must never be cancelled by guard drop.
#[test]
fn detection_through_fallback_arming_does_not_allocate() {
    const CHILD: &str = "PUMPKIN_WATCHDOG_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "detection_through_fallback_arming_does_not_allocate",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let halt = HaltGuard::start().unwrap();
    let mut state = Watchdog::new(20);
    state.epoch = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    state.enabled.store(true, Ordering::Relaxed);
    state.tick_start.store(ACTIVE_TICK, Ordering::Relaxed);
    // A fresh reader thread, with no ArcSwap/TLS warm-up, exercises the exact
    // detector branch used by run, including waking the real parked halt helper.
    std::thread::spawn(move || {
        ALLOCATIONS.store(0, Ordering::Relaxed);
        COUNT_ALLOCATIONS.set(true);
        let detected = state.detect_and_arm(&halt);
        COUNT_ALLOCATIONS.set(false);
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        assert!(detected.is_some());
        assert_eq!(
            allocations, 0,
            "Detection through fallback arming allocated"
        );
        std::process::exit(0);
    })
    .join()
    .unwrap();
}

#[test]
fn bounded_report_does_not_allocate_or_split_unicode() {
    use std::fmt::Write;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let state = Watchdog::new(60_000);
    state.enabled.store(true, Ordering::Relaxed);
    state.slots.register();
    let record = Arc::new(Record::new());
    state.worlds.store(Arc::new(vec![WorldStats {
        id: uuid::Uuid::from_u128(123),
        name: "end".into(),
        dimension: "minecraft:the_end",
        players: AtomicUsize::new(3),
        chunks: AtomicUsize::new(17),
        phase: record.clone(),
    }]));
    let marker = state.slots.scope(0, Some(record));
    let hopper = pumpkin_data::block_properties::BLOCK_ENTITY_TYPES
        .iter()
        .position(|name| *name == "hopper")
        .unwrap() as u16;
    let _subject = marker.block_entity(
        pumpkin_util::math::position::BlockPos::new(-30, 64, 20),
        hopper,
    );
    drop(state.worlds.load());
    state.slots.warm_reader();
    let mut report = diagnostics::ReportBuffer::new();
    let mut overflow = diagnostics::ReportBuffer::new();
    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.set(true);
    state.report(&runtime.metrics(), 0, &mut report);
    // Exercise the overflow branch with multibyte input, rather than merely
    // proving that a small report happens to fit the initial capacity.
    for _ in 0..40_000 {
        let _ = overflow.write_str("界");
    }
    overflow.finish();
    COUNT_ALLOCATIONS.set(false);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    assert!(
        report
            .as_str()
            .contains("[00000000-0000-0000-0000-00000000007b]")
    );
    assert!(
        report
            .as_str()
            .contains("minecraft:hopper at (-30, 64, 20)")
    );
    assert!(overflow.as_str().len() <= 64 * 1024);
    assert!(
        overflow
            .as_str()
            .ends_with("Watchdog report truncated at 64 KiB.\n")
    );
}
