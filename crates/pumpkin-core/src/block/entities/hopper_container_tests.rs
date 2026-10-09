use super::*;
use crate::{
    block::entities::{chest::ChestBlockEntity, trapped_chest::TrappedChestBlockEntity},
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{ChestLikeProperties, ChestType, HorizontalFacing},
};
use pumpkin_world::world::BlockFlags;

fn chest_pair(
    world: &Arc<World>,
    right: BlockPos,
) -> (Arc<ChestBlockEntity>, Arc<ChestBlockEntity>) {
    publish(world, proto(&Biome::PLAINS, &Block::STONE));
    let left = right.offset(pumpkin_util::math::vector3::Vector3::new(-1, 0, 0));
    for (position, half) in [(right, ChestType::Right), (left, ChestType::Left)] {
        let mut props = ChestLikeProperties::from_state_id(Block::CHEST.default_state.id);
        props.facing = HorizontalFacing::North;
        props.r#type = half;
        world.set_block_state(
            &position,
            props.to_state_id(&Block::CHEST),
            BlockFlags::FORCE_STATE,
        );
    }
    let first = Arc::new(ChestBlockEntity::new(right));
    let second = Arc::new(ChestBlockEntity::new(left));
    world.add_block_entity(first.clone());
    world.add_block_entity(second.clone());
    (first, second)
}

#[tokio::test]
async fn hopper_pulls_from_connected_chest_half() {
    let fixture = Fixture::new();
    let hopper = HopperBlockEntity::new(BlockPos::new(8, 64, 8), FacingHopper::Down);
    let (first, second) = chest_pair(&fixture.world, hopper.position.up());
    second.set_stack(0, ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND));
    assert!(hopper.suck_in_items(&fixture.world));
    assert_eq!(hopper.get_stack(0).item_count, 1);
    assert!(first.is_empty());
    assert_eq!(second.get_stack(0).item_count, 1);
    fixture.finish().await;
}

#[tokio::test]
async fn hopper_pushes_past_full_first_chest_half() {
    let fixture = Fixture::new();
    let hopper = HopperBlockEntity::new(BlockPos::new(8, 64, 7), FacingHopper::South);
    let (first, second) = chest_pair(&fixture.world, BlockPos::new(8, 64, 8));
    for slot in 0..first.size() {
        first.set_stack(slot, ItemStack::new(64, &pumpkin_data::item::Item::STONE));
    }
    hopper.set_stack(0, ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND));
    assert!(hopper.eject_items(&fixture.world, FacingHopper::South));
    assert_eq!(hopper.get_stack(0).item_count, 1);
    assert_eq!(second.get_stack(0).item_count, 1);
    assert_eq!(first.get_stack(0).item_count, 64);
    fixture.finish().await;
}

#[tokio::test]
async fn malformed_chest_partner_is_not_combined() {
    let fixture = Fixture::new();
    let right = BlockPos::new(8, 64, 8);
    let (first, second) = chest_pair(&fixture.world, right);
    let partner = second.position;
    assert_eq!(
        get_container_at(&fixture.world, &partner).unwrap().size(),
        54
    );
    for (block, half, facing) in [
        (Block::CHEST, ChestType::Right, HorizontalFacing::North),
        (Block::CHEST, ChestType::Single, HorizontalFacing::North),
        (Block::CHEST, ChestType::Left, HorizontalFacing::South),
        (
            Block::TRAPPED_CHEST,
            ChestType::Left,
            HorizontalFacing::North,
        ),
    ] {
        let mut props = ChestLikeProperties::from_state_id(block.default_state.id);
        props.r#type = half;
        props.facing = facing;
        fixture
            .world
            .set_block_state(&partner, props.to_state_id(&block), BlockFlags::FORCE_STATE);
        if block == Block::TRAPPED_CHEST {
            fixture
                .world
                .add_block_entity(Arc::new(TrappedChestBlockEntity::new(partner)));
        }
        assert_eq!(
            get_container_at(&fixture.world, &right).unwrap().size(),
            first.size()
        );
    }
    let mut props = ChestLikeProperties::from_state_id(Block::CHEST.default_state.id);
    props.facing = HorizontalFacing::North;
    props.r#type = ChestType::Left;
    fixture.world.set_block_state(
        &partner,
        props.to_state_id(&Block::CHEST),
        BlockFlags::FORCE_STATE,
    );
    fixture.world.add_block_entity(Arc::new(
        crate::block::entities::barrel::BarrelBlockEntity::new(partner),
    ));
    assert_eq!(
        get_container_at(&fixture.world, &right).unwrap().size(),
        first.size()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn failed_double_chest_transfer_preserves_source() {
    let fixture = Fixture::new();
    let hopper = HopperBlockEntity::new(BlockPos::new(8, 64, 7), FacingHopper::South);
    let (first, second) = chest_pair(&fixture.world, BlockPos::new(8, 64, 8));
    for chest in [first, second] {
        for slot in 0..chest.size() {
            chest.set_stack(slot, ItemStack::new(64, &pumpkin_data::item::Item::STONE));
        }
    }
    let source = ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND);
    hopper.set_stack(0, source.clone());
    assert!(!hopper.eject_items(&fixture.world, FacingHopper::South));
    assert!(hopper.get_stack(0).are_equal(&source));
    assert!(fixture.world.entities.load().is_empty());
    fixture.finish().await;
}
