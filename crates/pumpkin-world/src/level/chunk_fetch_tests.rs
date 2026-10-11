use super::*;
use std::time::Duration;

#[tokio::test]
async fn publication_before_listener_registration_completes_fetch() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let mut preload =
        crate::chunk_system::residency::ChunkResidency::new(level.chunk_loading.clone());
    preload.add(pos);
    let expected = ChunkData::empty_sync(pos.x, pos.y);
    let published = expected.clone();
    let weak = Arc::downgrade(&level);
    *level.before_chunk_listener.lock().unwrap() = Some(Box::new(move || {
        let level = weak.upgrade().unwrap();
        assert_eq!(level.chunk_listener.single_listener_count(), 0);
        // GenerationSchedule.receive_chunk publishes, then notifies: exactly the lost window.
        level.loaded_chunks.insert(pos, published.clone());
        level.chunk_listener.process_new_chunk(pos, &published);
    }));
    let fetched = tokio::time::timeout(
        Duration::from_secs(60),
        level.get_or_fetch_chunk(pos, Arc::clone),
    )
    .await;
    assert_eq!(level.chunk_listener.single_listener_count(), 0);
    drop(preload);
    assert!(level.chunk_loading.lock().unwrap().ticket.is_empty());
    level.shutdown().await.unwrap();
    assert!(Arc::ptr_eq(
        &fetched.expect("lost chunk-ready wakeup").unwrap(),
        &expected
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_requests_racing_shutdown_and_after_retirement_complete() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let requester = level.clone();
    let (ready, started) = tokio::sync::oneshot::channel();
    let requests = tokio::spawn(async move {
        let first = requester.level_channel.request_unload(&requester);
        ready.send(()).unwrap();
        let _ = first.await;
        for _ in 0..64 {
            let _ = requester.level_channel.request_unload(&requester).await;
        }
    });
    started.await.unwrap();
    level.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(60), requests)
        .await
        .unwrap()
        .unwrap();
    let retired = level.level_channel.request_unload(&level);
    assert!(
        tokio::time::timeout(Duration::from_secs(60), retired)
            .await
            .unwrap()
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_fetch_removes_listener_without_publication() {
    use std::{future::Future, task::Poll};

    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let mut fetch = Box::pin(level.get_or_fetch_chunk(Vector2::new(0, 0), |_| ()));
    std::future::poll_fn(|cx| {
        assert!(fetch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(level.chunk_listener.single_listener_count(), 1);
    drop(fetch);
    assert_eq!(level.chunk_listener.single_listener_count(), 0);
    assert!(level.chunk_loading.lock().unwrap().ticket.is_empty());
    level.shutdown().await.unwrap();
}
