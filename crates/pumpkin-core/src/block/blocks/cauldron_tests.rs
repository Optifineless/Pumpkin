//! Production-path regressions for Java 26.3 `CauldronInteractions` and the plugin veto contract.
use std::sync::atomic::Ordering;

use crate::{
    block::registry::BlockActionResult, item::items::glass_bottle::water_bottle,
    plugin::block::cauldron_level_change::CauldronChangeReason,
};
use pumpkin_data::{
    Block, BlockStateId,
    data_component_impl::{BannerPatternsImpl, DyedColorImpl},
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand};

#[path = "cauldron_test_support.rs"]
mod support;
use support::{Fixture, POS, dyed_armor, layered, named_shulker, other_hand, patterned_banner};

fn starting_cauldron_states() -> [(BlockStateId, i32); 8] {
    [
        (layered(&Block::WATER_CAULDRON, "1"), 1),
        (layered(&Block::WATER_CAULDRON, "2"), 2),
        (Block::CAULDRON.default_state.id, 0),
        (layered(&Block::WATER_CAULDRON, "3"), 3),
        (Block::LAVA_CAULDRON.default_state.id, 3),
        (layered(&Block::POWDER_SNOW_CAULDRON, "1"), 1),
        (layered(&Block::POWDER_SNOW_CAULDRON, "2"), 2),
        (layered(&Block::POWDER_SNOW_CAULDRON, "3"), 3),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filled_buckets_replace_every_cauldron_state_through_either_java_hand() {
    let mut fixture = Fixture::new();
    let fillings = [
        (&Item::WATER_BUCKET, layered(&Block::WATER_CAULDRON, "3")),
        (&Item::LAVA_BUCKET, Block::LAVA_CAULDRON.default_state.id),
        (
            &Item::POWDER_SNOW_BUCKET,
            layered(&Block::POWDER_SNOW_CAULDRON, "3"),
        ),
    ];
    for hand in Hand::all() {
        for (state, old_level) in starting_cauldron_states() {
            for (item, filled_state) in fillings {
                fixture.reset(state, hand, ItemStack::new(1, item));
                fixture.use_top(hand);
                assert_eq!(
                    fixture.world.get_block_state_id(&POS),
                    filled_state,
                    "filled bucket {} from state {state:?}",
                    item.registry_key
                );
                let inventory = fixture.client.player.inventory();
                assert!(
                    inventory
                        .get_stack_in_hand(hand)
                        .are_equal(&ItemStack::new(1, &Item::BUCKET))
                );
                assert_eq!(
                    inventory.get_stack_in_hand(other_hand(hand)).get_item(),
                    &Item::DIAMOND_SWORD
                );
                let stats = fixture.client.player.stats.lock().unwrap();
                assert_eq!(
                    stats.get(
                        StatisticCategory::Custom,
                        CustomStatistic::FillCauldron as i32
                    ),
                    1
                );
                assert_eq!(stats.get(StatisticCategory::Used, i32::from(item.id)), 1);
                drop(stats);
                assert_eq!(
                    *fixture.events.changes.lock().unwrap(),
                    [(state, old_level, 3, CauldronChangeReason::BucketEmpty)]
                );
                assert_eq!(
                    *fixture.events.game_events.lock().unwrap(),
                    [GameEvent::FluidPlace.name()]
                );
                assert_eq!(fixture.sound_count(), 1);
            }
        }
    }
    fixture.world.level.shutdown().await.unwrap();
}

fn cancellable_changes() -> Vec<(BlockStateId, ItemStack, i32, i32, CauldronChangeReason)> {
    use CauldronChangeReason::{BottleEmpty, BottleFill, BucketEmpty, BucketFill, Unknown};
    let empty = Block::CAULDRON.default_state.id;
    let water = |level| layered(&Block::WATER_CAULDRON, level);
    vec![
        (
            empty,
            ItemStack::new(1, &Item::WATER_BUCKET),
            0,
            3,
            BucketEmpty,
        ),
        (
            empty,
            ItemStack::new(1, &Item::LAVA_BUCKET),
            0,
            3,
            BucketEmpty,
        ),
        (
            empty,
            ItemStack::new(1, &Item::POWDER_SNOW_BUCKET),
            0,
            3,
            BucketEmpty,
        ),
        (
            water("1"),
            ItemStack::new(1, &Item::WATER_BUCKET),
            1,
            3,
            BucketEmpty,
        ),
        (
            water("3"),
            ItemStack::new(1, &Item::BUCKET),
            3,
            0,
            BucketFill,
        ),
        (
            Block::LAVA_CAULDRON.default_state.id,
            ItemStack::new(1, &Item::BUCKET),
            3,
            0,
            BucketFill,
        ),
        (
            layered(&Block::POWDER_SNOW_CAULDRON, "3"),
            ItemStack::new(1, &Item::BUCKET),
            3,
            0,
            BucketFill,
        ),
        (empty, water_bottle(), 0, 1, BottleEmpty),
        (water("2"), water_bottle(), 2, 3, BottleEmpty),
        (
            water("1"),
            ItemStack::new(1, &Item::GLASS_BOTTLE),
            1,
            0,
            BottleFill,
        ),
        (
            water("3"),
            ItemStack::new(1, &Item::GLASS_BOTTLE),
            3,
            2,
            BottleFill,
        ),
        (water("2"), named_shulker(), 2, 1, Unknown),
        (water("2"), patterned_banner(), 2, 1, Unknown),
        (water("2"), dyed_armor(), 2, 1, Unknown),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_precedes_every_item_block_stat_sound_and_game_event_change() {
    let mut fixture = Fixture::new();
    fixture.events.cancel.store(true, Ordering::Relaxed);
    for hand in Hand::all() {
        for (state, stack, old_level, new_level, reason) in cancellable_changes() {
            fixture.reset(state, hand, stack);
            let inventory = fixture.inventory();
            fixture.use_top(hand);
            assert_eq!(
                *fixture.events.changes.lock().unwrap(),
                [(state, old_level, new_level, reason)]
            );
            fixture.assert_unchanged(state, &inventory);
        }
    }
    fixture.client.player.gamemode.store(GameMode::Creative);
    let state = layered(&Block::WATER_CAULDRON, "3");
    fixture.reset(state, Hand::Left, named_shulker());
    let inventory = fixture.inventory();
    fixture.use_top(Hand::Left);
    fixture.assert_unchanged(state, &inventory);
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submerged_lava_and_snow_use_consumes_without_filling_or_falling_through() {
    let mut fixture = Fixture::new();
    let water_above = [
        Block::WATER.default_state.id,
        Block::WATER
            .from_properties(&[("level", "5")])
            .to_state_id(&Block::WATER),
        Block::OAK_SLAB
            .from_properties(&[("waterlogged", "true")])
            .to_state_id(&Block::OAK_SLAB),
    ];
    for above in water_above {
        for state in [
            Block::CAULDRON.default_state.id,
            layered(&Block::WATER_CAULDRON, "2"),
            Block::LAVA_CAULDRON.default_state.id,
            layered(&Block::POWDER_SNOW_CAULDRON, "2"),
        ] {
            for item in [&Item::LAVA_BUCKET, &Item::POWDER_SNOW_BUCKET] {
                fixture.reset(state, Hand::Left, ItemStack::new(1, item));
                fixture.put(POS.up(), above);
                fixture.clear_effects();
                let inventory = fixture.inventory();
                assert!(matches!(
                    fixture.block_result(Hand::Left),
                    BlockActionResult::Consume
                ));
                fixture.use_top(Hand::Left);
                fixture.assert_unchanged(state, &inventory);
                assert!(fixture.events.changes.lock().unwrap().is_empty());
                assert_eq!(fixture.world.get_block_state_id(&POS.up()), above);
            }
        }
    }
    // Water buckets remain accepted underwater; the other filled-bucket dry controls are above.
    fixture.reset(
        Block::CAULDRON.default_state.id,
        Hand::Left,
        ItemStack::new(1, &Item::WATER_BUCKET),
    );
    fixture.put(POS.up(), Block::WATER.default_state.id);
    fixture.use_top(Hand::Left);
    assert_eq!(
        fixture.world.get_block_state_id(&POS),
        layered(&Block::WATER_CAULDRON, "3")
    );
    assert_eq!(
        fixture.client.player.inventory().off_hand_item().get_item(),
        &Item::BUCKET
    );
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bucket_and_bottle_withdrawal_preserves_stacked_inputs_and_records_exact_effects() {
    let mut fixture = Fixture::new();
    for (state, output) in [
        (layered(&Block::WATER_CAULDRON, "3"), &Item::WATER_BUCKET),
        (Block::LAVA_CAULDRON.default_state.id, &Item::LAVA_BUCKET),
        (
            layered(&Block::POWDER_SNOW_CAULDRON, "3"),
            &Item::POWDER_SNOW_BUCKET,
        ),
    ] {
        fixture.reset(state, Hand::Left, ItemStack::new(2, &Item::BUCKET));
        fixture.use_top(Hand::Left);
        assert_eq!(
            fixture.world.get_block_state_id(&POS),
            Block::CAULDRON.default_state.id
        );
        assert_eq!(fixture.count(&Item::BUCKET), 1);
        assert_eq!(fixture.count(output), 1);
        assert_eq!(
            *fixture.events.changes.lock().unwrap(),
            [(state, 3, 0, CauldronChangeReason::BucketFill)]
        );
        assert_eq!(
            *fixture.events.game_events.lock().unwrap(),
            [GameEvent::FluidPickup.name()]
        );
        assert_eq!(fixture.sound_count(), 1);
    }
    for (level, new_state) in [
        ("1", Block::CAULDRON.default_state.id),
        ("3", layered(&Block::WATER_CAULDRON, "2")),
    ] {
        fixture.reset(
            layered(&Block::WATER_CAULDRON, level),
            Hand::Left,
            ItemStack::new(2, &Item::GLASS_BOTTLE),
        );
        fixture.use_top(Hand::Left);
        assert_eq!(fixture.world.get_block_state_id(&POS), new_state);
        assert_eq!(fixture.count(&Item::GLASS_BOTTLE), 1);
        let output = water_bottle();
        assert_eq!(
            fixture
                .inventory()
                .iter()
                .filter(|stack| stack.are_equal(&output))
                .count(),
            1
        );
        assert_eq!(
            *fixture.events.game_events.lock().unwrap(),
            [GameEvent::BlockChange.name(), GameEvent::FluidPickup.name()]
        );
        assert_eq!(fixture.sound_count(), 1);
        assert_eq!(
            fixture
                .client
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
            1
        );
    }
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn water_bottles_fill_one_level_and_ineligible_containers_do_not_mutate() {
    let mut fixture = Fixture::new();
    for (state, new_state) in [
        (
            Block::CAULDRON.default_state.id,
            layered(&Block::WATER_CAULDRON, "1"),
        ),
        (
            layered(&Block::WATER_CAULDRON, "1"),
            layered(&Block::WATER_CAULDRON, "2"),
        ),
        (
            layered(&Block::WATER_CAULDRON, "2"),
            layered(&Block::WATER_CAULDRON, "3"),
        ),
    ] {
        fixture.reset(state, Hand::Right, water_bottle());
        fixture.use_top(Hand::Right);
        assert_eq!(fixture.world.get_block_state_id(&POS), new_state);
        assert_eq!(fixture.count(&Item::POTION), 0);
        assert_eq!(fixture.count(&Item::GLASS_BOTTLE), 1);
        assert_eq!(
            *fixture.events.game_events.lock().unwrap(),
            [GameEvent::FluidPlace.name()]
        );
        assert_eq!(fixture.sound_count(), 1);
        assert_eq!(
            fixture
                .client
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Used, i32::from(Item::POTION.id)),
            1
        );
    }
    for (state, input) in [
        (
            layered(&Block::WATER_CAULDRON, "1"),
            ItemStack::new(1, &Item::BUCKET),
        ),
        (
            layered(&Block::WATER_CAULDRON, "2"),
            ItemStack::new(1, &Item::BUCKET),
        ),
        (
            layered(&Block::POWDER_SNOW_CAULDRON, "2"),
            ItemStack::new(1, &Item::BUCKET),
        ),
        (layered(&Block::WATER_CAULDRON, "3"), water_bottle()),
        (
            Block::CAULDRON.default_state.id,
            ItemStack::new(1, &Item::POTION),
        ),
    ] {
        fixture.reset(state, Hand::Right, input);
        let inventory = fixture.inventory();
        assert!(matches!(
            fixture.block_result(Hand::Right),
            BlockActionResult::PassToDefaultBlockAction
        ));
        fixture.use_top(Hand::Right);
        fixture.assert_unchanged(state, &inventory);
        assert!(fixture.events.changes.lock().unwrap().is_empty());
    }
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creative_shulker_washing_keeps_components_and_produces_each_clean_copy() {
    let mut fixture = Fixture::new();
    let colored = named_shulker();
    let mut cleaned = colored.copy_with_count(1);
    cleaned.item = &Item::SHULKER_BOX;
    for mode in [GameMode::Survival, GameMode::Creative] {
        fixture.client.player.gamemode.store(mode);
        fixture.reset(
            layered(&Block::WATER_CAULDRON, "3"),
            Hand::Left,
            colored.clone(),
        );
        fixture.use_top(Hand::Left);
        assert_eq!(
            fixture.world.get_block_state_id(&POS),
            layered(&Block::WATER_CAULDRON, "2")
        );
        let expected_hand = if mode == GameMode::Creative {
            &colored
        } else {
            &cleaned
        };
        assert!(
            fixture
                .client
                .player
                .inventory()
                .off_hand_item()
                .are_equal(expected_hand)
        );
        assert_eq!(
            fixture
                .inventory()
                .iter()
                .filter(|stack| stack.are_equal(&cleaned))
                .count(),
            1
        );
        assert_eq!(
            fixture.count(&Item::RED_SHULKER_BOX),
            u32::from(mode == GameMode::Creative)
        );
        assert_eq!(fixture.sound_count(), 0);
    }
    assert_eq!(
        *fixture.events.game_events.lock().unwrap(),
        [GameEvent::BlockChange.name()]
    );
    fixture.use_top(Hand::Left);
    assert!(
        fixture
            .client
            .player
            .inventory()
            .off_hand_item()
            .are_equal(&colored)
    );
    assert_eq!(
        fixture
            .inventory()
            .iter()
            .filter(|stack| stack.are_equal(&cleaned))
            .count(),
        2
    );
    assert_eq!(
        fixture.world.get_block_state_id(&POS),
        layered(&Block::WATER_CAULDRON, "1")
    );
    assert_eq!(
        fixture.client.player.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::CleanShulkerBox as i32
        ),
        2
    );
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creative_shulker_washing_drops_the_copy_when_inventory_is_full() {
    let mut fixture = Fixture::new();
    fixture.client.player.gamemode.store(GameMode::Creative);
    let colored = named_shulker();
    fixture.reset(
        layered(&Block::WATER_CAULDRON, "1"),
        Hand::Left,
        colored.clone(),
    );
    let inventory = fixture.client.player.inventory();
    for slot in 0..36 {
        inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    fixture.use_top(Hand::Left);
    let mut cleaned = colored.clone();
    cleaned.item = &Item::SHULKER_BOX;
    assert!(inventory.off_hand_item().are_equal(&colored));
    assert_eq!(fixture.count(&Item::SHULKER_BOX), 0);
    let drops = fixture.drops();
    assert_eq!(drops.len(), 1);
    assert!(drops[0].are_equal(&cleaned));
    assert_eq!(
        fixture.world.get_block_state_id(&POS),
        Block::CAULDRON.default_state.id
    );
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn banner_and_armor_washing_remove_only_the_targeted_component_data() {
    let mut fixture = Fixture::new();
    let banner = patterned_banner();
    let mut cleaned = banner.copy_with_count(1);
    cleaned
        .get_data_component_mut::<BannerPatternsImpl>()
        .unwrap()
        .layers
        .pop();
    fixture.reset(
        layered(&Block::WATER_CAULDRON, "2"),
        Hand::Left,
        banner.copy_with_count(2),
    );
    fixture.use_top(Hand::Left);
    assert!(
        fixture
            .client
            .player
            .inventory()
            .off_hand_item()
            .are_equal(&banner.copy_with_count(1))
    );
    assert_eq!(
        fixture
            .inventory()
            .iter()
            .filter(|stack| stack.are_equal(&cleaned))
            .count(),
        1
    );
    assert_eq!(
        fixture.world.get_block_state_id(&POS),
        layered(&Block::WATER_CAULDRON, "1")
    );
    assert_eq!(
        fixture.client.player.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::CleanBanner as i32
        ),
        1
    );
    fixture.reset(
        layered(&Block::WATER_CAULDRON, "1"),
        Hand::Right,
        dyed_armor(),
    );
    fixture.use_top(Hand::Right);
    assert_eq!(
        fixture.world.get_block_state_id(&POS),
        Block::CAULDRON.default_state.id
    );
    assert!(
        fixture
            .client
            .player
            .inventory()
            .held_item()
            .get_data_component::<DyedColorImpl>()
            .is_none()
    );
    assert_eq!(
        fixture.client.player.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::CleanArmor as i32
        ),
        1
    );
    assert_eq!(
        *fixture.events.game_events.lock().unwrap(),
        [GameEvent::BlockChange.name()]
    );
    assert_eq!(fixture.sound_count(), 0);
    fixture.world.level.shutdown().await.unwrap();
}
