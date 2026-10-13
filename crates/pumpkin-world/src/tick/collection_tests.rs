use super::*;
use crate::{chunk::ChunkData, level::Level};
use pumpkin_config::world::LevelConfig;
use pumpkin_data::{Block, dimension::Dimension};
use pumpkin_util::math::vector2::Vector2;
use rustc_hash::FxHashSet;
use std::{hint::black_box, sync::atomic::Ordering, time::Instant};

fn position(chunk: usize, entry: usize) -> BlockPos {
    BlockPos::new(
        chunk as i32 * 16 + (entry % 16) as i32,
        (entry / 256) as i32 - 64,
        ((entry / 16) % 16) as i32,
    )
}

async fn fixture(
    chunks: usize,
) -> (
    tempfile::TempDir,
    std::sync::Arc<Level>,
    FxHashSet<Vector2<i32>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    level.shutdown().await.unwrap();
    for index in 0..chunks {
        level.loaded_chunks.insert(
            Vector2::new(index as i32, 0),
            ChunkData::empty_sync(index as i32, 0),
        );
    }
    (
        dir,
        level,
        (0..chunks).map(|i| Vector2::new(i as i32, 0)).collect(),
    )
}

fn schedule(level: &Level, chunks: usize, count: usize, delay: u32) {
    for index in 0..count {
        level.schedule_block_tick_after(
            &Block::STONE,
            position(index % chunks, index / chunks),
            delay,
            TickPriority::values()[index % 7],
        );
    }
}

#[tokio::test]
async fn budget_boundary_keeps_global_order_and_remainder() {
    let (_dir, level, active) = fixture(2).await;
    // Equal triggers exercise both vanilla's global intra-tick order and budget carry-over.
    schedule(&level, 2, 70_000, 1);
    level.set_game_time(1);
    let first = level.get_tick_data(&active, 0).block_ticks;
    let second = level.get_tick_data(&active, 0).block_ticks;
    assert_eq!(first.len(), 65_536);
    assert_eq!(second.len(), 4_464);
    let all: Vec<_> = first.into_iter().chain(second).collect();
    assert!(all.windows(2).all(|pair| pair[0] <= pair[1]));
    let actual: FxHashSet<_> = all.iter().map(|tick| tick.position).collect();
    let expected: FxHashSet<_> = (0..70_000).map(|i| position(i % 2, i / 2)).collect();
    assert_eq!(actual, expected);
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
}

#[tokio::test]
async fn backward_time_does_not_lose_or_starve_ticks() {
    let (_dir, level, active) = fixture(2).await;
    let initial = position(1, 100);
    level.schedule_block_tick_after(&Block::STONE, initial, 4800, TickPriority::Normal);
    level.set_game_time(4800);
    let due = level.get_tick_data(&active, 0).block_ticks;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].position, initial);
    schedule(&level, 2, 20, 1);
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
    level.set_game_time(0);
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
    let mut deadlines: rustc_hash::FxHashMap<_, _> = (0..20)
        .map(|index| (position(index % 2, index / 2), 4801))
        .collect();
    for index in 20..40 {
        let delay = if index % 2 == 0 { 1 } else { 300 };
        let pos = position(index % 2, index / 2);
        deadlines.insert(pos, i64::from(delay));
        level.schedule_block_tick_after(&Block::STONE, pos, delay, TickPriority::Normal);
    }
    let mut seen = FxHashSet::default();
    for time in 1..=5057 {
        level.set_game_time(time);
        for tick in level.get_tick_data(&active, 0).block_ticks {
            let trigger = deadlines[&tick.position];
            assert!((trigger..=trigger + 256).contains(&time));
            assert!(seen.insert(tick.position));
        }
    }
    assert_eq!(seen.len(), 40);
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
}

