use super::*;
use crate::chunk::format::anvil::{SingleChunkDataSerializer, WORLD_DATA_VERSION};
use crate::tick::TickPriority;
use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
use std::ops::Deref;

const CHUNK: Vector2<i32> = Vector2::new(0, 0);
const BLOCK_POS: BlockPos = BlockPos::new(0, 64, 0);

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

fn active() -> FxHashSet<Vector2<i32>> {
    FxHashSet::from_iter([CHUNK])
}

fn insert_empty(level: &Level) -> SyncChunk {
    let chunk = ChunkData::empty_sync(CHUNK.x, CHUNK.y);
    // SerializableChunkData.parse/read preserves isLightOn for a completed full chunk.
    // Unlit fixtures enter terrain generation, which requires a world portal.
    chunk.light_populated.store(true, Ordering::Relaxed);
    level.bind_chunk_tick_clocks(&chunk);
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
    assert!(
        !level
            .chunks_with_scheduled_ticks
            .lock()
            .unwrap()
            .contains(&CHUNK)
    );
    level.shutdown().await.unwrap();
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
    level.shutdown().await.unwrap();
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
    old.chunk_saver
        .save_chunks(&old.level_folder, vec![(CHUNK, chunk)])
        .await
        .unwrap();
    old.shutdown().await.unwrap();
    drop(old);

    let loaded = level(dir.path());
    loaded.set_game_time(20_000);
    loaded.get_or_fetch_chunk(CHUNK, |_| ()).await.unwrap();
    assert!(
        loaded
            .get_tick_data(&FxHashSet::default(), 0)
            .block_ticks
            .is_empty()
    );
    let due = loaded.get_tick_data(&active(), 0);
    assert_eq!(due.block_ticks.len(), 1);
    assert_eq!(due.fluid_ticks.len(), 1);
    loaded.shutdown().await.unwrap();
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
    assert!(
        level
            .chunks_with_scheduled_ticks
            .lock()
            .unwrap()
            .contains(&CHUNK)
    );
    level.set_game_time(2);
    assert_eq!(level.get_tick_data(&active(), 0).fluid_ticks.len(), 1);
    level.shutdown().await.unwrap();
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
    level
        .chunk_saver
        .save_chunks(&level.level_folder, vec![(CHUNK, chunk)])
        .await
        .unwrap();
    level.set_game_time(1000);
    level.get_or_fetch_chunk(CHUNK, |_| ()).await.unwrap();
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
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn scheduled_time_catchup_keeps_future_wheel_slots_and_vanilla_container_order() {
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    insert_empty(&level);
    let other = Vector2::new(1, 0);
    let other_chunk = ChunkData::empty_sync(other.x, other.y);
    level.bind_chunk_tick_clocks(&other_chunk);
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
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn scheduled_time_clean_chunk_resaves_elapsed_and_drained_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let old = level(dir.path());
    old.set_game_time(100);
    let chunk = insert_empty(&old);
    old.schedule_block_tick_after(&Block::STONE, BLOCK_POS, 5000, TickPriority::Normal);
    old.schedule_fluid_tick(&Fluid::WATER, BLOCK_POS, 5, TickPriority::Normal);
    old.chunk_saver
        .save_chunks(&old.level_folder, vec![(CHUNK, chunk.clone())])
        .await
        .unwrap();
    assert!(!chunk.dirty.load(Ordering::Relaxed));
    assert!(!chunk.is_dirty());

    old.set_game_time(1100);
    old.get_tick_data(&FxHashSet::default(), 0);
    // No block, light or entity mutation: only the serialized delays have changed.
    assert!(!chunk.dirty.load(Ordering::Relaxed));
    old.chunk_saver
        .save_chunks(&old.level_folder, vec![(CHUNK, chunk)])
        .await
        .unwrap();
    old.shutdown().await.unwrap();
    drop(old);

    let loaded = level(dir.path());
    loaded.set_game_time(20_000);
    let chunk = loaded
        .get_or_fetch_chunk(CHUNK, Clone::clone)
        .await
        .unwrap();
    assert_eq!(saved_delay(&chunk, "block_ticks"), 4000);
    assert_eq!(saved_delay(&chunk, "fluid_ticks"), -995);
    assert_eq!(loaded.get_tick_data(&active(), 0).fluid_ticks.len(), 1);
    loaded.set_game_time(24_000);
    assert_eq!(loaded.get_tick_data(&active(), 0).block_ticks.len(), 1);
    // Collection itself must persist the empty lists even if the callback is a no-op.
    loaded
        .chunk_saver
        .save_chunks(&loaded.level_folder, vec![(CHUNK, chunk)])
        .await
        .unwrap();
    loaded.shutdown().await.unwrap();
    drop(loaded);

    let restarted = level(dir.path());
    restarted.get_or_fetch_chunk(CHUNK, |_| ()).await.unwrap();
    let due = restarted.get_tick_data(&active(), 0);
    assert!(due.block_ticks.is_empty());
    assert!(due.fluid_ticks.is_empty());
    assert!(
        !restarted
            .loaded_chunks
            .get(&CHUNK)
            .unwrap()
            .block_ticks
            .has_ticks()
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn scheduled_time_overdue_backlog_obeys_each_type_budget_and_container_order() {
    use crate::tick::MAX_SCHEDULED_TICKS_PER_TICK;
    const PER_CHUNK: usize = 40_000;
    let dir = tempfile::tempdir().unwrap();
    let level = level(dir.path());
    insert_empty(&level);
    let other = Vector2::new(1, 0);
    let chunk = ChunkData::empty_sync(other.x, other.y);
    level.bind_chunk_tick_clocks(&chunk);
    level.loaded_chunks.insert(other, chunk);
    let position = |chunk_x: i32, index: usize| {
        BlockPos::new(
            chunk_x * 16 + (index % 16) as i32,
            (index / 256) as i32,
            ((index / 16) % 16) as i32,
        )
    };
    for index in 0..PER_CHUNK {
        let late = index >= PER_CHUNK / 2;
        let priority = if late {
            TickPriority::ExtremelyHigh
        } else {
            TickPriority::Low
        };
        level.schedule_block_tick(
            &Block::STONE,
            position(0, index),
            if late { 2 } else { 1 },
            priority,
        );
        level.schedule_block_tick(&Block::STONE, position(1, index), 1, TickPriority::Normal);
        level.schedule_fluid_tick(&Fluid::WATER, position(0, index), 1, TickPriority::Normal);
        level.schedule_fluid_tick(&Fluid::WATER, position(1, index), 1, TickPriority::Low);
    }
    level.set_game_time(1000);
    let active = FxHashSet::from_iter([CHUNK, other]);
    let first = level.get_tick_data(&active, 0);
    assert_eq!(first.block_ticks.len(), MAX_SCHEDULED_TICKS_PER_TICK);
    assert_eq!(first.fluid_ticks.len(), MAX_SCHEDULED_TICKS_PER_TICK);
    let expected_blocks = (0..PER_CHUNK)
        .map(|i| position(1, i))
        .chain((0..PER_CHUNK).map(|i| position(0, i)))
        .collect::<Vec<_>>();
    let expected_fluids = (0..PER_CHUNK)
        .map(|i| position(0, i))
        .chain((0..PER_CHUNK).map(|i| position(1, i)))
        .collect::<Vec<_>>();
    assert_eq!(
        tick_positions(&first.block_ticks),
        expected_blocks[..MAX_SCHEDULED_TICKS_PER_TICK]
    );
    assert_eq!(
        tick_positions(&first.fluid_ticks),
        expected_fluids[..MAX_SCHEDULED_TICKS_PER_TICK]
    );
    let remaining = expected_blocks[MAX_SCHEDULED_TICKS_PER_TICK];
    assert!(
        level
            .loaded_chunks
            .get(&CHUNK)
            .unwrap()
            .block_ticks
            .is_scheduled(remaining, &Block::STONE)
    );
    // Leftovers remain serializable and registered while the next logical tick is pending.
    assert_eq!(
        level
            .loaded_chunks
            .get(&CHUNK)
            .unwrap()
            .block_ticks
            .to_vec()
            .len(),
        2 * PER_CHUNK - MAX_SCHEDULED_TICKS_PER_TICK
    );
    level.set_game_time(1001);
    let second = level.get_tick_data(&active, 0);
    assert_eq!(
        tick_positions(&second.block_ticks),
        expected_blocks[MAX_SCHEDULED_TICKS_PER_TICK..]
    );
    assert_eq!(
        tick_positions(&second.fluid_ticks),
        expected_fluids[MAX_SCHEDULED_TICKS_PER_TICK..]
    );
    assert!(level.chunks_with_scheduled_ticks.lock().unwrap().is_empty());
    assert!(level.get_tick_data(&active, 0).block_ticks.is_empty());
    level.shutdown().await.unwrap();
}

fn tick_positions<T>(ticks: &[OrderedTick<T>]) -> Vec<BlockPos> {
    ticks.iter().map(|tick| tick.position).collect()
}

#[tokio::test]
async fn scheduled_time_draining_a_clean_chunk_persists_the_empty_queue() {
    let dir = tempfile::tempdir().unwrap();
    let old = level(dir.path());
    let chunk = insert_empty(&old);
    old.schedule_block_tick(&Block::STONE, BLOCK_POS, 1, TickPriority::Normal);
    chunk.mark_dirty(true);
    old.chunk_saver
        .save_chunks(&old.level_folder, vec![(CHUNK, chunk.clone())])
        .await
        .unwrap();
    assert!(!chunk.is_dirty());
    old.set_game_time(1);
    assert_eq!(old.get_tick_data(&active(), 0).block_ticks.len(), 1);
    assert!(!chunk.dirty.load(Ordering::Relaxed));
    old.chunk_saver
        .save_chunks(&old.level_folder, vec![(CHUNK, chunk)])
        .await
        .unwrap();
    old.shutdown().await.unwrap();
    drop(old);
    let restarted = level(dir.path());
    let chunk = restarted
        .get_or_fetch_chunk(CHUNK, Clone::clone)
        .await
        .unwrap();
    assert!(!chunk.block_ticks.has_ticks());
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn scheduled_time_disk_ticks_stay_frozen_until_first_simulation() {
    let dir = tempfile::tempdir().unwrap();
    let old = level(dir.path());
    let chunk = insert_empty(&old);
    old.schedule_block_tick(&Block::STONE, BLOCK_POS, 100, TickPriority::Normal);
    old.schedule_fluid_tick(&Fluid::WATER, BLOCK_POS, 100, TickPriority::Normal);
    old.chunk_saver
        .save_chunks(&old.level_folder, vec![(CHUNK, chunk)])
        .await
        .unwrap();
    old.shutdown().await.unwrap();
    drop(old);

    let loaded = level(dir.path());
    loaded.set_game_time(1000);
    let chunk = loaded
        .get_or_fetch_chunk(CHUNK, Clone::clone)
        .await
        .unwrap();
    // Full/public but inactive. Fresh runtime work still uses the live world clock.
    let fresh = BlockPos::new(1, 64, 0);
    loaded.schedule_block_tick(&Block::STONE, fresh, 5, TickPriority::Normal);
    loaded.set_game_time(1200);
    let inactive = loaded.get_tick_data(&FxHashSet::default(), 0);
    assert!(inactive.block_ticks.is_empty());
    assert!(inactive.fluid_ticks.is_empty());
    assert_eq!(saved_delay(&chunk, "block_ticks"), 100);
    assert_eq!(saved_delay(&chunk, "fluid_ticks"), 100);
    loaded
        .chunk_saver
        .save_chunks(&loaded.level_folder, vec![(CHUNK, chunk.clone())])
        .await
        .unwrap();
    let first = loaded.get_tick_data(&active(), 0);
    assert_eq!(tick_positions(&first.block_ticks), [fresh]);
    assert!(first.fluid_ticks.is_empty());
    loaded.set_game_time(1299);
    let waiting = loaded.get_tick_data(&active(), 0);
    assert!(waiting.block_ticks.is_empty());
    assert!(waiting.fluid_ticks.is_empty());
    loaded.set_game_time(1300);
    let due = loaded.get_tick_data(&active(), 0);
    assert_eq!(tick_positions(&due.block_ticks), [BLOCK_POS]);
    assert_eq!(tick_positions(&due.fluid_ticks), [BLOCK_POS]);
    // Once activated, later inactivity uses absolute deadlines, without another rebase.
    loaded.schedule_block_tick(&Block::STONE, BLOCK_POS, 100, TickPriority::Normal);
    loaded.set_game_time(1500);
    assert!(
        loaded
            .get_tick_data(&FxHashSet::default(), 0)
            .block_ticks
            .is_empty()
    );
    assert_eq!(
        tick_positions(&loaded.get_tick_data(&active(), 0).block_ticks),
        [BLOCK_POS]
    );
    loaded.shutdown().await.unwrap();
}
