use super::*;
use pumpkin_nbt::compound::NbtCompound;

fn snapshot(pos: Vector2<i32>, name: &str) -> SyncEntityChunk {
    let mut nbt = NbtCompound::new();
    nbt.put_string("CustomName", name.into());
    Arc::new(ChunkEntityData {
        x: pos.x,
        z: pos.y,
        data: Mutex::new(vec![nbt]),
        dormant_records: std::sync::Mutex::new(None),
        live: AtomicBool::new(false),
        dirty: crate::chunk::io::DirtyFlag::new(true),
    })
}

#[tokio::test]
async fn stale_entity_read_cannot_install_after_unload() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    level
        .entity_saver
        .save_chunks(&level.level_folder, vec![(pos, snapshot(pos, "old disk"))])
        .await
        .unwrap();
    let weak = Arc::downgrade(&level);
    *level.after_entity_read.lock().unwrap() = Some(Box::new(move || {
        let level = weak.upgrade().unwrap();
        // Another lifecycle has changed and unloaded the coordinate while this disk result waited.
        level
            .loaded_entity_chunks
            .insert(pos, snapshot(pos, "new unload"));
        level.clean_entity_chunk(&pos);
    }));
    let loaded = level.get_entity_chunk(pos).await.unwrap();
    assert_eq!(
        loaded.entities_for_activation()[0].get_string("CustomName"),
        Some("new unload")
    );
    let mut already_live = level.receive_entity_chunks(vec![pos]);
    assert!(!already_live.recv().await.unwrap().1);
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn entity_generation_is_tracked_until_its_result_is_delivered() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let (tx, rx) = oneshot::channel();
    level.pending_entity_generations.insert(pos, vec![tx]);
    // Hold result delivery so shutdown can observe the outstanding Rayon producer.
    let delivery = level.pending_entity_generations.get_mut(&pos).unwrap();
    level.spawn_entity_generation(pos);
    assert!(!level.tasks.is_empty());
    drop(delivery);
    level.shutdown().await.unwrap();
    assert_eq!(rx.await.unwrap().x, pos.x);
}

