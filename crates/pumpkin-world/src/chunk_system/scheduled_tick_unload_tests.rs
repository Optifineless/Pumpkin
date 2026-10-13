use super::*;
use crate::{
    chunk::{
        ChunkData,
        format::anvil::AnvilChunkFile,
        io::{
            FileIO,
            file_manager::{ChunkFileManager, LevelFileIO},
        },
    },
    tick::TickPriority,
};
use pumpkin_config::{
    chunk::{AnvilChunkConfig, ChunkConfig},
    world::LevelConfig,
};
use pumpkin_data::{Block, dimension::Dimension, fluid::Fluid};
use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;
use std::ops::Deref;

const CHUNK: ChunkPos = ChunkPos::new(0, 0);
const POSITION: BlockPos = BlockPos::new(0, 64, 0);

struct TestLevel(Arc<Level>);

impl Deref for TestLevel {
    type Target = Arc<Level>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TestLevel {
    fn drop(&mut self) {
        self.cancel_token.cancel();
        self.shut_down_chunk_system.store(true, Relaxed);
        self.level_channel.notify();
    }
}

fn manager(level: &Level) -> &ChunkFileManager<AnvilChunkFile<ChunkData>> {
    let LevelFileIO::Anvil(manager) = level.chunk_saver.as_ref() else {
        panic!("fixture must use Anvil storage");
    };
    manager
}

fn ticket_for_reload(level: &Level) {
    let mut tickets = level.chunk_loading.lock().unwrap();
    tickets.add_ticket(CHUNK, ChunkLoading::FULL_CHUNK_LEVEL);
    tickets.send_change();
}

async fn unload(level: &Arc<Level>, chunk: SyncChunk) -> tokio::task::JoinHandle<()> {
    let (mut schedule, _) = super::storage_failure_tests::scheduler();
    schedule.public_chunk_map = level.loaded_chunks.clone();
    schedule.entity_unloads = level.entity_unloads.clone();
    schedule.chunk_loading = level.chunk_loading.clone();
    schedule.game_time = level.game_time.clone();
    schedule.chunks_with_scheduled_ticks = level.chunks_with_scheduled_ticks.clone();
    schedule.loaded_chunk_changes = level.loaded_chunk_changes.clone();
    schedule.chunk_map.insert(
        CHUNK,
        ChunkHolder {
            chunk: Some(Chunk::Level(chunk)),
            public: true,
            ..Default::default()
        },
    );
    let (writes, received) = tokio::sync::mpsc::channel(8);
    schedule.io_write = writes;
    let worker = tokio::spawn(io_write_work(
        received,
        level.clone(),
        schedule.io_lock.clone(),
    ));
    schedule.unload_chunks.insert(CHUNK);
    let schedule = tokio::task::spawn_blocking(move || {
        schedule.process_unload_queue();
        schedule
    })
    .await
    .unwrap();
    assert!(!schedule.chunk_map.contains_key(&CHUNK));
    assert!(!level.loaded_chunks.contains_key(&CHUNK));
    drop(schedule); // Close its write channel; joining the real worker fences publication.
    worker
}

fn clear_during_publication(chunk: &ChunkData) {
    let max = BlockPos::new(1, 65, 1);
    chunk.block_ticks.clear_area(&POSITION, &max);
    chunk.fluid_ticks.clear_area(&POSITION, &max);
}

fn assert_waiting(level: &Level, active: &FxHashSet<ChunkPos>, time: i64) {
    level.set_game_time(time);
    let due = level.get_tick_data(active, 0);
    assert!(
        due.block_ticks.is_empty(),
        "completed unload consumed the saved block delay"
    );
    assert!(
        due.fluid_ticks.is_empty(),
        "completed unload consumed the saved fluid delay"
    );
}

fn assert_due_at(level: &Level, active: &FxHashSet<ChunkPos>, time: i64, position: BlockPos) {
    level.set_game_time(time);
    let due = level.get_tick_data(active, 0);
    assert_eq!(
        due.block_ticks
            .iter()
            .map(|tick| tick.position)
            .collect::<Vec<_>>(),
        [position]
    );
    assert_eq!(
        due.fluid_ticks
            .iter()
            .map(|tick| tick.position)
            .collect::<Vec<_>>(),
        [position]
    );
}

#[tokio::test]
async fn scheduled_time_completed_unload_publication_reloads_relative_delays() {
    for mutate in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = LevelConfig {
            chunk: ChunkConfig::Anvil(AnvilChunkConfig::default()),
            ..Default::default()
        };
        let level = TestLevel(Level::from_root_folder(
            &config,
            directory.path().into(),
            0,
            Dimension::OVERWORLD,
        ));
        let active = FxHashSet::from_iter([CHUNK]);
        level.set_game_time(100);
        let chunk = ChunkData::empty_sync(CHUNK.x, CHUNK.y);
        chunk.light_populated.store(true, Relaxed);
        level.loaded_chunks.insert(CHUNK, chunk.clone());
        level.schedule_block_tick_after(&Block::STONE, POSITION, 100, TickPriority::Normal);
        level.schedule_fluid_tick(&Fluid::WATER, POSITION, 100, TickPriority::Normal);
        assert_waiting(&level, &active, 100); // Already simulated, unlike a fresh saved-list fixture.

        let (started, start) = tokio::sync::oneshot::channel();
        let (resume, resumed) = tokio::sync::oneshot::channel();
        manager(&level).pause_next_publication_for_test(started, resumed);
        let worker = unload(&level, chunk.clone()).await;
        start.await.unwrap(); // Real bytes serialized; disk publication has not completed.
        level.set_game_time(101);
        if mutate {
            clear_during_publication(&chunk);
        }
        resume.send(()).unwrap();
        worker.await.unwrap(); // Completed save, released position I/O ownership, no rewatch.
        let after_publication = manager(&level).retained_region_counts_for_test().await;
        if mutate {
            assert!(
                chunk.is_dirty(),
                "publication acknowledged a genuinely newer tick queue"
            );
            assert_eq!(
                after_publication.1, 1,
                "newer tick queue was dropped from pending"
            );
            level
                .chunk_saver
                .block_and_await_ongoing_tasks()
                .await
                .unwrap();
            assert!(!chunk.is_dirty());
        }
        level.set_game_time(300);
        assert!(!level.loaded_chunks.contains_key(&CHUNK));
        level.get_tick_data(&FxHashSet::default(), 0);
        // Exercise real eviction and disk decoding, not an already-activated pending Arc.
        level.chunk_saver.clear_watched_chunks().await;
        assert_eq!(
            manager(&level).retained_region_counts_for_test().await,
            (0, 0)
        );
        ticket_for_reload(&level);
        let loaded = level.get_or_fetch_chunk(CHUNK, Clone::clone).await.unwrap();
        assert_waiting(&level, &FxHashSet::default(), 300);
        assert_waiting(&level, &active, 300);
        assert!(
            !Arc::ptr_eq(&loaded, &chunk),
            "reload reused the retired scheduler"
        );
        if mutate {
            assert_waiting(&level, &active, 400);
        } else {
            assert_eq!(
                after_publication,
                (1, 0),
                "elapsed-only time retained the published original"
            );
        }
        if !mutate {
            assert_waiting(&level, &active, 399);
            assert_due_at(&level, &active, 400, POSITION);
        }
        level.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn collection_invariant_unpublished_chunk_preserves_ticks_before_save_completion() {
    let directory = tempfile::tempdir().unwrap();
    let config = LevelConfig {
        chunk: ChunkConfig::Anvil(AnvilChunkConfig::default()),
        ..Default::default()
    };
    let level = TestLevel(Level::from_root_folder(
        &config,
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    ));
    let active = FxHashSet::from_iter([CHUNK]);
    level.set_game_time(100);
    let chunk = ChunkData::empty_sync(CHUNK.x, CHUNK.y);
    chunk.light_populated.store(true, Relaxed);
    level.loaded_chunks.insert(CHUNK, chunk.clone());
    let future = BlockPos::new(1, 64, 0);
    level.schedule_block_tick_after(&Block::STONE, POSITION, 0, TickPriority::High);
    level.schedule_block_tick_after(&Block::STONE, future, 100, TickPriority::Normal);
    level.schedule_fluid_tick(&Fluid::WATER, POSITION, 0, TickPriority::High);
    level.schedule_fluid_tick(&Fluid::WATER, future, 100, TickPriority::Normal);
    let retained = crate::level::level_ticks::retain_containers(&level, &active, 100);
    assert_eq!(retained.len(), 1);

    let (started, start) = tokio::sync::oneshot::channel();
    let (resume, resumed) = tokio::sync::oneshot::channel();
    manager(&level).pause_next_publication_for_test(started, resumed);
    let worker = unload(&level, chunk.clone()).await;
    start.await.unwrap();
    assert!(!level.loaded_chunks.contains_key(&CHUNK));
    // This existing storage boundary follows freezing; it does not expose removal vs freeze.
    let mut due = crate::level::TickData {
        block_ticks: Vec::new(),
        fluid_ticks: Vec::new(),
        random_ticks: Vec::new(),
    };
    crate::level::level_ticks::drain_retained_containers(&level, &active, &mut due, retained, 100);
    assert!(
        due.block_ticks.is_empty(),
        "unpublished block ticks escaped"
    );
    assert!(
        due.fluid_ticks.is_empty(),
        "unpublished fluid ticks escaped"
    );
    assert_eq!(
        chunk
            .block_ticks
            .to_vec()
            .iter()
            .map(|t| t.delay)
            .collect::<Vec<_>>(),
        [0, 100]
    );
    assert_eq!(
        chunk
            .fluid_ticks
            .to_vec()
            .iter()
            .map(|t| t.delay)
            .collect::<Vec<_>>(),
        [0, 100]
    );
    resume.send(()).unwrap();
    worker.await.unwrap();

    // Evict retained storage so reload exercises persisted bytes and fresh schedulers.
    level.chunk_saver.clear_watched_chunks().await;
    assert_eq!(
        manager(&level).retained_region_counts_for_test().await,
        (0, 0)
    );
    level.set_game_time(300);
    ticket_for_reload(&level);
    let loaded = level.get_or_fetch_chunk(CHUNK, Clone::clone).await.unwrap();
    assert!(!Arc::ptr_eq(&loaded, &chunk));
    assert_due_at(&level, &active, 300, POSITION);
    assert_waiting(&level, &active, 399);
    assert_due_at(&level, &active, 400, future);
    assert_waiting(&level, &active, 401);
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn collection_invariant_retained_arc_cannot_unpack_after_real_unload() {
    let directory = tempfile::tempdir().unwrap();
    let config = LevelConfig {
        chunk: ChunkConfig::Anvil(AnvilChunkConfig::default()),
        ..Default::default()
    };
    let level = TestLevel(Level::from_root_folder(
        &config,
        directory.path().into(),
        0,
        Dimension::OVERWORLD,
    ));
    let active = FxHashSet::from_iter([CHUNK]);
    level.set_game_time(100);
    let chunk = ChunkData::empty_sync(CHUNK.x, CHUNK.y);
    chunk.light_populated.store(true, Relaxed);
    level.loaded_chunks.insert(CHUNK, chunk.clone());
    let future = BlockPos::new(1, 64, 0);
    level.schedule_block_tick_after(&Block::STONE, POSITION, 0, TickPriority::High);
    level.schedule_block_tick_after(&Block::STONE, future, 100, TickPriority::Normal);
    level.schedule_fluid_tick(&Fluid::WATER, POSITION, 0, TickPriority::High);
    level.schedule_fluid_tick(&Fluid::WATER, future, 100, TickPriority::Normal);
    // Complete Arc retention, then drive native unpublication, freezing and the real save worker.
    let retained = crate::level::level_ticks::retain_containers(&level, &active, 100);
    assert_eq!(retained.len(), 1);
    let worker = unload(&level, chunk.clone()).await;
    worker.await.unwrap();
    level.set_game_time(300);
    let mut due = crate::level::TickData {
        block_ticks: Vec::new(),
        fluid_ticks: Vec::new(),
        random_ticks: Vec::new(),
    };
    crate::level::level_ticks::drain_retained_containers(&level, &active, &mut due, retained, 300);
    assert!(
        due.block_ticks.is_empty(),
        "collector drained a retired scheduler"
    );
    assert!(due.fluid_ticks.is_empty());
    assert_eq!(
        chunk
            .block_ticks
            .to_vec()
            .iter()
            .map(|t| t.delay)
            .collect::<Vec<_>>(),
        [0, 100]
    );
    // Force the retained-pending path, not only disk decoding, and fence storage completion.
    chunk.mark_dirty(true);
    level
        .chunk_saver
        .queue_chunks(&level.level_folder, vec![(CHUNK, chunk.clone())]);
    ticket_for_reload(&level);
    let loaded = level.get_or_fetch_chunk(CHUNK, Clone::clone).await.unwrap();
    assert!(!Arc::ptr_eq(&loaded, &chunk));
    assert_due_at(&level, &active, 300, POSITION);
    assert_waiting(&level, &active, 399);
    assert_due_at(&level, &active, 400, future);
    assert_eq!(
        chunk
            .block_ticks
            .to_vec()
            .iter()
            .map(|t| t.delay)
            .collect::<Vec<_>>(),
        [0, 100]
    );
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn scheduler_retirement_late_producer_persists_and_fires_after_reload() {
    for during_publication in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = LevelConfig {
            chunk: ChunkConfig::Anvil(AnvilChunkConfig::default()),
            ..Default::default()
        };
        let level = TestLevel(Level::from_root_folder(
            &config,
            directory.path().into(),
            0,
            Dimension::OVERWORLD,
        ));
        let active = FxHashSet::from_iter([CHUNK]);
        level.set_game_time(100);
        let chunk = ChunkData::empty_sync(CHUNK.x, CHUNK.y);
        chunk.light_populated.store(true, Relaxed);
        level.loaded_chunks.insert(CHUNK, chunk.clone());
        level.schedule_block_tick_after(&Block::STONE, POSITION, 100, TickPriority::Normal);
        level.schedule_fluid_tick(&Fluid::WATER, POSITION, 100, TickPriority::Normal);
        assert_waiting(&level, &active, 100);
        chunk.block_ticks.freeze_for_unload(100);
        chunk.fluid_ticks.freeze_for_unload(100);
        let late = BlockPos::new(1, 64, 0);
        let (started, start) = tokio::sync::oneshot::channel();
        let (resume, resumed) = tokio::sync::oneshot::channel();
        manager(&level).pause_next_publication_for_test(started, resumed);
        if !during_publication {
            // Sequential fallback: frozen but public, through the actual producer entry points.
            level.set_game_time(101);
            level.schedule_block_tick_after(&Block::STONE, late, 10, TickPriority::High);
            level.schedule_fluid_tick(&Fluid::WATER, late, 10, TickPriority::High);
        }
        let worker = unload(&level, chunk.clone()).await;
        start.await.unwrap(); // Existing storage boundary: the first image is already encoded.
        if during_publication {
            level.set_game_time(101);
            schedule_late_retained_ticks(&chunk, late);
        }
        assert_eq!(
            chunk
                .block_ticks
                .to_vec()
                .iter()
                .map(|t| t.delay)
                .collect::<Vec<_>>(),
            [100, 11]
        );
        assert_eq!(
            chunk
                .fluid_ticks
                .to_vec()
                .iter()
                .map(|t| t.delay)
                .collect::<Vec<_>>(),
            [100, 11]
        );
        assert!(chunk.block_ticks.prepare_tick(200).is_none());
        assert!(chunk.fluid_ticks.prepare_tick(200).is_none());
        resume.send(()).unwrap();
        worker.await.unwrap();
        if during_publication {
            assert!(
                chunk.is_dirty(),
                "late producer mutation was acknowledged as published"
            );
            assert_eq!(manager(&level).retained_region_counts_for_test().await.1, 1);
            level
                .chunk_saver
                .block_and_await_ongoing_tasks()
                .await
                .unwrap();
            assert!(!chunk.is_dirty());
        }
        assert_late_tick_disk_reload(&level, &chunk, &active, late).await;
        level.shutdown().await.unwrap();
    }
}

fn schedule_late_retained_ticks(chunk: &ChunkData, late: BlockPos) {
    chunk.block_ticks.schedule_tick(
        &crate::tick::ScheduledTick {
            delay: 10,
            priority: TickPriority::High,
            position: late,
            value: &Block::STONE,
        },
        2,
    );
    chunk.fluid_ticks.schedule_tick(
        &crate::tick::ScheduledTick {
            delay: 10,
            priority: TickPriority::High,
            position: late,
            value: &Fluid::WATER,
        },
        3,
    );
}

async fn assert_late_tick_disk_reload(
    level: &Arc<Level>,
    chunk: &SyncChunk,
    active: &FxHashSet<ChunkPos>,
    late: BlockPos,
) {
    level.chunk_saver.clear_watched_chunks().await;
    assert_eq!(
        manager(level).retained_region_counts_for_test().await,
        (0, 0)
    );
    level.set_game_time(300);
    ticket_for_reload(level);
    let loaded = level.get_or_fetch_chunk(CHUNK, Clone::clone).await.unwrap();
    assert!(!Arc::ptr_eq(&loaded, chunk));
    assert_waiting(level, active, 300);
    assert_waiting(level, active, 310);
    assert_due_at(level, active, 311, late);
    assert_waiting(level, active, 399);
    assert_due_at(level, active, 400, POSITION);
    assert_waiting(level, active, 401);
}
