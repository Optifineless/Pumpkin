use super::*;

#[test]
fn scheduler_retirement_empty_keeps_wheel_unallocated() {
    let clock = Arc::new(AtomicI64::new(100));
    let scheduler = ChunkTickScheduler::<&u8>::default();
    scheduler.bind_clock(clock.clone());
    scheduler.freeze_for_unload(100);
    let guard = scheduler.inner.lock().unwrap();
    assert!(guard.inner.is_none(), "retirement allocated the tick wheel");
    assert_eq!(guard.frozen_time, Some(100));
    assert!(std::mem::size_of_val(&*guard) <= 4 * std::mem::size_of::<usize>());
    drop(guard);
    assert!(scheduler.to_vec().is_empty());
    assert!(scheduler.prepare_tick(200).is_none());
    let mut due = Vec::new();
    assert!(scheduler.poll_due_batch(1, None, &mut due).is_none());
    assert!(due.is_empty());
    scheduler.freeze_for_unload(200);
    clock.store(101, Ordering::Relaxed);
    let tick = ScheduledTick {
        delay: 10,
        priority: TickPriority::Normal,
        position: BlockPos::new(0, 64, 0),
        value: &0u8,
    };
    scheduler.schedule_tick(&tick, 0);
    scheduler.schedule_tick(&tick, 1); // Deduplication also applies to the frozen image.
    assert_eq!(scheduler.to_vec().len(), 1);
    assert_eq!(scheduler.to_vec()[0].delay, 11);
    assert!(!scheduler.has_due_ticks(200));
    scheduler.clear_area(&tick.position, &BlockPos::new(1, 65, 1));
    let guard = scheduler.inner.lock().unwrap();
    assert!(guard.inner.is_none());
    assert_eq!(guard.frozen_time, Some(100));
}
