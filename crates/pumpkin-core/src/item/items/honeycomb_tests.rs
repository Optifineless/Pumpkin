use crate::{
    entity::EntityBase, net::java::combat_test_support::TestPlayer, server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{
        Half, HorizontalFacing, SlabType, StairsShape, WhiteWoolSlabLikeProperties,
        WhiteWoolStairsLikeProperties,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItemOn};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::Arc;

fn fixture() -> (
    tempfile::TempDir,
    Arc<crate::server::Server>,
    Arc<crate::world::World>,
    TestPlayer,
) {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );

    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 0.5));
    player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);

    (dir, server, world, player)
}

fn use_honeycomb(
    player: &TestPlayer,
    server: &Arc<crate::server::Server>,
    position: BlockPos,
    sequence: i32,
) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(sequence),
            },
            server,
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_preserves_top_copper_slab() {
    wax_slab_case(SlabType::Top, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_preserves_double_copper_slab() {
    wax_slab_case(SlabType::Double, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_preserves_waterlogged_top_copper_slab() {
    wax_slab_case(SlabType::Top, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_preserves_waterlogged_bottom_copper_slab() {
    wax_slab_case(SlabType::Bottom, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_preserves_bottom_copper_slab_control() {
    wax_slab_case(SlabType::Bottom, false).await;
}

async fn wax_slab_case(slab_type: SlabType, waterlogged: bool) {
    let (_dir, server, world, player) = fixture();
    let position = BlockPos::new(0, 64, 1);
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEYCOMB));

    let mut properties = WhiteWoolSlabLikeProperties::default(&Block::CUT_COPPER_SLAB);
    properties.r#type = slab_type;
    properties.waterlogged = waterlogged;
    world.set_block_state(
        &position,
        properties.to_state_id(&Block::CUT_COPPER_SLAB),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );

    use_honeycomb(&player, &server, position, 1);

    let waxed_state_id = world.get_block_state_id(&position);
    assert_eq!(
        Block::from_state_id(waxed_state_id),
        &Block::WAXED_CUT_COPPER_SLAB
    );
    let waxed_properties = WhiteWoolSlabLikeProperties::from_state_id(waxed_state_id);
    assert_eq!(waxed_properties.r#type, slab_type);
    assert_eq!(waxed_properties.waterlogged, waterlogged);
    assert_eq!(
        player.player.inventory().held_item().item_count,
        1,
        "successful waxing consumes exactly one honeycomb"
    );

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_preserves_copper_stair_shape() {
    let (_dir, server, world, player) = fixture();
    let position = BlockPos::new(0, 64, 1);
    let mut properties = WhiteWoolStairsLikeProperties::default(&Block::CUT_COPPER_STAIRS);
    properties.facing = HorizontalFacing::East;
    properties.half = Half::Top;
    properties.shape = StairsShape::InnerLeft;
    properties.waterlogged = true;
    world.set_block_state(
        &position,
        properties.to_state_id(&Block::CUT_COPPER_STAIRS),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEYCOMB));

    use_honeycomb(&player, &server, position, 1);

    let waxed_state_id = world.get_block_state_id(&position);
    assert_eq!(
        Block::from_state_id(waxed_state_id),
        &Block::WAXED_CUT_COPPER_STAIRS
    );
    let waxed_properties = WhiteWoolStairsLikeProperties::from_state_id(waxed_state_id);
    assert_eq!(waxed_properties.facing, HorizontalFacing::East);
    assert_eq!(waxed_properties.half, Half::Top);
    assert_eq!(waxed_properties.shape, StairsShape::InnerLeft);
    assert!(waxed_properties.waterlogged);
    assert_eq!(player.player.inventory().held_item().item_count, 1);

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_honeycomb_use_does_not_consume_for_non_waxable_block() {
    let (_dir, server, world, player) = fixture();
    let position = BlockPos::new(0, 64, 1);
    world.set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEYCOMB));

    use_honeycomb(&player, &server, position, 1);

    assert_eq!(player.player.inventory().held_item().item_count, 2);
    assert_eq!(
        world.get_block_state_id(&position),
        Block::STONE.default_state.id
    );

    world.level.shutdown().await.unwrap();
}
