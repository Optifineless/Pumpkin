use super::*;
use crate::{
    chunk::{
        ChunkData, ChunkSerializingError,
        format::anvil::{AnvilChunkFile, SingleChunkDataSerializer},
    },
    level::Level,
    tick::TickPriority,
};
use bytes::Bytes;
use pumpkin_config::{chunk::AnvilChunkConfig, world::LevelConfig};
use pumpkin_data::{Block, dimension::Dimension, fluid::Fluid};
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::Ordering;

const POSITION: BlockPos = BlockPos::new(0, 64, 0);
const CHUNK: Vector2<i32> = Vector2::new(0, 0);

struct CheckedCopy {
    chunk: Arc<ChunkData>,
    runtime_thread: std::thread::ThreadId,
}

impl Dirtiable for CheckedCopy {
    fn copy_for_load(
        self: &Arc<Self>,
        dimension: &Dimension,
    ) -> Result<Arc<Self>, ChunkReadingError> {
        // Exercise the real detach/decode at the Dirtiable boundary, without a pause hook.
        assert_ne!(
            std::thread::current().id(),
            self.runtime_thread,
            "pending detach/decode ran on the Tokio runtime worker"
        );
        Ok(Arc::new(Self {
            chunk: self.chunk.copy_for_load(dimension)?,
            runtime_thread: self.runtime_thread,
        }))
    }

    fn is_dirty(&self) -> bool {
        self.chunk.is_dirty()
    }

    fn mark_dirty(&self, dirty: bool) {
        self.chunk.mark_dirty(dirty);
    }
}

impl PathFromLevelFolder for CheckedCopy {
    fn file_path(folder: &LevelFolder, name: &str) -> PathBuf {
        folder.region_folder.join(name)
    }
}

