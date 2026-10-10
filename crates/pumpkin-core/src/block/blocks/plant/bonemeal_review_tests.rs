use crate::{
    block::{
        BlockBehaviour, BonemealArgs, blocks::bed::test_support::PlayerFixture,
        registry::BlockActionResult,
    },
    item::{ItemBehaviour, items::bone_meal::BoneMealItem},
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::{
        DoubleBlockHalf, PitcherCropLikeProperties, SmallDripleafLikeProperties,
        TallSeagrassLikeProperties,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

fn apply(
    fixture: &PlayerFixture,
    client: &TestPlayer,
    pos: BlockPos,
    stack: &mut ItemStack,
) -> BlockActionResult {
    BoneMealItem.use_on_block(
        stack,
        &client.player,
        pos,
        BlockDirection::Up,
        Vector3::default(),
        fixture.world.get_block(&pos),
        &fixture.world.server.upgrade().unwrap(),
    )
}

#[tokio::test]
async fn obstructed_pitcher_bonemeal_preserves_bedrock_and_item() {
    let fixture = PlayerFixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::FARMLAND));
    let pos = BlockPos::new(5, 64, 5);
    let mut props = PitcherCropLikeProperties::default(&Block::PITCHER_CROP);
    props.half = DoubleBlockHalf::Lower;
    props.age = 2;
    chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::PITCHER_CROP));
    chunk.set_block_absolute_y(5, 65, 5, Block::BEDROCK.default_state.id);
    let client = TestPlayer::new(&fixture.world);
    let mut stack = ItemStack::new(4, &Item::BONE_MEAL);
    assert!(matches!(
        apply(&fixture, &client, pos, &mut stack),
        BlockActionResult::Pass
    ));
    assert_eq!(stack.item_count, 4);
    assert_eq!(fixture.world.get_block(&pos.up()), &Block::BEDROCK);
    super::crop::pitcher_crop::PitcherCropBlock.perform_bonemeal(BonemealArgs {
        world: &fixture.world,
        block: &Block::PITCHER_CROP,
        position: &pos,
        state_id: fixture.world.get_block_state_id(&pos),
    });
    assert_eq!(
        PitcherCropLikeProperties::from_state_id(fixture.world.get_block_state_id(&pos)).age,
        2
    );
    chunk.set_block_absolute_y(5, 65, 5, Block::AIR.default_state.id);
    assert!(matches!(
        apply(&fixture, &client, pos, &mut stack),
        BlockActionResult::Success
    ));
    assert_eq!(stack.item_count, 3);
    assert_eq!(fixture.world.get_block(&pos.up()), &Block::PITCHER_CROP);
    fixture.finish().await;
}

#[tokio::test]
async fn seagrass_bonemeal_dispatch_builds_both_halves() {
    let fixture = PlayerFixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    chunk.set_block_absolute_y(5, 64, 5, Block::SEAGRASS.default_state.id);
    chunk.set_block_absolute_y(5, 65, 5, Block::WATER.default_state.id);
    let client = TestPlayer::new(&fixture.world);
    let mut stack = ItemStack::new(2, &Item::BONE_MEAL);
    assert!(matches!(
        apply(&fixture, &client, pos, &mut stack),
        BlockActionResult::Success
    ));
    assert_eq!(stack.item_count, 1);
    for (pos, half) in [
        (pos, DoubleBlockHalf::Lower),
        (pos.up(), DoubleBlockHalf::Upper),
    ] {
        assert_eq!(fixture.world.get_block(&pos), &Block::TALL_SEAGRASS);
        assert_eq!(
            TallSeagrassLikeProperties::from_state_id(fixture.world.get_block_state_id(&pos)).half,
            half
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn tall_flower_bonemeal_dispatch_drops_one_flower_from_either_half() {
    for upper in [false, true] {
        let fixture = PlayerFixture::new();
        publish(&fixture.world, proto(&Biome::PLAINS, &Block::DIRT));
        let lower = BlockPos::new(5, 64, 5);
        fixture.world.set_block_state(
            &lower,
            Block::SUNFLOWER.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        let client = TestPlayer::new(&fixture.world);
        let mut stack = ItemStack::new(2, &Item::BONE_MEAL);
        assert!(matches!(
            apply(
                &fixture,
                &client,
                if upper { lower.up() } else { lower },
                &mut stack
            ),
            BlockActionResult::Success
        ));
        assert_eq!(stack.item_count, 1);
        let count: i32 = fixture
            .world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| i32::from(item.get_item_stack().lock().unwrap().item_count))
            .sum();
        assert_eq!(count, 1);
        assert_eq!(fixture.world.get_block(&lower), &Block::SUNFLOWER);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn small_dripleaf_bonemeal_dispatch_grows_waterlogged_big_dripleaf() {
    let fixture = PlayerFixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::CLAY));
    let lower = BlockPos::new(5, 64, 5);
    let mut props = SmallDripleafLikeProperties::default(&Block::SMALL_DRIPLEAF);
    props.waterlogged = true;
    props.half = DoubleBlockHalf::Lower;
    chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::SMALL_DRIPLEAF));
    props.half = DoubleBlockHalf::Upper;
    chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(&Block::SMALL_DRIPLEAF));
    let client = TestPlayer::new(&fixture.world);
    let mut stack = ItemStack::new(2, &Item::BONE_MEAL);
    assert!(matches!(
        apply(&fixture, &client, lower.up(), &mut stack),
        BlockActionResult::Success
    ));
    assert_eq!(stack.item_count, 1);
    assert_eq!(fixture.world.get_block(&lower), &Block::BIG_DRIPLEAF_STEM);
    assert!(
        pumpkin_data::block_properties::LadderLikeProperties::from_state_id(
            fixture.world.get_block_state_id(&lower)
        )
        .waterlogged
    );
    let leaf = (1..5)
        .map(|y| lower.offset(Vector3::new(0, y, 0)))
        .find(|pos| fixture.world.get_block(pos) == &Block::BIG_DRIPLEAF);
    assert!(leaf.is_some());
    fixture.finish().await;
}