#[test]
fn double_freeze_unpack_preserves_pending_first_pack_order() {
    let clock = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(100));
    let saved = |index| SavedTick {
        delay: 10,
        priority: TickPriority::Normal,
        position: position(0, index),
        value: &Block::STONE,
    };
    let scheduler: scheduler::ChunkTickScheduler<_> = [saved(0), saved(1)].into_iter().collect();
    scheduler.bind_clock(clock.clone());
    scheduler.schedule_tick(
        &ScheduledTick {
            delay: 10,
            priority: TickPriority::Normal,
            position: position(0, 2),
            value: &Block::STONE,
        },
        0,
    );
    scheduler.freeze_for_unload(100);
    assert_eq!(
        scheduler
            .to_vec()
            .iter()
            .map(|t| t.position)
            .collect::<Vec<_>>(),
        [position(0, 0), position(0, 1), position(0, 2)]
    );
    assert!(drain_containers(&[&scheduler], 200, MAX_SCHEDULED_TICKS_PER_TICK).is_empty());
    clock.store(200, Ordering::Relaxed);
    scheduler.schedule_tick(
        &ScheduledTick {
            delay: 10,
            priority: TickPriority::Normal,
            position: position(0, 3),
            value: &Block::STONE,
        },
        1,
    );
    scheduler.freeze_for_unload(200);
    let packed = scheduler.to_vec();
    assert_eq!(
        packed.iter().map(|t| t.position).collect::<Vec<_>>(),
        [
            position(0, 0),
            position(0, 1),
            position(0, 2),
            position(0, 3)
        ]
    );
    assert!(packed.iter().all(|t| t.delay == 10));
    clock.store(205, Ordering::Relaxed);
    assert!(drain_containers(&[&scheduler], 205, MAX_SCHEDULED_TICKS_PER_TICK).is_empty());
    let due = drain_containers(&[&scheduler], 215, MAX_SCHEDULED_TICKS_PER_TICK);
    assert_eq!(
        due.iter().map(|t| t.position).collect::<Vec<_>>(),
        [
            position(0, 0),
            position(0, 1),
            position(0, 2),
            position(0, 3)
        ]
    );
    assert!(!scheduler.has_ticks());
}

#[tokio::test]
async fn collection_releases_dedup_and_refreshes_earlier_deadline() {
    let (_dir, level, active) = fixture(2).await;
    let early = position(0, 0);
    let later = position(0, 1);
    level.schedule_block_tick_after(&Block::STONE, later, 1000, TickPriority::Normal);
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
    level.schedule_block_tick_after(&Block::STONE, early, 1, TickPriority::Normal);
    level.set_game_time(1);
    let due = level.get_tick_data(&active, 0).block_ticks;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].position, early);
    assert!(!level.is_block_tick_scheduled(&early, &Block::STONE));
    level.schedule_block_tick_after(&Block::STONE, early, 1, TickPriority::Normal);
    level.set_game_time(2);
    let due = level.get_tick_data(&active, 0).block_ticks;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].position, early);
    level.set_game_time(1000);
    let due = level.get_tick_data(&active, 0).block_ticks;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].position, later);
}

#[tokio::test]
#[ignore = "collection timing fixture; run alone with --nocapture"]
#[expect(
    clippy::print_stderr,
    reason = "ignored timing fixture reports its samples"
)]
async fn collection_measurement() {
    for (chunks, count, delay) in [
        (8, 8, 100_000),
        (200, 200, 100_000),
        (8, 100, 1),
        (8, 1000, 1),
        (8, 5000, 1),
        (8, 160_000, 1),
    ] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            let (_dir, level, active) = fixture(chunks).await;
            schedule(&level, chunks, count, delay);
            level.set_game_time(5000);
            let start = Instant::now();
            if delay > 5000 {
                for _ in 0..1000 {
                    assert!(
                        black_box(level.get_tick_data(&active, 0))
                            .block_ticks
                            .is_empty()
                    );
                }
                samples.push(start.elapsed().as_nanos() / 1000);
            } else {
                let mut collected = 0;
                while collected < count {
                    collected += black_box(level.get_tick_data(&active, 0)).block_ticks.len();
                }
                samples.push(start.elapsed().as_nanos());
                assert_eq!(collected, count);
            }
        }
        samples.sort_unstable();
        eprintln!(
            "K={chunks} N={count} delay={delay}: median_ns={} samples={samples:?}",
            samples[3]
        );
    }
}
