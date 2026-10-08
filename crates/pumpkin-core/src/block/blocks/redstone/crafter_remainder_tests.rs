use super::*;
use crate::server::combat_test_support::{server, world};
use pumpkin_data::item::Item;
use pumpkin_util::math::vector2::Vector2;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deep_review_crafter_cake_then_planks_does_not_dispense_free_buckets() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = BlockPos::new(8, 64, 8);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::CRAFTER.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let crafter = Arc::new(CrafterBlockEntity::new(pos));
    for (i, item) in [
        &Item::MILK_BUCKET,
        &Item::MILK_BUCKET,
        &Item::MILK_BUCKET,
        &Item::SUGAR,
        &Item::EGG,
        &Item::SUGAR,
        &Item::WHEAT,
        &Item::WHEAT,
        &Item::WHEAT,
    ]
    .into_iter()
    .enumerate()
    {
        crafter.set_stack(i, ItemStack::new(1, item));
    }
    world.add_block_entity(crafter.clone());
    CrafterBlock::dispense_from(&world, &pos, &Block::CRAFTER);
    let count_buckets = || {
        world
            .entities
            .load()
            .iter()
            .filter_map(|e| e.get_item_entity())
            .map(|e| e.get_item_stack().lock().unwrap().clone())
            .filter(|s| s.item == &Item::BUCKET)
            .map(|s| u32::from(s.item_count))
            .sum::<u32>()
    };
    assert_eq!(count_buckets(), 3);
    for _ in 0..2 {
        crafter.set_stack(8, ItemStack::new(1, &Item::OAK_LOG));
        CrafterBlock::dispense_from(&world, &pos, &Block::CRAFTER);
    }
    assert_eq!(count_buckets(), 3);
    assert!(world.level.shutdown().await.is_ok());
}
