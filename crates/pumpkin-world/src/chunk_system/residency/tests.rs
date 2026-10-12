#[expect(
    clippy::unwrap_used,
    reason = "Regression fixture construction and assertions"
)]
#[tokio::test]
async fn cancelled_chunk_fetch_releases_its_temporary_ticket() {
    use std::{future::Future, task::Poll};

    use crate::{
        chunk_system::{ChunkLoading, ChunkPos},
        level::Level,
    };

    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &pumpkin_config::world::LevelConfig::default(),
        directory.path().into(),
        0,
        pumpkin_data::dimension::Dimension::OVERWORLD,
    );
    let pos = ChunkPos::new(0, 0);
    // An unrelated owner already retains this position; cancellation must preserve it.
    let baseline = {
        let mut loading = level.chunk_loading.lock().unwrap();
        loading.add_ticket(pos, ChunkLoading::FULL_CHUNK_LEVEL);
        loading.ticket.clone()
    };
    let mut fetch = Box::pin(level.get_or_fetch_chunk(pos, |_| ()));
    // This current-thread runtime cannot run chunk I/O until we yield. Poll exactly once
    // to stop at the real request's await, then cancel without any timing assumption.
    std::future::poll_fn(|cx| {
        assert!(fetch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(level.chunk_loading.lock().unwrap().ticket[&pos].len(), 2);
    drop(fetch);
    let remaining = level.chunk_loading.lock().unwrap().ticket.clone();
    level
        .chunk_loading
        .lock()
        .unwrap()
        .remove_ticket(pos, ChunkLoading::FULL_CHUNK_LEVEL);
    level.shutdown().await.unwrap();
    assert_eq!(
        remaining, baseline,
        "cancelled chunk fetch leaked its ticket"
    );
}
