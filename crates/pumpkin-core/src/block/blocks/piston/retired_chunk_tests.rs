use crate::block::entities::piston::PistonBlockEntity;
use crate::server::combat_test_support;
use pumpkin_data::Block;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;

#[tokio::test]
async fn piston_push_into_retired_loaded_neighbour_preserves_block() {
    // PistonBaseBlock.neighborChanged/triggerEvent moves into a non-ticking LevelChunk.
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let source_chunk = Vector2::new(0, 0);
    let neighbour = Vector2::new(1, 0);
    combat_test_support::publish_empty_chunk(&world, source_chunk);
    combat_test_support::publish_empty_chunk(&world, neighbour);
    world.active_chunks.write().unwrap().insert(source_chunk);

    let piston = BlockPos::new(14, 64, 8);
    let source = BlockPos::new(15, 64, 8);
    let destination = BlockPos::new(16, 64, 8);
    let power = BlockPos::new(13, 64, 8);
    let piston_state = Block::PISTON
        .from_properties(&[("facing", "east"), ("extended", "false")])
        .to_state_id(&Block::PISTON);
    world.set_block_state(&piston, piston_state, BlockFlags::FORCE_STATE);
    world.set_block_state(
        &source,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert!(world.unload_block_entities(neighbour));
    assert!(world.level.loaded_chunks.get(&neighbour).is_some());
    assert!(!world.active_chunks.read().unwrap().contains(&neighbour));

    // The real neighbor notification queues the event; only the source chunk ticks.
    world.set_block_state(
        &power,
        Block::REDSTONE_BLOCK.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    world.flush_synced_block_events();
    assert_eq!(
        world.get_block(&destination),
        &Block::MOVING_PISTON,
        "piston destination write was refused while its source was cleared"
    );
    let pushed = world.get_block_entity(&destination).unwrap();
    let pushed = pushed.as_any().downcast_ref::<PistonBlockEntity>().unwrap();
    assert_eq!(pushed.pushed_block_state.id, Block::STONE.default_state.id);

    // Complete the real block-entity animation without activating the neighbor.
    for position in [source, destination] {
        let moving = world.get_block_entity(&position).unwrap();
        for _ in 0..3 {
            moving.tick(&world);
        }
    }
    assert_eq!(world.get_block(&source), &Block::PISTON_HEAD);
    assert_eq!(world.get_block(&destination), &Block::STONE);
    assert!(world.get_block_entity(&destination).is_none());
}