impl SingleChunkDataSerializer for CheckedCopy {
    fn position(&self) -> (i32, i32) {
        self.chunk.position()
    }

    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        self.chunk.to_bytes()
    }

    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        Ok(Self {
            chunk: Arc::new(ChunkData::from_bytes(bytes, pos)?),
            runtime_thread: std::thread::current().id(),
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pending_snapshot_batch_copies_on_blocking_threads_with_fresh_schedulers() {
    let directory = tempfile::tempdir().unwrap();
    let folder = super::tests::folder(directory.path());
    let manager = ChunkFileManager::<AnvilChunkFile<CheckedCopy>>::new(AnvilChunkConfig::default());
    let positions: Vec<_> = (0..4).map(|x| Vector2::new(x, 0)).collect();
    let originals: Vec<_> = positions
        .iter()
        .map(|pos| {
            let chunk = ChunkData::empty_sync(pos.x, pos.y);
            let position = BlockPos::new(pos.x * 16, 64, 0);
            chunk.block_ticks.schedule_tick(
                &crate::tick::ScheduledTick {
                    delay: 5000,
                    priority: TickPriority::Normal,
                    position,
                    value: &Block::STONE,
                },
                0,
            );
            chunk.fluid_ticks.schedule_tick(
                &crate::tick::ScheduledTick {
                    delay: 5000,
                    priority: TickPriority::Normal,
                    position,
                    value: &Fluid::WATER,
                },
                0,
            );
            chunk.block_ticks.freeze_for_unload(100);
            chunk.fluid_ticks.freeze_for_unload(100);
            Arc::new(CheckedCopy {
                chunk,
                runtime_thread: std::thread::current().id(),
            })
        })
        .collect();
    manager.queue_chunks(
        &folder,
        positions.iter().copied().zip(originals.clone()).collect(),
    );
    let (send, mut receive) = mpsc::channel(positions.len());
    manager.fetch_chunks(&folder, &positions, send).await;
    for original in &originals {
        let Some(LoadedData::Loaded(copy)) = receive.recv().await else {
            panic!("pending chunk was not copied");
        };
        assert!(!Arc::ptr_eq(&original.chunk, &copy.chunk));
        assert!(copy.chunk.block_ticks.step_tick(300).is_empty());
        assert!(copy.chunk.fluid_ticks.step_tick(300).is_empty());
        assert_eq!(copy.chunk.block_ticks.step_tick(5200).len(), 1);
        assert_eq!(copy.chunk.fluid_ticks.step_tick(5200).len(), 1);
        assert!(original.chunk.block_ticks.step_tick(5200).is_empty());
        assert!(original.chunk.fluid_ticks.step_tick(5200).is_empty());
        assert_eq!(original.chunk.block_ticks.to_vec()[0].delay, 4900);
        assert_eq!(original.chunk.fluid_ticks.to_vec()[0].delay, 4900);
    }
    assert!(receive.recv().await.is_none());
}

#[derive(Clone, Copy)]
enum DuringWrite {
    Clock,
    Schedule,
    Clear,
}

struct AdvancingConfig {
    level: Arc<Level>,
    mutation: DuringWrite,
}

#[derive(Default)]
struct AdvancingWriter {
    anvil: AnvilChunkFile<ChunkData>,
    context: Option<AdvancingConfig>,
}

impl ChunkSerializer for AdvancingWriter {
    type Data = ChunkData;
    type WriteBackend = PathBuf;
    type ChunkConfig = AdvancingConfig;

    fn get_chunk_key(chunk: &Vector2<i32>) -> String {
        AnvilChunkFile::<ChunkData>::get_chunk_key(chunk)
    }

    fn should_write(&self, watched: bool) -> bool {
        self.anvil.should_write(watched)
    }

    async fn write(&self, path: &PathBuf) -> Result<(), std::io::Error> {
        let context = self.context.as_ref().unwrap();
        let time = context.level.game_time.load(Ordering::Relaxed);
        // The actual serializer boundary lies after encoding and before acknowledgement.
        // Advance on EVERY write, so the original implementation exhausts its bounded drain.
        context.level.set_game_time(time + 1);
        let chunk = context.level.loaded_chunks.get(&CHUNK).unwrap().clone();
        assert!(chunk.is_dirty());
        context.level.set_game_time(time + 2);
        assert!(chunk.is_dirty());
        if time == 1000 && matches!(context.mutation, DuringWrite::Clock) {
            let retained = chunk.clone();
            let copy = run_blocking(move || retained.copy_for_load(&Dimension::OVERWORLD))
                .await
                .unwrap()
                .unwrap();
            assert!(!Arc::ptr_eq(&chunk, &copy));
            assert_eq!(copy.block_ticks.to_vec()[0].delay, 4998);
        }
        if time == 1000 {
            match context.mutation {
                DuringWrite::Clock => {}
                DuringWrite::Schedule => context.level.schedule_block_tick_after(
                    &Block::STONE,
                    BlockPos::new(1, 64, 0),
                    6000,
                    TickPriority::Normal,
                ),
                DuringWrite::Clear => chunk
                    .block_ticks
                    .clear_area(&POSITION, &BlockPos::new(1, 65, 1)),
            }
        }
        self.anvil.write(path).await
    }

    fn read(bytes: Bytes) -> Result<Self, ChunkReadingError> {
        Ok(Self {
            anvil: AnvilChunkFile::read(bytes)?,
            context: None,
        })
    }

    async fn update_chunk(
        &mut self,
        chunk: Arc<ChunkData>,
        config: &AdvancingConfig,
    ) -> Result<(), ChunkWritingError> {
        self.context = Some(AdvancingConfig {
            level: config.level.clone(),
            mutation: config.mutation,
        });
        self.anvil
            .update_chunk(chunk, &AnvilChunkConfig::default())
            .await
    }

    async fn get_chunks(
        &self,
        chunks: Vec<Vector2<i32>>,
        stream: mpsc::Sender<LoadedData<ChunkData, ChunkReadingError>>,
        dimension: Dimension,
    ) {
        self.anvil.get_chunks(chunks, stream, dimension).await;
    }
}

#[tokio::test]
async fn pending_snapshot_clock_only_drain_publishes_once_but_mutations_retry() {
    for mutation in [
        DuringWrite::Clock,
        DuringWrite::Schedule,
        DuringWrite::Clear,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let level = Level::from_root_folder(
            &LevelConfig::default(),
            directory.path().into(),
            0,
            Dimension::OVERWORLD,
        );
        level.set_game_time(1000);
        let chunk = ChunkData::empty_sync(0, 0);
        chunk.light_populated.store(true, Ordering::Relaxed);
        chunk.block_ticks.bind_clock(level.game_time.clone());
        chunk.fluid_ticks.bind_clock(level.game_time.clone());
        level.loaded_chunks.insert(CHUNK, chunk.clone());
        level.schedule_block_tick_after(&Block::STONE, POSITION, 5000, TickPriority::Normal);
        let manager = ChunkFileManager::<AdvancingWriter>::new(AdvancingConfig {
            level: level.clone(),
            mutation,
        });
        manager.queue_chunks(&level.level_folder, vec![(CHUNK, chunk.clone())]);
        let result = manager.block_and_await_ongoing_tasks().await;
        let time = level.game_time.load(Ordering::Relaxed);
        // Stop the native scheduler even when the negative-control assertion will fail.
        level.cancel_token.cancel();
        level.shut_down_chunk_system.store(true, Ordering::Relaxed);
        level.level_channel.notify();
        assert!(
            result.is_ok(),
            "stable save did not drain: game_time={time}, result={result:?}"
        );
        assert_eq!(manager.retained_region_counts_for_test().await.1, 0);
        let expected_time = if matches!(mutation, DuringWrite::Clock) {
            1002
        } else {
            1004
        };
        assert_eq!(
            time, expected_time,
            "unexpected number of region publications"
        );
        // Read published bytes through a new manager, never the pending/live object.
        let reader =
            ChunkFileManager::<AnvilChunkFile<ChunkData>>::new(AnvilChunkConfig::default());
        let (send, mut receive) = mpsc::channel(1);
        reader
            .fetch_chunks(&level.level_folder, &[CHUNK], send)
            .await;
        let Some(LoadedData::Loaded(saved)) = receive.recv().await else {
            panic!("published tick image was lost");
        };
        let ticks = saved.block_ticks.to_vec();
        match mutation {
            DuringWrite::Clock => {
                assert_eq!(ticks.len(), 1);
                assert_eq!(ticks[0].delay, 5000);
                // A later save must still observe elapsed time relative to the encoded image.
                assert!(chunk.is_dirty());
            }
            DuringWrite::Schedule => {
                assert_eq!(ticks.len(), 2);
                assert_eq!(ticks[0].delay, 4998);
                assert_eq!(ticks[1].delay, 6000);
            }
            DuringWrite::Clear => assert!(ticks.is_empty()),
        }
    }
}
