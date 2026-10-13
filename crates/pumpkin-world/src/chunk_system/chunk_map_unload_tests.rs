use super::*;
use crate::{chunk::ChunkData, tick::TickPriority};
use pumpkin_data::{Block, dimension::Dimension, fluid::Fluid};
use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

#[tokio::test]
async fn terrain_unload_cancellation_keeps_both_schedulers_active() {
    let pos = ChunkPos::new(0, 0);
    let position = BlockPos::new(0, 64, 0);
    for cancellation in ["ticket", "retention", "replacement", "none"] {
        let directory = tempfile::tempdir().unwrap();
        let level = Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            directory.path().into(),
            0,
            Dimension::OVERWORLD,
        );
        level.shutdown().await.unwrap();
        let chunk = ChunkData::empty_sync(pos.x, pos.y);
        level.loaded_chunks.insert(pos, chunk.clone());
        level.set_game_time(100);
        level.schedule_block_tick_after(&Block::STONE, position, 100, TickPriority::Normal);
        level.schedule_fluid_tick(&Fluid::WATER, position, 100, TickPriority::Normal);
        let active = FxHashSet::from_iter([pos]);
        let before = level.get_tick_data(&active, 0);
        assert!(before.block_ticks.is_empty() && before.fluid_ticks.is_empty());

        let (mut schedule, _) = storage_failure_tests::scheduler();
        schedule.public_chunk_map = level.loaded_chunks.clone();
        schedule.entity_unloads = level.entity_unloads.clone();
        schedule.chunk_loading = level.chunk_loading.clone();
        schedule.game_time = level.game_time.clone();
        schedule.chunks_with_scheduled_ticks = level.chunks_with_scheduled_ticks.clone();
        schedule.loaded_chunk_changes = level.loaded_chunk_changes.clone();
        schedule.chunk_map.insert(
            pos,
            ChunkHolder {
                chunk: Some(Chunk::Level(chunk.clone())),
                public: true,
                ..Default::default()
            },
        );
        let (writes, mut received) = tokio::sync::mpsc::channel(8);
        schedule.io_write = writes;
        let published = if cancellation == "replacement" {
            let replacement = ChunkData::empty_sync(pos.x, pos.y);
            level.loaded_chunks.insert(pos, replacement.clone());
            replacement
        } else {
            chunk.clone()
        };
        if cancellation == "ticket" {
            level
                .chunk_loading
                .lock()
                .unwrap()
                .add_ticket(pos, ChunkLoading::FULL_CHUNK_LEVEL);
        } else if cancellation == "retention" {
            level.watch_entity_chunks(&[pos]);
        }
        schedule.unload_chunks.insert(pos);
        let mut schedule = tokio::task::spawn_blocking(move || {
            schedule.process_unload_queue();
            schedule
        })
        .await
        .unwrap();

        if cancellation == "none" {
            assert!(!schedule.chunk_map.contains_key(&pos));
            assert!(!level.loaded_chunks.contains_key(&pos));
            assert_eq!(chunk.block_ticks.to_vec()[0].delay, 100);
            assert_eq!(chunk.fluid_ticks.to_vec()[0].delay, 100);
            assert_eq!(received.try_recv().unwrap().len(), 1);
        } else {
            assert!(schedule.chunk_map.contains_key(&pos), "{cancellation}");
            assert!(schedule.unload_chunks.contains(&pos), "{cancellation}");
            assert!(Arc::ptr_eq(
                &level.loaded_chunks.get(&pos).unwrap(),
                &published
            ));
            assert!(received.try_recv().is_err());
            // Restore only the fixture lookup to check the cancelled holder through real collection.
            level.loaded_chunks.insert(pos, chunk.clone());
            level.set_game_time(200);
            let due = level.get_tick_data(&active, 0);
            assert_eq!(due.block_ticks.len(), 1, "{cancellation}");
            assert_eq!(due.fluid_ticks.len(), 1, "{cancellation}");
        }
        // Do not retain the queue sender past this case.
        schedule.unload_chunks.clear();
    }
}
