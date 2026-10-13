use super::*;
use crate::chunk::format::anvil::{SingleChunkDataSerializer, WORLD_DATA_VERSION};
use crate::tick::TickPriority;
use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
use std::{
    future::{Future, poll_fn},
    ops::Deref,
    task::Poll,
    time::Duration,
};

const CHUNK: Vector2<i32> = Vector2::new(0, 0);
const BLOCK_POS: BlockPos = BlockPos::new(0, 64, 0);
const MAX_ASYNC_POLLS: usize = 10_000;

struct TestLevel(Arc<Level>);

impl Deref for TestLevel {
    type Target = Arc<Level>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TestLevel {
    fn drop(&mut self) {
        // An assertion failure must also stop the native scheduler before runtime teardown.
        self.cancel_token.cancel();
        self.shut_down_chunk_system.store(true, Ordering::Relaxed);
        self.level_channel.notify();
    }
}

fn level(path: &std::path::Path) -> TestLevel {
    TestLevel(Level::from_root_folder(
        &LevelConfig::default(),
        path.into(),
        0,
        Dimension::OVERWORLD,
    ))
}

async fn bounded<F: Future>(operation: &str, future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    for _ in 0..MAX_ASYNC_POLLS {
        if let Poll::Ready(result) = poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await {
            return result;
        }
        // Let filesystem and native scheduler workers run between counted polls.
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("{operation} remained pending after {MAX_ASYNC_POLLS} polls");
}

fn active() -> FxHashSet<Vector2<i32>> {
    FxHashSet::from_iter([CHUNK])
}

fn insert_empty(level: &Level) -> SyncChunk {
    let chunk = ChunkData::empty_sync(CHUNK.x, CHUNK.y);
    // SerializableChunkData.parse/read preserves isLightOn for a completed full chunk.
    // Unlit fixtures enter terrain generation, which requires a world portal.
    chunk.light_populated.store(true, Ordering::Relaxed);
    level.unpack_chunk_ticks(&chunk);
    level.loaded_chunks.insert(CHUNK, chunk.clone());
    chunk
}

fn saved_delay(chunk: &ChunkData, list: &str) -> i32 {
    let bytes = chunk.to_bytes().unwrap();
    let mut cursor = std::io::Cursor::new(bytes.as_ref());
    let nbt = Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
        &mut cursor,
    ))
    .unwrap();
    let NbtTag::Compound(tick) = &nbt.root_tag.get_list(list).unwrap()[0] else {
        panic!("saved tick must be a compound");
    };
    tick.get_int("t").unwrap()
}

#[tokio::test]
async fn scheduled_time_long_and_short_ticks_resume_on_first_simulated_tick() {
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    level.set_game_time(100);
    insert_empty(&level);
    level.schedule_block_tick_after(&Block::STONE, BLOCK_POS, 5000, TickPriority::Normal);
    level.schedule_fluid_tick(&Fluid::WATER, BLOCK_POS, 5, TickPriority::Normal);
    assert!(level.get_tick_data(&active(), 0).block_ticks.is_empty());
    level.set_game_time(6000);
    let inactive = level.get_tick_data(&FxHashSet::default(), 0);
    assert!(inactive.block_ticks.is_empty());
    assert!(inactive.fluid_ticks.is_empty());
    let resumed = level.get_tick_data(&active(), 0);
    assert_eq!(resumed.block_ticks.len(), 1);
    assert_eq!(resumed.fluid_ticks.len(), 1);
    assert_eq!(resumed.block_ticks[0].position, BLOCK_POS);
    assert!(!level.chunks_with_scheduled_ticks.contains(&CHUNK));
    bounded("level shutdown", level.shutdown()).await.unwrap();
}

#[tokio::test]
async fn scheduled_time_saved_delay_includes_unsimulated_game_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    level.set_game_time(100);
    let chunk = insert_empty(&level);
    level.schedule_block_tick_after(&Block::STONE, BLOCK_POS, 5000, TickPriority::Normal);
    level.schedule_fluid_tick(&Fluid::WATER, BLOCK_POS, 5, TickPriority::Normal);
    // No scheduler step between scheduling and saving, including for the short wheel.
    level.set_game_time(1100);
    level.get_tick_data(&FxHashSet::default(), 0);
    assert_eq!(saved_delay(&chunk, "block_ticks"), 4000);
    assert_eq!(saved_delay(&chunk, "fluid_ticks"), -995);
    bounded("level shutdown", level.shutdown()).await.unwrap();
}

#[tokio::test]
async fn scheduled_time_overdue_save_reloads_through_chunk_load_and_runs_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let old = level(dir.path());
    old.set_game_time(100);
    let chunk = insert_empty(&old);
    old.schedule_block_tick_after(&Block::STONE, BLOCK_POS, 5000, TickPriority::Normal);
    old.schedule_fluid_tick(&Fluid::WATER, BLOCK_POS, 5, TickPriority::Normal);
    old.set_game_time(6100);
    old.get_tick_data(&FxHashSet::default(), 0);
    assert_eq!(saved_delay(&chunk, "block_ticks"), -1000);
    assert_eq!(saved_delay(&chunk, "fluid_ticks"), -5995);
    chunk.mark_dirty(true);
    bounded(
        "overdue chunk save",
        old.chunk_saver
            .save_chunks(&old.level_folder, vec![(CHUNK, chunk)]),
    )
    .await
    .unwrap();
    bounded("old level shutdown", old.shutdown()).await.unwrap();
    drop(old);

    let loaded = level(dir.path());
    loaded.set_game_time(20_000);
    bounded(
        "overdue chunk load (saved full chunk must be lit)",
        loaded.get_or_fetch_chunk(CHUNK, |_| ()),
    )
    .await
    .unwrap();
    assert!(
        loaded
            .get_tick_data(&FxHashSet::default(), 0)
            .block_ticks
            .is_empty()
    );
    let due = loaded.get_tick_data(&active(), 0);
    assert_eq!(due.block_ticks.len(), 1);
    assert_eq!(due.fluid_ticks.len(), 1);
    bounded("loaded level shutdown", loaded.shutdown())
        .await
        .unwrap();
}

