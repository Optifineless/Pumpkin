#![expect(
    clippy::unwrap_used,
    reason = "Captured brewing menu and NBT regressions"
)]

use std::{borrow::Cow, sync::atomic::Ordering::Relaxed};

use pumpkin_data::{
    data_component_impl::{
        BrewingFuelImpl, BrewingFuelProvider, CustomNameImpl, PotionContentsImpl,
    },
    item::Item,
    item_stack::ItemStack,
    potion::Potion,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::text::TextComponent;

use super::{
    BlockEntity, PropertyDelegate, block_entity_from_nbt,
    brewing_stand::BrewingStandBlockEntity,
    brewing_stand_test_support::{BrewingFixture, potion, property_packets},
};

fn assert_properties(stand: &BrewingStandBlockEntity, expected: [i32; 4]) {
    assert_eq!(stand.get_properties_size(), 4);
    for (index, value) in expected.into_iter().enumerate() {
        assert_eq!(stand.get_property(index as i32), value);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_registered_java_menu_publishes_four_properties_and_countdown() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::POTION, &Potion::WATER),
        ItemStack::new(1, &Item::NETHER_WART),
    );
    fixture.ticks(1);
    assert_properties(&fixture.stand, [400, 19, 400, 20]);
    let mut player = fixture.open_menu();
    assert_eq!(
        property_packets(&mut player),
        [(0, 400), (1, 19), (2, 400), (3, 20)]
    );
    fixture.ticks(1);
    player.player.tick(&fixture.host.server);
    assert_eq!(property_packets(&mut player), [(0, 399)]);
    assert_properties(&fixture.stand, [399, 19, 400, 20]);
    assert_eq!(
        fixture.stand.get_stack(4).item_count,
        1,
        "remaining fuel keeps the second powder"
    );
    fixture.ticks(399);
    player.player.tick(&fixture.host.server);
    assert_eq!(property_packets(&mut player), [(0, 0)]);
    assert_eq!(fixture.stand.get_stack(0).item, &Item::POTION);
    assert_eq!(
        fixture
            .stand
            .get_stack(0)
            .get_data_component::<PotionContentsImpl>()
            .unwrap()
            .potion_id,
        Some(i32::from(Potion::AWKWARD.id))
    );
    assert!(fixture.stand.get_stack(3).is_empty());
    assert_properties(&fixture.stand, [0, 19, 400, 20]);
    player.player.close_handled_screen();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_component_fuel_samples_custom_providers_and_publishes_changed_totals() {
    let fixture = BrewingFixture::new().await;
    fixture
        .host
        .server
        .datapack_manager
        .insert_context_int_provider("test:brewing_uses".into(), NbtTag::Int(7));
    fixture
        .host
        .server
        .datapack_manager
        .insert_context_float_provider("test:brewing_speed".into(), NbtTag::Float(2.5));
    let mut fuel = ItemStack::new(2, &Item::DRAGON_BREATH);
    fuel.set_data_component(CustomNameImpl {
        name: TextComponent::text("custom brewing fuel"),
    });
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Id(Cow::Borrowed("test:brewing_uses")),
        speed_multiplier: BrewingFuelProvider::Id(Cow::Borrowed("test:brewing_speed")),
    });
    let expected_remainder = fuel.copy_with_count(1);
    fixture.ingredients(
        potion(&Item::POTION, &Potion::WATER),
        ItemStack::new(1, &Item::NETHER_WART),
    );
    fixture.stand.set_stack(4, fuel);
    let mut player = fixture.open_menu();
    assert_eq!(
        property_packets(&mut player),
        [(0, 0), (1, 0), (2, 0), (3, 0)]
    );
    fixture.ticks(1);
    player.player.tick(&fixture.host.server);
    assert_eq!(
        property_packets(&mut player),
        [(0, 160), (1, 6), (2, 160), (3, 7)]
    );
    assert_properties(&fixture.stand, [160, 6, 160, 7]);
    fixture.ticks(160);
    player.player.tick(&fixture.host.server);
    assert_eq!(property_packets(&mut player), [(0, 0)]);
    assert!(fixture.stand.get_stack(4).are_equal(&expected_remainder));
    let dropped = fixture.dropped_items();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].item, &Item::GLASS_BOTTLE);
    assert_eq!(dropped[0].item_count, 1);
    assert_eq!(
        fixture
            .stand
            .get_stack(0)
            .get_data_component::<PotionContentsImpl>()
            .unwrap()
            .potion_id,
        Some(i32::from(Potion::AWKWARD.id))
    );
    player.player.close_handled_screen();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_missing_fuel_providers_use_zero_uses_and_unit_speed_defaults() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::POTION, &Potion::WATER),
        ItemStack::new(1, &Item::NETHER_WART),
    );
    let mut fuel = ItemStack::new(2, &Item::DIAMOND);
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Id(Cow::Borrowed("test:missing_uses")),
        speed_multiplier: BrewingFuelProvider::Constant(2.0),
    });
    fixture.stand.set_stack(4, fuel);
    fixture.ticks(1);
    assert_properties(&fixture.stand, [0, 0, 0, 0]);
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    assert_eq!(fixture.stand.get_stack(3).item_count, 1);
    let mut player = fixture.open_menu();
    assert_eq!(
        property_packets(&mut player),
        [(0, 0), (1, 0), (2, 0), (3, 0)]
    );
    fixture
        .host
        .server
        .datapack_manager
        .insert_context_int_provider("minecraft:7".into(), NbtTag::Int(0));
    let mut fuel = ItemStack::new(2, &Item::DIAMOND);
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Id(Cow::Borrowed("7")),
        speed_multiplier: BrewingFuelProvider::Constant(2.0),
    });
    fixture.stand.set_stack(4, fuel);
    fixture.ticks(1);
    player.player.tick(&fixture.host.server);
    assert_properties(&fixture.stand, [0, 0, 0, 0]);
    assert!(property_packets(&mut player).is_empty());
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    assert_eq!(fixture.stand.get_stack(3).item_count, 1);
    let mut overflow = NbtCompound::new();
    overflow.put_string("type", "minecraft:sum".into());
    overflow.put_list("inputs", vec![NbtTag::Int(i32::MAX), NbtTag::Int(1)]);
    fixture
        .host
        .server
        .datapack_manager
        .insert_context_int_provider("test:overflow_uses".into(), NbtTag::Compound(overflow));
    let mut fuel = ItemStack::new(2, &Item::DIAMOND);
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Id(Cow::Borrowed("test:overflow_uses")),
        speed_multiplier: BrewingFuelProvider::Constant(2.0),
    });
    fixture.stand.set_stack(4, fuel);
    fixture.ticks(1);
    player.player.tick(&fixture.host.server);
    assert_properties(&fixture.stand, [0, 0, 0, 0]);
    assert!(property_packets(&mut player).is_empty());
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    assert_eq!(fixture.stand.get_stack(3).item_count, 1);
    let mut fuel = ItemStack::new(2, &Item::DIAMOND);
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Constant(7),
        speed_multiplier: BrewingFuelProvider::Id(Cow::Borrowed("test:missing_speed")),
    });
    fixture.stand.set_stack(4, fuel);
    fixture.ticks(1);
    player.player.tick(&fixture.host.server);
    assert_properties(&fixture.stand, [400, 6, 400, 7]);
    assert_eq!(
        property_packets(&mut player),
        [(0, 400), (1, 6), (2, 400), (3, 7)]
    );
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    player.player.close_handled_screen();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_partial_custom_brew_saves_int_totals_speed_and_resumes_after_reload() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::POTION, &Potion::WATER),
        ItemStack::new(1, &Item::NETHER_WART),
    );
    let mut fuel = ItemStack::new(2, &Item::BLAZE_POWDER);
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Constant(7),
        speed_multiplier: BrewingFuelProvider::Constant(2.5),
    });
    fixture.stand.set_stack(4, fuel);
    fixture.ticks(42);
    assert_properties(&fixture.stand, [119, 6, 160, 7]);
    let mut saved = NbtCompound::new();
    fixture.stand.write_internal(&mut saved);
    assert_eq!(saved.get_int("BrewTime"), Some(119));
    assert_eq!(saved.get_int("Fuel"), Some(6));
    assert_eq!(saved.get_int("total_brew_time"), Some(160));
    assert_eq!(saved.get_int("total_fuel"), Some(7));
    assert_eq!(saved.get_float("speed_multiplier"), Some(2.5));
    let restored = block_entity_from_nbt(&saved).unwrap();
    fixture.world.add_block_entity(restored.clone());
    let stand = restored
        .as_any()
        .downcast_ref::<BrewingStandBlockEntity>()
        .unwrap();
    assert_properties(stand, [119, 6, 160, 7]);
    let mut player = fixture.open_menu();
    assert_eq!(
        property_packets(&mut player),
        [(0, 119), (1, 6), (2, 160), (3, 7)]
    );
    for _ in 0..118 {
        restored.tick(&fixture.world);
    }
    assert_eq!(
        stand
            .get_stack(0)
            .get_data_component::<PotionContentsImpl>()
            .unwrap()
            .potion_id,
        Some(i32::from(Potion::WATER.id))
    );
    assert_eq!(stand.brew_time.load(Relaxed), 1);
    restored.tick(&fixture.world);
    player.player.tick(&fixture.host.server);
    assert_eq!(property_packets(&mut player), [(0, 0)]);
    assert_properties(stand, [0, 6, 160, 7]);
    assert_eq!(
        stand
            .get_stack(0)
            .get_data_component::<PotionContentsImpl>()
            .unwrap()
            .potion_id,
        Some(i32::from(Potion::AWKWARD.id))
    );
    assert_eq!(stand.get_stack(4).item_count, 1);
    player.player.close_handled_screen();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_vanilla_int_and_legacy_short_byte_partial_brews_keep_remaining_time() {
    let fixture = BrewingFixture::new().await;
    for legacy in [false, true] {
        let source = BrewingStandBlockEntity::new(super::brewing_stand_test_support::POSITION);
        source.set_stack(0, potion(&Item::SPLASH_POTION, &Potion::WATER));
        source.set_stack(3, ItemStack::new(1, &Item::DRAGON_BREATH));
        let mut saved = NbtCompound::new();
        source.write_internal(&mut saved);
        if legacy {
            saved.put_short("BrewTime", 5);
            saved.put_byte("Fuel", 3);
            saved.child_tags.remove("total_brew_time");
            saved.child_tags.remove("total_fuel");
            saved.child_tags.remove("speed_multiplier");
        } else {
            saved.put_int("BrewTime", 5);
            saved.put_int("Fuel", 3);
            saved.put_int("total_brew_time", 80);
            saved.put_int("total_fuel", 9);
            saved.put_float("speed_multiplier", 5.0);
        }
        let restored = block_entity_from_nbt(&saved).unwrap();
        fixture.world.add_block_entity(restored.clone());
        let stand = restored
            .as_any()
            .downcast_ref::<BrewingStandBlockEntity>()
            .unwrap();
        let expected = if legacy {
            [5, 3, 400, 20]
        } else {
            [5, 3, 80, 9]
        };
        assert_properties(stand, expected);
        let mut player = fixture.open_menu();
        let expected_packets: Vec<_> = expected
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index as i16, value as i16))
            .collect();
        assert_eq!(property_packets(&mut player), expected_packets);
        restored.tick(&fixture.world);
        player.player.tick(&fixture.host.server);
        assert_eq!(property_packets(&mut player), [(0, 4)]);
        for _ in 0..4 {
            restored.tick(&fixture.world);
        }
        assert_eq!(stand.brew_time.load(Relaxed), 0);
        assert_eq!(stand.fuel.load(Relaxed), 3);
        assert_eq!(stand.get_stack(0).item, &Item::LINGERING_POTION);
        assert_eq!(stand.get_stack(3).item, &Item::GLASS_BOTTLE);
        player.player.close_handled_screen();
    }
    fixture.shutdown().await;
}
