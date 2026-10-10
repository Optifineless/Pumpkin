use super::*;
use pumpkin_nbt::compound::NbtCompound;

#[tokio::test]
async fn entity_cleanup_waits_for_an_admitted_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let chunk = Arc::new(ChunkEntityData {
        x: pos.x,
        z: pos.y,
        data: Mutex::new(Vec::new()),
        live: AtomicBool::new(false),
        dirty: crate::chunk::io::DirtyFlag::new(false),
    });
    level.loaded_entity_chunks.insert(pos, chunk.clone());
    let mutation = level.begin_chunk_mutation(pos);
    level.clean_entity_chunks([pos]);
    assert!(Arc::ptr_eq(
        &chunk,
        &level.get_entity_chunk_sync(&pos).unwrap()
    ));
    let mut nbt = NbtCompound::new();
    nbt.put_string("CustomName", "late write".into());
    chunk.data.lock().unwrap().push(nbt);
    chunk.mark_dirty(true);
    drop(mutation);
    level.clean_entity_chunks([pos]);
    assert!(level.get_entity_chunk_sync(&pos).is_none());
    level.drain_entity_storage().await.unwrap();
    let loaded = level.get_entity_chunk(pos).await.unwrap();
    assert_eq!(
        loaded.data.lock().unwrap()[0].get_string("CustomName"),
        Some("late write")
    );
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_fetch_releases_its_temporary_ticket() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(100, 100);
    let mut fetch = Box::pin(level.fetch_chunk(pos));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(fetch.as_mut(), &mut context).is_pending());
    assert!(
        level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .contains_key(&pos)
    );
    drop(fetch);
    assert!(
        !level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .contains_key(&pos)
    );
    assert_eq!(
        level.chunk_lifecycles.at(pos).lock().unwrap().mutations(),
        0
    );
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn tick_scheduling_waits_for_container_unregistration() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let block = BlockPos::new(1, 1, 1);
    level.loaded_chunks.insert(pos, ChunkData::empty_sync(0, 0));
    let registration = level.scheduled_tick_registration.lock().unwrap();
    let barrier = std::sync::Barrier::new(2);
    let (done, result) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let level = &level;
        let barrier = &barrier;
        scope.spawn(move || {
            barrier.wait();
            level.schedule_block_tick(&Block::STONE, block, 1, TickPriority::Normal);
            done.send(()).unwrap();
        });
        barrier.wait();
        // Simulate a drain paused after seeing an empty queue, before removing its registration.
        assert!(matches!(
            result.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        level.chunks_with_scheduled_ticks.remove(&pos);
        drop(registration);
        result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    });
    assert!(level.chunks_with_scheduled_ticks.contains(&pos));
    let mut active = FxHashSet::default();
    active.insert(pos);
    assert_eq!(level.get_tick_data(&active, 0).block_ticks.len(), 1);
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn mutation_admission_and_drop_do_not_wait_for_lifecycle_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    level.loaded_chunks.insert(pos, ChunkData::empty_sync(0, 0));
    let lifecycle = level.chunk_lifecycles.at(pos);
    let (sender, receiver) = std::sync::mpsc::channel();
    let admitted = std::thread::scope(|scope| {
        let state = lifecycle.lock().unwrap();
        let level = &level;
        let worker = scope.spawn(move || {
            let mutation = level.begin_chunk_mutation(pos);
            drop(mutation);
            sender.send(()).unwrap();
        });
        let admitted = receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_ok();
        drop(state);
        worker.join().unwrap();
        admitted
    });
    assert!(
        admitted,
        "a lifecycle inspection stalled the common mutation path"
    );
    level.shutdown().await.unwrap();
}
