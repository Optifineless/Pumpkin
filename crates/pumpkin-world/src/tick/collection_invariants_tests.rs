use super::*;
use crate::{chunk::ChunkData, level::Level};
use pumpkin_config::world::LevelConfig;
use pumpkin_data::{Block, dimension::Dimension};
use pumpkin_util::math::vector2::Vector2;
use rustc_hash::FxHashSet;

async fn fixture() -> (tempfile::TempDir, std::sync::Arc<Level>) {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    level.shutdown().await.unwrap();
    for x in 0..2 {
        level
            .loaded_chunks
            .insert(Vector2::new(x, 0), ChunkData::empty_sync(x, 0));
    }
    (dir, level)
}

#[tokio::test]
async fn collection_invariant_clear_rekeys_first_prepared_head() {
    for clear in [false, true] {
        let (_dir, level) = fixture().await;
        let high = BlockPos::new(0, 64, 0);
        let low = BlockPos::new(1, 64, 0);
        let normal = BlockPos::new(16, 64, 0);
        for (pos, priority) in [
            (high, TickPriority::High),
            (low, TickPriority::Low),
            (normal, TickPriority::Normal),
        ] {
            level.schedule_block_tick_after(&Block::STONE, pos, 1, priority);
        }
        level.set_game_time(1);
        let chunks: Vec<_> = [Vector2::new(0, 0), Vector2::new(1, 0)]
            .into_iter()
            .map(|pos| level.loaded_chunks.get(&pos).unwrap().value().clone())
            .collect();
        let containers = [&chunks[0].block_ticks, &chunks[1].block_ticks];
        // Complete the real preparation boundary before the real clear entry point.
        let prepared: Vec<_> = containers
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.prepare_tick(1).unwrap()))
            .collect();
        if clear {
            level.clear_scheduled_block_ticks(&high, &BlockPos::new(1, 65, 1));
        }
        let due = drain_prepared_containers(&containers, prepared, MAX_SCHEDULED_TICKS_PER_TICK);
        let actual: Vec<_> = due.iter().map(|t| t.position).collect();
        let expected = if clear {
            vec![normal, low]
        } else {
            vec![high, normal, low]
        };
        assert_eq!(actual, expected, "clear={clear}");
        assert!(
            level
                .get_tick_data(
                    &FxHashSet::from_iter([Vector2::new(0, 0), Vector2::new(1, 0)]),
                    0
                )
                .block_ticks
                .is_empty()
        );
    }
}

#[tokio::test]
async fn collection_invariant_unprepared_container_admits_delay_zero_in_order() {
    let (_dir, level) = fixture().await;
    let low = BlockPos::new(0, 64, 0);
    let normal = BlockPos::new(16, 64, 0);
    let high = BlockPos::new(17, 64, 0);
    level.schedule_block_tick_after(&Block::STONE, low, 1, TickPriority::Low);
    level.schedule_block_tick_after(&Block::STONE, normal, 1, TickPriority::Normal);
    level.set_game_time(1);
    let a = level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap()
        .value()
        .clone();
    let b = level
        .loaded_chunks
        .get(&Vector2::new(1, 0))
        .unwrap()
        .value()
        .clone();
    let containers = [&a.block_ticks, &b.block_ticks];
    // The lazy preparation boundary completes A, then schedules into B before preparing B.
    let prepared = containers.iter().enumerate().map(|(i, c)| {
        if i == 1 {
            level.schedule_block_tick_after(&Block::STONE, high, 0, TickPriority::High);
        }
        (i, c.prepare_tick(1).unwrap())
    });
    let due = drain_prepared_containers(&containers, prepared, MAX_SCHEDULED_TICKS_PER_TICK);
    assert_eq!(
        due.iter().map(|t| t.position).collect::<Vec<_>>(),
        [high, normal, low]
    );
}

#[test]
fn collection_invariant_empty_retirement_survives_clear_and_repeated_freeze() {
    let scheduler = scheduler::ChunkTickScheduler::default();
    scheduler.freeze_for_unload(100);
    scheduler.clear_area(&BlockPos::new(0, 0, 0), &BlockPos::new(16, 256, 16));
    scheduler.freeze_for_unload(200);
    scheduler.schedule_tick(
        &ScheduledTick {
            delay: 0,
            priority: TickPriority::Normal,
            position: BlockPos::new(0, 64, 0),
            value: &Block::STONE,
        },
        0,
    );
    assert!(scheduler.has_ticks());
    assert!(scheduler.prepare_tick(200).is_none());
    let mut due = Vec::new();
    assert!(scheduler.poll_due_batch(1, None, &mut due).is_none());
    assert!(due.is_empty());
}