#[tokio::test]
async fn storage_review_entity_reads_deduplicate_only_the_same_position() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let neighbor = Vector2::new(1, 0);
    level.entity_saver.queue_chunks(
        &level.level_folder,
        vec![
            (pos, snapshot(pos, "first")),
            (neighbor, snapshot(neighbor, "second")),
        ],
    );
    let (release, paused) = oneshot::channel();
    *level.before_entity_read.lock().unwrap() = Some(paused);
    let mut first = Box::pin(level.get_entity_chunk(pos));
    let mut duplicate = Box::pin(level.get_entity_chunk(pos));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(first.as_mut(), &mut context).is_pending());
    assert!(std::future::Future::poll(duplicate.as_mut(), &mut context).is_pending());
    let mut independent = Box::pin(level.get_entity_chunk(neighbor));
    assert!(matches!(
        std::future::Future::poll(independent.as_mut(), &mut context),
        std::task::Poll::Ready(Ok(_))
    ));
    assert_eq!(level.entity_read_count.load(Ordering::Relaxed), 2);
    release.send(()).unwrap();
    let (first, duplicate) = tokio::join!(first, duplicate);
    assert!(Arc::ptr_eq(&first.unwrap(), &duplicate.unwrap()));
    assert_eq!(level.entity_read_count.load(Ordering::Relaxed), 2);
    assert!(level.entity_loads.lock().unwrap().is_empty());
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_review_entity_read_cancellation_releases_last_waiter() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    level
        .entity_saver
        .queue_chunks(&level.level_folder, vec![(pos, snapshot(pos, "pending"))]);
    let (release, paused) = oneshot::channel();
    *level.before_entity_read.lock().unwrap() = Some(paused);
    let mut first = Box::pin(level.get_entity_chunk(pos));
    let mut waiter = Box::pin(level.get_entity_chunk(pos));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(first.as_mut(), &mut context).is_pending());
    assert!(std::future::Future::poll(waiter.as_mut(), &mut context).is_pending());
    drop(first);
    assert_eq!(level.entity_loads.lock().unwrap().len(), 1);
    drop(waiter);
    assert!(level.entity_loads.lock().unwrap().is_empty());
    assert!(release.send(()).is_err());
    assert!(!level.loaded_entity_chunks.contains_key(&pos));
    level.get_entity_chunk(pos).await.unwrap();
    assert!(level.entity_loads.lock().unwrap().is_empty());
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_review_cancelled_reader_keeps_waiters_and_unload_invalidates_ready_reads() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    level
        .entity_saver
        .queue_chunks(&level.level_folder, vec![(pos, snapshot(pos, "old"))]);
    let (release, paused) = oneshot::channel();
    *level.before_entity_read.lock().unwrap() = Some(paused);
    let mut first = Box::pin(level.get_entity_chunk(pos));
    let mut waiter = Box::pin(level.get_entity_chunk(pos));
    let mut later_waiter = Box::pin(level.get_entity_chunk(pos));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(first.as_mut(), &mut context).is_pending());
    assert!(std::future::Future::poll(waiter.as_mut(), &mut context).is_pending());
    assert!(std::future::Future::poll(later_waiter.as_mut(), &mut context).is_pending());
    drop(first);
    release.send(()).unwrap();
    waiter.await.unwrap();
    assert_eq!(level.entity_read_count.load(Ordering::Relaxed), 1);
    level
        .loaded_entity_chunks
        .insert(pos, snapshot(pos, "new unload"));
    level.clean_entity_chunk(&pos);
    let loaded = later_waiter.await.unwrap();
    assert_eq!(
        loaded.entities_for_activation()[0].get_string("CustomName"),
        Some("new unload")
    );
    assert!(level.entity_loads.lock().unwrap().is_empty());
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_review_failed_entity_read_is_shared_and_releases_state() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let path = level.level_folder.entities_folder.join("r.0.0.mca");
    std::fs::create_dir(&path).unwrap();
    let (release, paused) = oneshot::channel();
    *level.before_entity_read.lock().unwrap() = Some(paused);
    let mut first = Box::pin(level.get_entity_chunk(pos));
    let mut waiter = Box::pin(level.get_entity_chunk(pos));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(first.as_mut(), &mut context).is_pending());
    assert!(std::future::Future::poll(waiter.as_mut(), &mut context).is_pending());
    release.send(()).unwrap();
    let (first, waiter) = tokio::join!(first, waiter);
    assert!(first.is_err());
    assert!(waiter.is_err());
    assert_eq!(level.entity_read_count.load(Ordering::Relaxed), 1);
    assert!(level.entity_loads.lock().unwrap().is_empty());
    assert!(!level.loaded_entity_chunks.contains_key(&pos));
    std::fs::remove_dir(path).unwrap();
    level.get_entity_chunk(pos).await.unwrap();
    assert!(level.entity_loads.lock().unwrap().is_empty());
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_review_unrelated_unload_does_not_restart_entity_read() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let neighbor = Vector2::new(1, 0);
    level
        .entity_saver
        .queue_chunks(&level.level_folder, vec![(pos, snapshot(pos, "reading"))]);
    level
        .loaded_entity_chunks
        .insert(neighbor, snapshot(neighbor, "unloading"));
    let restarted = Arc::new(AtomicBool::new(false));
    let observed = restarted.clone();
    let weak = Arc::downgrade(&level);
    *level.after_entity_read.lock().unwrap() = Some(Box::new(move || {
        let level = weak.upgrade().unwrap();
        level.clean_entity_chunk(&neighbor);
        *level.after_entity_read.lock().unwrap() = Some(Box::new(move || {
            observed.store(true, Ordering::Relaxed);
        }));
    }));
    level.get_entity_chunk(pos).await.unwrap();
    level.shutdown().await.unwrap();
    assert!(!restarted.load(Ordering::Relaxed));
}

#[tokio::test]
async fn storage_review_entity_unload_flushes_only_its_regions() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let other = Vector2::new(64, 0);
    let unloaded = snapshot(pos, "unloaded");
    let unrelated = snapshot(other, "unrelated pending save");
    level
        .entity_saver
        .queue_chunks(&level.level_folder, vec![(other, unrelated.clone())]);
    level.loaded_entity_chunks.insert(pos, unloaded.clone());
    level.clean_entity_chunk(&pos);
    level.tasks.close();
    level.tasks.wait().await;
    let unrelated_still_pending = unrelated.is_dirty();
    assert!(!unloaded.is_dirty());
    // The save-all integration can still request a global barrier explicitly.
    level.drain_entity_storage().await.unwrap();
    assert!(!unrelated.is_dirty());
    unrelated.mark_dirty(true);
    level
        .entity_saver
        .queue_chunks(&level.level_folder, vec![(other, unrelated.clone())]);
    level.shutdown().await.unwrap();
    assert!(unrelated_still_pending);
    assert!(
        !unrelated.is_dirty(),
        "shutdown must still drain all regions"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_review_concurrent_requesters_release_the_entry() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    level
        .entity_saver
        .queue_chunks(&level.level_folder, vec![(pos, snapshot(pos, "shared"))]);
    for _ in 0..50 {
        // Two requesters finish on different worker threads; the count under the lock must
        // retire the entry regardless of which one observes the other's drop first.
        let first = tokio::spawn({
            let level = level.clone();
            async move { level.get_entity_chunk(pos).await.map(|_| ()) }
        });
        let second = tokio::spawn({
            let level = level.clone();
            async move { level.get_entity_chunk(pos).await.map(|_| ()) }
        });
        let (first, second) = tokio::join!(first, second);
        first.unwrap().unwrap();
        second.unwrap().unwrap();
        assert!(level.entity_loads.lock().unwrap().is_empty());
        level.loaded_entity_chunks.remove(&pos);
    }
    level.shutdown().await.unwrap();
}
