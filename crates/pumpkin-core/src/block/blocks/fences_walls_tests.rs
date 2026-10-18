use super::Fixture;
use crate::entity::EntityBase;
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{
        EastWall, HorizontalFacing, NorthWall, OakFenceGateLikeProperties, OakFenceLikeProperties,
        ResinBrickWallLikeProperties, SlabType, SouthWall, WhiteWoolSlabLikeProperties,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

fn place(fixture: &Fixture, block: &'static Block, item: &'static Item, position: BlockPos) {
    fixture.player.player.get_entity().set_pos(Vector3::new(
        f64::from(position.0.x) + 0.5,
        f64::from(position.0.y),
        f64::from(position.0.z) + 2.5,
    ));
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, item));
    fixture.use_block(position.down(), BlockDirection::Up);
    assert_eq!(fixture.world.get_block(&position), block);
}

fn wall_state(fixture: &Fixture, position: BlockPos) -> ResinBrickWallLikeProperties {
    ResinBrickWallLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
}

fn connects_east(fixture: &Fixture, block: &Block, position: BlockPos) -> bool {
    if block == &Block::OAK_FENCE {
        OakFenceLikeProperties::from_state_id(fixture.world.get_block_state_id(&position)).east
    } else {
        wall_state(fixture, position).east != EastWall::None
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fences_and_walls_do_not_connect_to_melons() {
    let f = Fixture::new();

    for (position, block, item) in [
        (BlockPos::new(3, 64, 3), &Block::OAK_FENCE, &Item::OAK_FENCE),
        (
            BlockPos::new(8, 64, 3),
            &Block::COBBLESTONE_WALL,
            &Item::COBBLESTONE_WALL,
        ),
    ] {
        let neighbor = position.offset(BlockDirection::East.to_offset());
        f.set(position.down(), Block::STONE.default_state.id);
        f.set(neighbor, Block::MELON.default_state.id);
        place(&f, block, item, position);
        assert!(!connects_east(&f, block, position));

        f.world.set_block_state(
            &neighbor,
            Block::STONE.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        assert!(connects_east(&f, block, position));
        f.world.set_block_state(
            &neighbor,
            Block::MELON.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        assert!(!connects_east(&f, block, position));
    }

    let fence_position = BlockPos::new(3, 64, 7);
    let fence_neighbor = fence_position.offset(BlockDirection::East.to_offset());
    f.set(fence_position.down(), Block::STONE.default_state.id);
    f.set(fence_neighbor, Block::OAK_FENCE.default_state.id);
    place(&f, &Block::OAK_FENCE, &Item::OAK_FENCE, fence_position);
    assert!(connects_east(&f, &Block::OAK_FENCE, fence_position));

    let wall_position = BlockPos::new(8, 64, 7);
    let wall_neighbor = wall_position.offset(BlockDirection::East.to_offset());
    f.set(wall_position.down(), Block::STONE.default_state.id);
    f.set(wall_neighbor, Block::STONE_BRICK_WALL.default_state.id);
    place(
        &f,
        &Block::COBBLESTONE_WALL,
        &Item::COBBLESTONE_WALL,
        wall_position,
    );
    assert!(connects_east(&f, &Block::COBBLESTONE_WALL, wall_position));

    let fence_gate_position = BlockPos::new(3, 64, 11);
    let fence_gate_neighbor = fence_position_north(fence_gate_position);
    let mut gate = OakFenceGateLikeProperties::default(&Block::OAK_FENCE_GATE);
    gate.facing = HorizontalFacing::East;
    f.set(
        fence_gate_neighbor,
        gate.to_state_id(&Block::OAK_FENCE_GATE),
    );
    f.set(fence_gate_position.down(), Block::STONE.default_state.id);
    place(&f, &Block::OAK_FENCE, &Item::OAK_FENCE, fence_gate_position);
    assert!(
        OakFenceLikeProperties::from_state_id(f.world.get_block_state_id(&fence_gate_position))
            .north
    );

    let wall_gate_position = BlockPos::new(8, 64, 11);
    let wall_gate_neighbor = fence_position_north(wall_gate_position);
    f.set(wall_gate_neighbor, gate.to_state_id(&Block::OAK_FENCE_GATE));
    f.set(wall_gate_position.down(), Block::STONE.default_state.id);
    place(
        &f,
        &Block::COBBLESTONE_WALL,
        &Item::COBBLESTONE_WALL,
        wall_gate_position,
    );
    assert_ne!(wall_state(&f, wall_gate_position).north, NorthWall::None);

    assert!(f.world.level.shutdown().await.is_ok());
}

fn fence_position_north(position: BlockPos) -> BlockPos {
    position.offset(BlockDirection::North.to_offset())
}

fn set_above(fixture: &Fixture, position: BlockPos, state_id: BlockStateId) {
    fixture
        .world
        .set_block_state(&position, state_id, BlockFlags::NOTIFY_ALL);
}

fn assert_wall_north_south_post(
    fixture: &Fixture,
    position: BlockPos,
    north: NorthWall,
    south: SouthWall,
    up: bool,
    reason: &str,
) {
    let state = wall_state(fixture, position);
    assert_eq!(state.north, north);
    assert_eq!(state.south, south);
    assert_eq!(state.up, up, "{reason}");
}

fn check_post_and_slab_controls(fixture: &Fixture, position: BlockPos, above: BlockPos) {
    assert_wall_north_south_post(
        fixture,
        position,
        NorthWall::Low,
        SouthWall::Low,
        false,
        "a straight wall post stays low under air",
    );

    set_above(fixture, above, Block::TORCH.default_state.id);
    assert_wall_north_south_post(
        fixture,
        position,
        NorthWall::Low,
        SouthWall::Low,
        true,
        "WallBlock.shouldRaisePost honors the wall_post_override tag",
    );

    set_above(fixture, above, Block::STONE.default_state.id);
    assert_wall_north_south_post(
        fixture,
        position,
        NorthWall::Tall,
        SouthWall::Tall,
        false,
        "the center post decision is independent of arm coverage",
    );

    let mut bottom_slab = WhiteWoolSlabLikeProperties::default(&Block::OAK_SLAB);
    bottom_slab.r#type = SlabType::Bottom;
    set_above(fixture, above, bottom_slab.to_state_id(&Block::OAK_SLAB));
    assert_wall_north_south_post(
        fixture,
        position,
        NorthWall::Tall,
        SouthWall::Tall,
        false,
        "the bottom slab does not raise the center post",
    );

    let mut top_slab = WhiteWoolSlabLikeProperties::default(&Block::OAK_SLAB);
    top_slab.r#type = SlabType::Top;
    set_above(fixture, above, top_slab.to_state_id(&Block::OAK_SLAB));
    assert_wall_north_south_post(
        fixture,
        position,
        NorthWall::Low,
        SouthWall::Low,
        false,
        "WallBlock.shouldRaisePost remains independent for a top slab",
    );
}

fn check_asymmetric_coverage_and_corner_post(
    fixture: &Fixture,
    position: BlockPos,
    above: BlockPos,
) {
    fixture.set(
        above.offset(BlockDirection::North.to_offset()),
        Block::STONE.default_state.id,
    );
    set_above(fixture, above, Block::OAK_FENCE.default_state.id);
    assert_wall_north_south_post(
        fixture,
        position,
        NorthWall::Tall,
        SouthWall::Low,
        true,
        "the upper fence arm covers only the north test footprint and the post footprint",
    );

    set_above(fixture, above, Block::AIR.default_state.id);
    set_above(
        fixture,
        position.offset(BlockDirection::East.to_offset()),
        Block::STONE.default_state.id,
    );
    assert!(
        wall_state(fixture, position).up,
        "WallBlock.shouldRaisePost keeps a post at a corner"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wall_sides_become_tall_beneath_a_bottom_slab() {
    let f = Fixture::new();
    let position = BlockPos::new(8, 64, 8);
    f.set(position.down(), Block::STONE.default_state.id);
    f.set(
        position.offset(BlockDirection::North.to_offset()),
        Block::STONE.default_state.id,
    );
    f.set(
        position.offset(BlockDirection::South.to_offset()),
        Block::STONE.default_state.id,
    );
    place(
        &f,
        &Block::COBBLESTONE_WALL,
        &Item::COBBLESTONE_WALL,
        position,
    );

    let above = position.up();
    check_post_and_slab_controls(&f, position, above);
    check_asymmetric_coverage_and_corner_post(&f, position, above);

    assert!(f.world.level.shutdown().await.is_ok());
}
