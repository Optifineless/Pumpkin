use super::*;
use crate::chunk::{
    ChunkSerializingError,
    format::anvil::{AnvilChunkFile, SingleChunkDataSerializer},
};
use bytes::Bytes;
use std::sync::atomic::{AtomicBool, Ordering};

struct Chunk {
    versioned: bool,
    pos: Vector2<i32>,
    fail: AtomicBool,
    dirty: crate::chunk::io::DirtyFlag,
}

impl Dirtiable for Chunk {
    fn dirty_version(&self) -> Option<u64> {
        self.versioned.then(|| self.dirty.version())
    }
    fn clear_published(&self, version: u64) {
        self.dirty.clear_published(version);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
    fn mark_dirty(&self, flag: bool) {
        self.dirty.store(flag, Ordering::Relaxed);
    }
}

impl PathFromLevelFolder for Chunk {
    fn file_path(folder: &LevelFolder, name: &str) -> PathBuf {
        folder.region_folder.join(name)
    }
}

impl SingleChunkDataSerializer for Chunk {
    fn position(&self) -> (i32, i32) {
        (self.pos.x, self.pos.y)
    }
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        if self.fail.load(Ordering::Relaxed) {
            Err(ChunkSerializingError::ErrorSerializingChunk(
                pumpkin_nbt::Error::NoRootCompound(0),
            ))
        } else {
            Ok(Bytes::from_static(b"retained"))
        }
    }
    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        assert_eq!(bytes.as_ref(), b"retained");
        Ok(Self {
            pos,
            versioned: true,
            fail: AtomicBool::new(false),
            dirty: crate::chunk::io::DirtyFlag::new(false),
        })
    }
}

#[tokio::test]
async fn failed_serializer_retains_original_and_does_not_drop_remaining_batch() {
    let directory = tempfile::tempdir().unwrap();
    let folder = super::tests::folder(directory.path());
    let manager = ChunkFileManager::<AnvilChunkFile<Chunk>>::new(
        pumpkin_config::chunk::AnvilChunkConfig::default(),
    );
    let chunk = |x, fail| {
        Arc::new(Chunk {
            pos: Vector2::new(x, 0),
            versioned: true,
            fail: AtomicBool::new(fail),
            dirty: crate::chunk::io::DirtyFlag::new(true),
        })
    };
    let a = chunk(0, true);
    let b = chunk(1, false);
    let weak = Arc::downgrade(&a);
    assert!(
        manager
            .save_chunks(&folder, vec![(a.pos, a.clone()), (b.pos, b)])
            .await
            .is_err()
    );
    drop(a);
    let a = weak
        .upgrade()
        .expect("unloaded object must survive serialization failure");
    assert!(a.is_dirty());
    // A failed chunk does not prevent B's publication.
    let fresh = ChunkFileManager::<AnvilChunkFile<Chunk>>::new(
        pumpkin_config::chunk::AnvilChunkConfig::default(),
    );
    let (tx, mut rx) = mpsc::channel(1);
    fresh.fetch_chunks(&folder, &[Vector2::new(1, 0)], tx).await;
    assert!(matches!(rx.recv().await, Some(LoadedData::Loaded(_))));
    a.fail.store(false, Ordering::Relaxed);
    drop(a);
    manager.block_and_await_ongoing_tasks().await.unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    fresh.fetch_chunks(&folder, &[Vector2::new(0, 0)], tx).await;
    assert!(matches!(rx.recv().await, Some(LoadedData::Loaded(_))));
}

#[tokio::test]
async fn unversioned_publication_retains_the_original_and_reports_error() {
    let directory = tempfile::tempdir().unwrap();
    let folder = super::tests::folder(directory.path());
    let manager = ChunkFileManager::<AnvilChunkFile<Chunk>>::new(
        pumpkin_config::chunk::AnvilChunkConfig::default(),
    );
    let chunk = Arc::new(Chunk {
        pos: Vector2::new(0, 0),
        versioned: false,
        fail: AtomicBool::new(false),
        dirty: crate::chunk::io::DirtyFlag::new(true),
    });
    let weak = Arc::downgrade(&chunk);
    manager.queue_chunks(&folder, vec![(chunk.pos, chunk)]);
    assert!(manager.block_and_await_ongoing_tasks().await.is_err());
    assert!(weak.upgrade().is_some());
}
