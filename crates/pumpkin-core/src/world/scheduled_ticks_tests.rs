use crate::server::combat_test_support::{publish_empty_chunk, server, world};
use pumpkin_data::{Block, block_properties::RedstoneOreLikeProperties};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};
use std::sync::Arc;

struct TickWorld(Arc<super::World>);

impl Drop for TickWorld {
    fn drop(&mut self) {
        // Keep worker teardown safe when a behavioural assertion fails.
        self.0.level.cancel_token.cancel();
        self.0
            .level
            .shut_down_chunk_system
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.0.level.level_channel.notify();
    }
}

#[tokio::test]
async fn scheduled_time_production_world_tick_publishes_time_while_chunk_is_inactive() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let fixture = TickWorld(world(&server, directory.path()));
    let world = &fixture.0;
    publish_empty_chunk(world, Vector2::new(0, 0));
    let position = BlockPos::new(8, 64, 8);
    let mut properties = RedstoneOreLikeProperties::default(&Block::REDSTONE_LAMP);
    properties.lit = true;
    world.set_block_state(
        &position,
        properties.to_state_id(&Block::REDSTONE_LAMP),
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    // ServerLevel.tickTime precedes LevelTicks.tick; no fixture publishes the clock.
    world.tick(&server);
    world.schedule_block_tick(&Block::REDSTONE_LAMP, position, 4, TickPriority::Normal);
    for _ in 0..5 {
        world.tick(&server);
        assert!(RedstoneOreLikeProperties::from_state_id(world.get_block_state(&position).id).lit);
    }
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    world.tick(&server);
    assert!(!RedstoneOreLikeProperties::from_state_id(world.get_block_state(&position).id).lit);
    assert_eq!(world.level_time.lock().unwrap().world_age, 7);
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
async fn collection_invariant_world_load_publishes_restored_age_before_scheduling() {
    use pumpkin_world::world_info::{LevelData, WorldInfoWriter, anvil::AnvilLevelInfo};
    let directory = tempfile::tempdir().unwrap();
    let mut saved = LevelData::default(pumpkin_util::world_seed::Seed(0));
    saved.game_time = 10_000;
    AnvilLevelInfo
        .write_world_info(&saved, directory.path())
        .unwrap();
    // Both loads use real level.dat decoding and World::load, with no fixture clock publication.
    for _ in 0..2 {
        let server = server(directory.path());
        let fixture = TickWorld(world(&server, directory.path()));
        let world = &fixture.0;
        let chunk_pos = Vector2::new(0, 0);
        publish_empty_chunk(world, chunk_pos);
        world.forced_chunks.lock().unwrap().insert(chunk_pos);
        let position = BlockPos::new(8, 64, 8);
        let mut properties = RedstoneOreLikeProperties::default(&Block::REDSTONE_LAMP);
        properties.lit = true;
        world.set_block_state(
            &position,
            properties.to_state_id(&Block::REDSTONE_LAMP),
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        world.schedule_block_tick(&Block::REDSTONE_LAMP, position, 100, TickPriority::Normal);
        let chunk = world
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .unwrap()
            .value()
            .clone();
        world.tick(&server);
        assert_eq!(world.get_world_age(), 10_001);
        assert!(
            chunk
                .block_ticks
                .is_scheduled(position, &Block::REDSTONE_LAMP),
            "pre-tick schedule fired immediately after restart"
        );
        let pending = chunk.block_ticks.to_vec();
        let tick = pending
            .iter()
            .find(|tick| tick.position == position)
            .unwrap();
        assert_eq!(tick.delay, 99);
        assert_eq!(world.get_world_age() + i64::from(tick.delay), 10_100);
        assert!(RedstoneOreLikeProperties::from_state_id(world.get_block_state(&position).id).lit);
        world.level.shutdown().await.unwrap();
    }
}
