use crate::world::spawn_test_support::{Fixture, proto, publish};
use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::{DoubleBlockHalf, SmallDripleafLikeProperties, TallSeagrassLikeProperties},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

#[test]
fn aquatic_plant_water_checks_distinguish_full_falling_and_sources() {
    // FluidState.isFull accepts amount 8; SmallDripleafBlock requires a source instead.
    for (level, source, full) in [(0, true, true), (1, false, false), (8, false, true)] {
        let state = pumpkin_data::block_properties::WaterLikeProperties { level }
            .to_state_id(&Block::WATER)
            .to_state();
        assert_eq!(
            super::water_source(state),
            source,
            "source at level {level}"
        );
        assert_eq!(
            super::full_water(state),
            full,
            "full water at level {level}"
        );
    }
}

#[tokio::test]
async fn mismatched_double_plant_species_and_parts_do_not_stay_paired() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::DIRT));
    let lower = BlockPos::new(5, 64, 5);
    let mut props = TallSeagrassLikeProperties::default(&Block::SUNFLOWER);
    props.half = DoubleBlockHalf::Lower;
    chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::SUNFLOWER));
    props.half = DoubleBlockHalf::Upper;
    chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(&Block::LILAC));
    fixture.world.replace_with_state_for_neighbor_update(
        &lower,
        BlockDirection::Up,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(fixture.world.get_block(&lower).is_air());
    for upper in [false, true] {
        let mut props = SmallDripleafLikeProperties::default(&Block::SMALL_DRIPLEAF);
        props.half = if upper {
            DoubleBlockHalf::Upper
        } else {
            DoubleBlockHalf::Lower
        };
        chunk.set_block_absolute_y(5, 63, 5, Block::CLAY.default_state.id);
        chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::SMALL_DRIPLEAF));
        chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(&Block::SMALL_DRIPLEAF));
        fixture.world.replace_with_state_for_neighbor_update(
            &lower,
            if upper {
                BlockDirection::Down
            } else {
                BlockDirection::Up
            },
            BlockFlags::NOTIFY_ALL,
        );
        fixture.world.replace_with_state_for_neighbor_update(
            &lower.up(),
            BlockDirection::Down,
            BlockFlags::NOTIFY_ALL,
        );
        assert!(fixture.world.get_block(&lower).is_air());
        assert!(fixture.world.get_block(&lower.up()).is_air());
    }
    fixture.finish().await;
}

#[tokio::test]
async fn lower_double_flower_break_drops_once_and_upper_loot_is_empty() {
    for block in [
        &Block::SUNFLOWER,
        &Block::PEONY,
        &Block::ROSE_BUSH,
        &Block::LILAC,
        &Block::PITCHER_PLANT,
    ] {
        for creative in [false, true] {
            let fixture = crate::block::blocks::bed::test_support::PlayerFixture::new();
            publish(&fixture.world, proto(&Biome::PLAINS, &Block::DIRT));
            let lower = BlockPos::new(5, 64, 5);
            let mut props = TallSeagrassLikeProperties::default(block);
            props.half = DoubleBlockHalf::Lower;
            fixture
                .world
                .set_block_state(&lower, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
            let upper_state = fixture.world.get_block_state(&lower.up());
            let params = crate::world::loot::LootContextParameters {
                block_state: Some(upper_state),
                ..Default::default()
            };
            assert!(
                crate::block::block_drops(&fixture.world, block, &lower.up(), &params).is_empty()
            );
            if creative {
                crate::block::blocks::bed::test_support::creative_break_java(&fixture.world, lower);
            } else {
                fixture
                    .world
                    .break_block(&lower, None, BlockFlags::NOTIFY_ALL);
            }
            assert!(fixture.world.get_block(&lower.up()).is_air());
            let drops: i32 = fixture
                .world
                .entities
                .load()
                .iter()
                .filter_map(|entity| entity.get_item_entity())
                .map(|item| i32::from(item.get_item_stack().lock().unwrap().item_count))
                .sum();
            assert_eq!(drops, i32::from(!creative));
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn tall_seagrass_placement_builds_upper_half_and_loses_orphan() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let lower = BlockPos::new(5, 64, 5);
    chunk.set_block_absolute_y(5, 64, 5, Block::WATER.default_state.id);
    chunk.set_block_absolute_y(5, 65, 5, Block::WATER.default_state.id);
    let mut props = TallSeagrassLikeProperties::default(&Block::TALL_SEAGRASS);
    props.half = DoubleBlockHalf::Lower;
    fixture.world.set_block_state(
        &lower,
        props.to_state_id(&Block::TALL_SEAGRASS),
        BlockFlags::NOTIFY_ALL,
    );
    assert_eq!(fixture.world.get_block(&lower.up()), &Block::TALL_SEAGRASS);
    assert_eq!(
        TallSeagrassLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower.up()))
            .half,
        DoubleBlockHalf::Upper
    );
    fixture
        .world
        .break_block(&lower, None, BlockFlags::NOTIFY_ALL);
    assert_eq!(fixture.world.get_block(&lower), &Block::WATER);
    assert_eq!(fixture.world.get_block(&lower.up()), &Block::WATER);
    fixture.finish().await;
}

#[tokio::test]
async fn mature_pitcher_rejects_same_half_partner() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::FARMLAND));
    let lower = BlockPos::new(5, 64, 5);
    let mut props =
        pumpkin_data::block_properties::PitcherCropLikeProperties::default(&Block::PITCHER_CROP);
    props.age = 3;
    props.half = DoubleBlockHalf::Lower;
    chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::PITCHER_CROP));
    chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(&Block::PITCHER_CROP));
    fixture.world.replace_with_state_for_neighbor_update(
        &lower,
        BlockDirection::Up,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(fixture.world.get_block(&lower).is_air());
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_creative_upper_flower_break_suppresses_bottom_drop_in_both_editions() {
    use crate::block::blocks::bed::test_support::{self, PlayerFixture};
    for bedrock in [false, true] {
        let fixture = PlayerFixture::new();
        publish(&fixture.world, proto(&Biome::PLAINS, &Block::DIRT));
        let lower = BlockPos::new(5, 64, 5);
        fixture.world.set_block_state(
            &lower,
            Block::SUNFLOWER.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        let upper = lower.up();
        if bedrock {
            test_support::creative_break_bedrock(&fixture.world, upper).await;
        } else {
            crate::block::blocks::bed::test_support::creative_break_java(&fixture.world, upper);
        }
        assert!(fixture.world.get_block(&lower).is_air());
        assert!(fixture.world.get_block(&upper).is_air());
        assert!(
            !fixture
                .world
                .entities
                .load()
                .iter()
                .any(|entity| entity.get_item_entity().is_some())
        );
        fixture.finish().await;
    }
}