#[tokio::test]
async fn scheduled_time_marker_survives_schedule_between_empty_check_and_remove() {
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    insert_empty(&level);
    level.schedule_block_tick(&Block::STONE, BLOCK_POS, 1, TickPriority::Normal);
    let weak = Arc::downgrade(&level);
    *level.before_tick_chunk_remove.lock().unwrap() = Some(Box::new(move || {
        // Collector has drained the old work and observed empty. The concurrent scheduler
        // inserts new work and its marker before the collector removes that marker.
        let level = weak.upgrade().unwrap();
        level.schedule_fluid_tick(&Fluid::WATER, BLOCK_POS, 1, TickPriority::High);
    }));
    level.set_game_time(1);
    assert_eq!(level.get_tick_data(&active(), 0).block_ticks.len(), 1);
    assert!(level.chunks_with_scheduled_ticks.contains(&CHUNK));
    level.set_game_time(2);
    assert_eq!(level.get_tick_data(&active(), 0).fluid_ticks.len(), 1);
    bounded("level shutdown", level.shutdown()).await.unwrap();
}

#[tokio::test]
async fn scheduled_time_old_fork_and_vanilla_tick_lists_load_with_relative_delays() {
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    // The old fork saves positive remaining delays; vanilla also saves zero and negative t.
    let mut root = NbtCompound::new();
    root.put_int("DataVersion", WORLD_DATA_VERSION);
    root.put_int("xPos", CHUNK.x);
    root.put_int("zPos", CHUNK.y);
    root.put_string("Status", "minecraft:full".into());
    root.put_bool("isLightOn", true);
    root.put_list("sections", vec![]);
    let tick_list = |id: &str| {
        [-4, 0, 17]
            .into_iter()
            .enumerate()
            .map(|(x, delay)| {
                let mut tick = NbtCompound::new();
                tick.put_string("i", id.into());
                tick.put_int("x", x as i32);
                tick.put_int("y", 64);
                tick.put_int("z", 0);
                tick.put_int("t", delay);
                tick.put_int("p", TickPriority::Normal as i32);
                NbtTag::Compound(tick)
            })
            .collect()
    };
    root.put_list("block_ticks", tick_list("minecraft:stone"));
    root.put_list("fluid_ticks", tick_list("minecraft:water"));
    let chunk = Arc::new(
        ChunkData::internal_from_bytes(
            &Nbt::new(String::new(), root).write(),
            CHUNK,
            &Dimension::OVERWORLD,
        )
        .unwrap(),
    );
    chunk.mark_dirty(true);
    bounded(
        "old fork and vanilla chunk save",
        level
            .chunk_saver
            .save_chunks(&level.level_folder, vec![(CHUNK, chunk)]),
    )
    .await
    .unwrap();
    level.set_game_time(1000);
    bounded(
        "old fork and vanilla chunk load (saved full chunk must be lit)",
        level.get_or_fetch_chunk(CHUNK, |_| ()),
    )
    .await
    .unwrap();
    let due = level.get_tick_data(&active(), 0);
    assert_eq!(
        due.block_ticks
            .iter()
            .map(|tick| tick.position.0.x)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(due.fluid_ticks.len(), 2);
    level.set_game_time(1016);
    assert!(level.get_tick_data(&active(), 0).block_ticks.is_empty());
    level.set_game_time(1017);
    let due = level.get_tick_data(&active(), 0);
    assert_eq!(due.block_ticks[0].position.0.x, 2);
    assert_eq!(due.fluid_ticks.len(), 1);
    bounded("level shutdown", level.shutdown()).await.unwrap();
}

#[tokio::test]
async fn scheduled_time_catchup_keeps_future_wheel_slots_and_vanilla_container_order() {
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    insert_empty(&level);
    let other = Vector2::new(1, 0);
    let other_chunk = ChunkData::empty_sync(other.x, other.y);
    level.unpack_chunk_ticks(&other_chunk);
    level.loaded_chunks.insert(other, other_chunk);
    let later = BlockPos::new(1, 64, 0);
    let future = BlockPos::new(2, 64, 0);
    let peer = BlockPos::new(16, 64, 0);
    level.schedule_block_tick(&Block::STONE, BLOCK_POS, 1, TickPriority::Low);
    level.schedule_block_tick(&Block::STONE, later, 2, TickPriority::High);
    level.schedule_block_tick(&Block::STONE, peer, 1, TickPriority::Normal);
    level.set_game_time(1000);
    // Its wheel bucket collides with an overdue bucket, but it is still in the future.
    level.schedule_block_tick(&Block::STONE, future, 25, TickPriority::ExtremelyHigh);
    let active = FxHashSet::from_iter([CHUNK, other]);
    let due = level.get_tick_data(&active, 0);
    // Vanilla compares container heads by priority, preserving trigger order inside a chunk.
    assert_eq!(
        due.block_ticks
            .iter()
            .map(|tick| tick.position)
            .collect::<Vec<_>>(),
        [peer, BLOCK_POS, later]
    );
    level.set_game_time(1024);
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
    level.set_game_time(1025);
    assert_eq!(
        level.get_tick_data(&active, 0).block_ticks[0].position,
        future
    );
    bounded("level shutdown", level.shutdown()).await.unwrap();
}
