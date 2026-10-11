use super::*;
use crate::{
    chunk::ChunkData,
    tick::{ScheduledTick, TickPriority},
};
use pumpkin_config::world::LevelConfig;
use pumpkin_data::{Block, dimension::Dimension, fluid::Fluid};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

fn fixture() -> (tempfile::TempDir, Arc<Level>) {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().to_path_buf(),
        0,
        Dimension::OVERWORLD,
    );
    (dir, level)
}

// Pause after the real collector decides to unregister. If it owns the contract lock,
// defer the producer until it can proceed. Without that ownership, complete the producer
// inside the losing window, before allowing the collector to remove the registration.
fn at_unregister(level: &Arc<Level>, producer: impl FnOnce() + Send + 'static) {
    let (paused, arrived) = mpsc::channel();
    let (resume, proceed) = mpsc::channel();
    *level.before_tick_unregister.lock().unwrap() = Some(Box::new(move || {
        paused.send(()).unwrap();
        proceed.recv_timeout(Duration::from_secs(10)).unwrap();
    }));
    let collecting = level.clone();
    let collector = std::thread::spawn(move || {
        collecting.get_tick_data(&FxHashSet::from_iter([Vector2::new(0, 0)]), 0)
    });
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    let producer = if level.chunks_with_scheduled_ticks.try_lock().is_err() {
        Some(std::thread::spawn(producer))
    } else {
        producer();
        None
    };
    resume.send(()).unwrap();
    collector.join().unwrap();
    if let Some(producer) = producer {
        producer.join().unwrap();
    }
}

#[tokio::test]
async fn scheduling_at_collector_unregistration_keeps_block_and_fluid_ticks() {
    let (_dir, level) = fixture();
    let pos = Vector2::new(0, 0);
    let block_pos = BlockPos::new(0, 64, 0);
    level.loaded_chunks.insert(pos, ChunkData::empty_sync(0, 0));
    level.schedule_block_tick(&Block::STONE, block_pos, 1, TickPriority::Normal);
    let scheduling = level.clone();
    at_unregister(&level, move || {
        scheduling.schedule_block_tick(&Block::STONE, block_pos, 1, TickPriority::Normal);
        scheduling.schedule_fluid_tick(&Fluid::WATER, block_pos, 1, TickPriority::Normal);
    });
    let next = level.get_tick_data(&FxHashSet::from_iter([pos]), 0);
    assert_eq!(
        next.block_ticks.len(),
        1,
        "new block tick lost its registration"
    );
    assert_eq!(
        next.fluid_ticks.len(),
        1,
        "new fluid tick lost its registration"
    );
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn loaded_container_registration_survives_stale_unregistration() {
    let (_dir, level) = fixture();
    let pos = Vector2::new(0, 0);
    level.loaded_chunks.insert(pos, ChunkData::empty_sync(0, 0));
    level.schedule_block_tick(
        &Block::STONE,
        BlockPos::new(0, 64, 0),
        1,
        TickPriority::Normal,
    );
    level.loaded_chunks.remove(&pos);
    let loading = level.clone();
    at_unregister(&level, move || {
        let chunk = ChunkData::empty_sync(0, 0);
        // A loaded container arrives with SavedTick entries, before LevelTicks.addContainer.
        chunk.block_ticks.schedule_tick(
            &ScheduledTick {
                delay: 1,
                position: BlockPos::new(1, 64, 0),
                priority: TickPriority::Normal,
                value: &Block::DIRT,
            },
            0,
        );
        loading.loaded_chunks.insert(pos, chunk);
        loading.register_loaded_chunk_ticks(pos);
    });
    let next = level.get_tick_data(&FxHashSet::from_iter([pos]), 0);
    assert_eq!(
        next.block_ticks.len(),
        1,
        "saved tick lost its loaded-container registration"
    );
    assert_eq!(next.block_ticks[0].position, BlockPos::new(1, 64, 0));
    level.shutdown().await.unwrap();
}
