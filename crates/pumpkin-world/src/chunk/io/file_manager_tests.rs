use super::*;
use crate::{
    chunk::io::FileIO,
    chunk::{ChunkEntityData, format::anvil::AnvilChunkFile},
};
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::{Mutex, atomic::AtomicBool};

pub(super) fn folder(root: &Path) -> LevelFolder {
    LevelFolder {
        root_folder: root.into(),
        dim_folder: root.into(),
        region_folder: root.into(),
        entities_folder: root.into(),
        poi_folder: root.into(),
    }
}

fn entities(position: Vector2<i32>, name: &str) -> Arc<ChunkEntityData> {
    let mut entity = NbtCompound::new();
    entity.put_string("id", "minecraft:cow".into());
    entity.put_string("CustomName", name.into());
    Arc::new(ChunkEntityData {
        x: position.x,
        z: position.y,
        data: Mutex::new(vec![entity]),
        live: AtomicBool::new(false),
        dirty: crate::chunk::io::DirtyFlag::new(true),
    })
}

async fn load(
    manager: &ChunkFileManager<AnvilChunkFile<ChunkEntityData>>,
    folder: &LevelFolder,
    position: Vector2<i32>,
) -> Arc<ChunkEntityData> {
    let (send, mut receive) = mpsc::channel(1);
    manager.fetch_chunks(folder, &[position], send).await;
    match receive.recv().await.unwrap() {
        LoadedData::Loaded(data) => data,
        _ => panic!("Saved entities were lost"),
    }
}

#[tokio::test]
async fn unloaded_entities_survive_last_region_watcher_and_reload() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let a = Vector2::new(0, 0);
    let b = Vector2::new(1, 0);
    // Audit finding 1: both watched; A unloads and saves while B keeps the region watched.
    manager.watch_chunks(&folder, &[a, b]).await;
    manager.unwatch_chunks(&folder, &[a]).await;
    manager
        .save_chunks(&folder, vec![(a, entities(a, "new calf"))])
        .await
        .unwrap();
    // B's last watcher disappears; its later save must retain A's saved records.
    manager.unwatch_chunks(&folder, &[b]).await;
    manager
        .save_chunks(&folder, vec![(b, entities(b, "other cow"))])
        .await
        .unwrap();
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let loaded = load(&fresh, &folder, a).await;
    assert_eq!(
        loaded.data.lock().unwrap()[0].get_string("CustomName"),
        Some("new calf")
    );
}

#[tokio::test]
async fn failed_disk_write_is_retained_and_retried_before_eviction() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let a = Vector2::new(0, 0);
    let path = directory.path().join("r.0.0.mca");
    manager.watch_chunks(&folder, &[a]).await;
    drop(manager.get_serializer(&path).await.unwrap());
    // A directory at the destination makes publication fail after serialization.
    std::fs::create_dir(&path).unwrap();
    assert!(
        manager
            .save_chunks(&folder, vec![(a, entities(a, "retained calf"))])
            .await
            .is_err()
    );
    manager.unwatch_chunks(&folder, &[a]).await;
    assert!(manager.file_locks.read().await.contains_key(&path));
    std::fs::remove_dir(&path).unwrap();
    // Also retries unwatched failed regions at shutdown.
    manager.clear_watched_chunks().await;
    assert!(manager.file_locks.read().await.is_empty());
    let loaded = load(&manager, &folder, a).await;
    assert_eq!(
        loaded.data.lock().unwrap()[0].get_string("CustomName"),
        Some("retained calf")
    );
}

#[tokio::test]
async fn shutdown_retries_failed_publication_without_another_save_request() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let pos = Vector2::new(0, 0);
    let path = directory.path().join("r.0.0.mca");
    drop(manager.get_serializer(&path).await.unwrap());
    std::fs::create_dir(&path).unwrap();
    let chunk = entities(pos, "shutdown calf");
    assert!(
        manager
            .save_chunks(&folder, vec![(pos, chunk.clone())])
            .await
            .is_err()
    );
    assert!(chunk.is_dirty());
    assert!(manager.block_and_await_ongoing_tasks().await.is_err());
    drop(chunk); // Only the manager now owns the unloaded object.
    std::fs::remove_dir(&path).unwrap();
    manager.block_and_await_ongoing_tasks().await.unwrap();
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let loaded = load(&fresh, &folder, pos).await;
    assert_eq!(
        loaded.data.lock().unwrap()[0].get_string("CustomName"),
        Some("shutdown calf")
    );
}

#[tokio::test]
async fn pending_unload_is_loaded_before_disk_and_delayed_drain_cannot_overwrite_reload() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let pos = Vector2::new(0, 0);
    manager
        .save_chunks(&folder, vec![(pos, entities(pos, "old disk"))])
        .await
        .unwrap();
    // Level::clean_entity_chunks registers before removing the loaded entry or spawning its drain.
    manager.queue_chunks(&folder, vec![(pos, entities(pos, "unloaded snapshot"))]);
    let loaded = load(&manager, &folder, pos).await;
    assert_eq!(
        loaded.data.lock().unwrap()[0].get_string("CustomName"),
        Some("unloaded snapshot")
    );
    loaded.data.lock().unwrap()[0].put_string("CustomName", "new after reload".into());
    loaded.mark_dirty(true);
    manager
        .save_chunks(&folder, vec![(pos, loaded)])
        .await
        .unwrap();
    manager.block_and_await_ongoing_tasks().await.unwrap(); // Delayed original unload worker.
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    assert_eq!(
        load(&fresh, &folder, pos).await.data.lock().unwrap()[0].get_string("CustomName"),
        Some("new after reload")
    );
}

