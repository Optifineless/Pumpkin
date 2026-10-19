use crate::{
    entity::EntityBase, net::java::combat_test_support::TestPlayer, server::combat_test_support,
};
use pumpkin_data::{Block, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bucket_retired_loaded_neighbour_pickup_conserves_fluid() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let neighbour = Vector2::new(1, 0);
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    combat_test_support::publish_empty_chunk(&world, neighbour);
    let actor = TestPlayer::new(&world);
    actor
        .player
        .get_entity()
        .set_pos(Vector3::new(15.5, 64.0, 8.5));
    actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::BUCKET));
    let water = BlockPos::new(16, 65, 8);
    world.set_block_state(
        &water,
        Block::WATER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let storage = world.level.get_entity_chunk(neighbour).await.unwrap();
    assert!(world.make_chunk_entities_live(&storage, None));
    world.level.request_entity_unloads([neighbour]);
    world.process_entity_chunk_lifecycle();
    assert!(!storage.live.load(std::sync::atomic::Ordering::Acquire));
    assert!(world.level.is_chunk_loaded(&neighbour));

    // BucketItem.use reaches across the terrain/entity ticket boundary through the packet handler.
    actor.client().handle_use_item(
        &actor.player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: -90.0,
            pitch: 0.0,
        },
        &server,
    );
    let filled =
        actor.player.inventory().get_stack_in_hand(Hand::Right).item == &Item::WATER_BUCKET;
    let removed = world.get_block(&water) == &Block::AIR;
    world.level.shutdown().await.unwrap();
    assert_eq!(
        removed, filled,
        "bucket filling and source removal diverged"
    );
    assert!(filled, "loaded neighbour refused the bucket pickup");
}
