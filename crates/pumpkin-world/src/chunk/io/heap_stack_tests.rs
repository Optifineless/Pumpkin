#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "regression test assertions"
)]

use super::*;
use std::sync::atomic::AtomicBool;

fn folder(path: &Path) -> LevelFolder {
    LevelFolder {
        root_folder: path.into(),
        dim_folder: path.into(),
        region_folder: path.into(),
        entities_folder: path.into(),
        poi_folder: path.into(),
    }
}

#[test]
fn anvil_entity_save_and_reload_fit_on_a_one_mib_stack() {
    // Use an explicit 1 MiB stack so libtest's larger stack cannot mask
    // overflowing region-loader futures on Windows.
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    use crate::chunk::{ChunkEntityData, format::anvil::AnvilChunkFile};
                    use pumpkin_config::chunk::AnvilChunkConfig;
                    use pumpkin_nbt::compound::NbtCompound;

                    let directory = tempfile::tempdir().unwrap();
                    let folder = folder(directory.path());
                    let manager = ChunkFileManager::<AnvilChunkFile<ChunkEntityData>>::new(
                        AnvilChunkConfig::default(),
                    );
                    let position = Vector2::new(0, 0);
                    let mut entity = NbtCompound::new();
                    entity.put_string("id", "minecraft:armor_stand".into());
                    let chunk = Arc::new(ChunkEntityData {
                        x: 0,
                        z: 0,
                        data: std::sync::Mutex::new(vec![entity.clone()]),
                        dormant_records: std::sync::Mutex::new(None),
                        live: AtomicBool::new(false),
                        dirty: crate::chunk::io::DirtyFlag::new(true),
                    });
                    manager
                        .save_chunks(&folder, vec![(position, chunk.clone())])
                        .await
                        .unwrap();
                    // The storage merge retains serializers after save; explicitly evict
                    // before exercising the region loader on this constrained stack.
                    manager.clear_watched_chunks().await;
                    assert!(manager.file_locks.read().await.is_empty());
                    manager
                        .save_chunks(&folder, vec![(position, chunk)])
                        .await
                        .unwrap();
                    let (send, mut recv) = mpsc::channel(1);
                    manager.fetch_chunks(&folder, &[position], send).await;
                    let Some(LoadedData::Loaded(reloaded)) = recv.recv().await else {
                        panic!("expected saved entity chunk");
                    };
                    assert_eq!(*reloaded.data.lock().unwrap(), vec![entity]);
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
