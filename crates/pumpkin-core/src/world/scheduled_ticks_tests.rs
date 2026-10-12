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