#[tokio::test]
async fn drain_publishes_work_registered_after_its_first_pass() {
    let directory = tempfile::tempdir().unwrap();
    let folder = Arc::new(folder(directory.path()));
    let manager = Arc::new(ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(
        AnvilChunkConfig::default(),
    ));
    let late = Vector2::new(32, 0); // A region absent from the first pass's path snapshot.
    let weak = Arc::downgrade(&manager);
    let late_folder = folder.clone();
    *manager.after_drain_pass.lock().unwrap() = Some(Box::new(move || {
        weak.upgrade()
            .unwrap()
            .queue_chunks(&late_folder, vec![(late, entities(late, "late calf"))]);
    }));
    manager.block_and_await_ongoing_tasks().await.unwrap();
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    assert_eq!(
        load(&fresh, &folder, late).await.data.lock().unwrap()[0].get_string("CustomName"),
        Some("late calf")
    );
}

#[tokio::test]
async fn activation_cannot_consume_a_pending_entity_save() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let pos = Vector2::new(0, 0);
    manager.queue_chunks(&folder, vec![(pos, entities(pos, "retained calf"))]);
    let loaded = load(&manager, &folder, pos).await;
    // The exact activation helper called by World::make_chunk_entities_live.
    assert_eq!(loaded.entities_for_activation().len(), 1);
    assert_eq!(loaded.data.lock().unwrap().len(), 1);
    assert!(loaded.entities_for_activation().is_empty());
    loaded.data.lock().unwrap().clear(); // Even another destructive consumer owns only its copy.
    manager.block_and_await_ongoing_tasks().await.unwrap();
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    assert_eq!(
        load(&fresh, &folder, pos).await.data.lock().unwrap()[0].get_string("CustomName"),
        Some("retained calf")
    );
}

#[tokio::test]
async fn drain_republishes_mutation_during_publication() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let pos = Vector2::new(0, 0);
    let chunk = entities(pos, "before");
    manager.queue_chunks(&folder, vec![(pos, chunk.clone())]);
    let mutated = chunk.clone();
    *manager.before_publish.lock().unwrap() = Some(Box::new(move || {
        mutated.data.lock().unwrap()[0].put_string("CustomName", "during publication".into());
        mutated.mark_dirty(true);
    }));
    manager.block_and_await_ongoing_tasks().await.unwrap();
    assert!(!chunk.is_dirty());
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    assert_eq!(
        load(&fresh, &folder, pos).await.data.lock().unwrap()[0].get_string("CustomName"),
        Some("during publication")
    );
}

#[tokio::test]
async fn failed_read_reopens_repaired_region() {
    let directory = tempfile::tempdir().unwrap();
    let folder = folder(directory.path());
    let manager =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    let pos = Vector2::new(0, 0);
    manager
        .save_chunks(&folder, vec![(pos, entities(pos, "repaired"))])
        .await
        .unwrap();
    let path = directory.path().join("r.0.0.mca");
    let saved = std::fs::read(&path).unwrap();
    let mut corrupt = saved.clone();
    corrupt[2 * 4096..2 * 4096 + 4].fill(0xff); // Invalid payload, not an ignored location.
    std::fs::write(&path, corrupt).unwrap();
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    fresh.watch_chunks(&folder, &[pos]).await;
    let (tx, mut rx) = mpsc::channel(1);
    fresh.fetch_chunks(&folder, &[pos], tx).await;
    assert!(matches!(rx.recv().await, Some(LoadedData::Error(_))));
    std::fs::write(&path, saved).unwrap();
    assert_eq!(
        load(&fresh, &folder, pos).await.data.lock().unwrap()[0].get_string("CustomName"),
        Some("repaired")
    );
}

#[tokio::test]
async fn cancelled_caller_cannot_release_an_ongoing_region_publication() {
    let directory = tempfile::tempdir().unwrap();
    let folder = Arc::new(folder(directory.path()));
    let manager = Arc::new(ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(
        AnvilChunkConfig::default(),
    ));
    let pos = Vector2::new(0, 0);
    let (started, start) = tokio::sync::oneshot::channel();
    let (resume, resumed) = tokio::sync::oneshot::channel();
    *manager.publication_pause.lock().unwrap() = Some((started, resumed));
    let saving = manager.clone();
    let save_folder = folder.clone();
    let caller = tokio::spawn(async move {
        saving
            .save_chunks(&save_folder, vec![(pos, entities(pos, "old publication"))])
            .await
    });
    start.await.unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let path = directory.path().join("r.0.0.mca");
    let serializer = manager.get_serializer(&path).await.unwrap();
    assert!(
        serializer.try_write().is_err(),
        "cancellation released the publication lock"
    );
    manager.queue_chunks(&folder, vec![(pos, entities(pos, "new publication"))]);
    resume.send(()).unwrap();
    manager.block_and_await_ongoing_tasks().await.unwrap();
    let fresh =
        ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(AnvilChunkConfig::default());
    assert_eq!(
        load(&fresh, &folder, pos).await.data.lock().unwrap()[0].get_string("CustomName"),
        Some("new publication")
    );
}
