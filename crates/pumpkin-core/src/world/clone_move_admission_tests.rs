use crate::block::entities::{BlockEntity, chest::ChestBlockEntity};
use crate::command::CommandSender;
use crate::server::combat_test_support;
use pumpkin_data::{Block, item::Item, item_stack::ItemStack};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

fn diamonds_at(world: &crate::world::World, pos: &BlockPos) -> Option<u8> {
    let inventory = world.get_block_entity(pos)?.get_inventory()?;
    Some(inventory.get_stack(0).item_count)
}

#[tokio::test]
async fn clone_move_from_unloading_chunk_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let source_chunk = Vector2::new(0, 0);
    combat_test_support::publish_empty_chunk(&world, source_chunk);
    combat_test_support::publish_empty_chunk(&world, Vector2::new(1, 0));
    let (source, dest) = (BlockPos::new(8, 64, 8), BlockPos::new(24, 64, 8));
    let dest_before = world.get_block_state_id(&dest);
    world.set_block_state(
        &source,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let chest: Arc<dyn BlockEntity> = Arc::new(ChestBlockEntity::new(source));
    world.add_block_entity(chest.clone());
    chest
        .get_inventory()
        .unwrap()
        .set_stack(0, ItemStack::new(64, &Item::DIAMOND));
    let console = CommandSender::Console.into_source(&server);
    let dispatcher = server.command_dispatcher.load();
    let command = "clone 8 64 8 8 64 8 24 64 8 replace move";

    // The source chunk's final capture is in flight, so its chest cannot be cleared.
    assert!(world.begin_block_entity_unload(source_chunk));
    assert!(dispatcher.execute_input(command, &console).is_err());
    world.finish_block_entity_unload(source_chunk);
    assert_eq!(world.get_block_state_id(&dest), dest_before);
    assert!(world.get_block_entity(&dest).is_none());
    assert_eq!(diamonds_at(&world, &source), Some(64));

    // Control: with the chunk admitted again, the same command moves the chest exactly once.
    assert_eq!(dispatcher.execute_input(command, &console).unwrap(), 1);
    assert_eq!(diamonds_at(&world, &dest), Some(64));
    assert_eq!(diamonds_at(&world, &source), None);
    assert_ne!(
        world.get_block_state_id(&source),
        Block::CHEST.default_state.id
    );
}
