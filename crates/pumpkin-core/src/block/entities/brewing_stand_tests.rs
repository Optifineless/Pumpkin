#![expect(clippy::unwrap_used, reason = "Real brewing regression assertions")]

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering::Relaxed},
};

use pumpkin_data::{
    data_component_impl::{
        BrewingFuelImpl, BrewingFuelProvider, CustomNameImpl, PotionContentsImpl, UseRemainderImpl,
    },
    item::Item,
    item_stack::ItemStack,
    potion::Potion,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
};
use pumpkin_util::text::TextComponent;

use super::brewing_stand_test_support::{
    BrewingFixture, assert_inventory_unlocked, potion, register,
};
use crate::plugin::api::events::{
    block::brewing_start::BrewingStartEvent,
    entity::item_spawn::ItemSpawnEvent,
    inventory::{brew::BrewEvent, brewing_stand_fuel::BrewingStandFuelEvent},
};

enum BrewingEventCalls {}
impl pumpkin_util::sync::LockClass for BrewingEventCalls {
    const NAME: &'static str = "brewing regression event calls";
    const LEVEL: u8 = 40;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_dragon_breath_final_ingredient_returns_bottle_after_real_ticks() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::SPLASH_POTION, &Potion::WATER),
        ItemStack::new(1, &Item::DRAGON_BREATH),
    );
    fixture.ticks(1);
    assert_eq!(fixture.stand.brew_time.load(Relaxed), 400);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 19);
    fixture.ticks(399);
    assert_eq!(fixture.stand.get_stack(0).item, &Item::SPLASH_POTION);
    assert_eq!(fixture.stand.get_stack(3).item, &Item::DRAGON_BREATH);
    fixture.ticks(1);
    let output = fixture.stand.get_stack(0);
    assert_eq!(output.item, &Item::LINGERING_POTION);
    assert_eq!(
        output.get_data_component::<PotionContentsImpl>(),
        potion(&Item::LINGERING_POTION, &Potion::WATER).get_data_component::<PotionContentsImpl>()
    );
    assert_eq!(fixture.stand.brew_time.load(Relaxed), 0);
    assert_eq!(fixture.stand.get_stack(3).item, &Item::GLASS_BOTTLE);
    assert_eq!(fixture.stand.get_stack(3).item_count, 1);
    assert!(fixture.dropped_items().is_empty());
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_stacked_ingredient_keeps_components_and_drops_one_crafting_bottle() {
    let fixture = BrewingFixture::new().await;
    let mut reagent = ItemStack::new(2, &Item::DRAGON_BREATH);
    reagent.set_data_component(CustomNameImpl {
        name: TextComponent::text("retained dragon breath"),
    });
    let mut unrelated_remainder = ItemStack::new(3, &Item::DIAMOND);
    unrelated_remainder.set_data_component(CustomNameImpl {
        name: TextComponent::text("not a crafting remainder"),
    });
    reagent.set_data_component(UseRemainderImpl {
        remainder: None,
        template: Some(Box::new(unrelated_remainder)),
    });
    let expected_remaining = reagent.copy_with_count(1);
    fixture.ingredients(potion(&Item::SPLASH_POTION, &Potion::WATER), reagent);
    let stand = fixture.stand.clone();
    register::<ItemSpawnEvent>(&fixture.host.server, move |_| {
        assert_inventory_unlocked(&stand);
    });
    fixture.ticks(401);
    assert_eq!(fixture.stand.get_stack(0).item, &Item::LINGERING_POTION);
    assert!(fixture.stand.get_stack(3).are_equal(&expected_remaining));
    let dropped = fixture.dropped_items();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].item, &Item::GLASS_BOTTLE);
    assert_eq!(dropped[0].item_count, 1);
    assert!(dropped[0].get_data_component::<CustomNameImpl>().is_none());
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_invalid_recipe_and_full_destinations_keep_ingredient_and_fuel() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::POTION, &Potion::WATER),
        ItemStack::new(1, &Item::DRAGON_BREATH),
    );
    let before = fixture.stand.get_stack(0);
    fixture.ticks(401);
    assert!(fixture.stand.get_stack(0).are_equal(&before));
    assert_eq!(fixture.stand.get_stack(3).item, &Item::DRAGON_BREATH);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 20);
    assert_eq!(fixture.stand.brew_time.load(Relaxed), 0);
    for slot in 0..3 {
        fixture
            .stand
            .set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    fixture
        .stand
        .set_stack(3, ItemStack::new(2, &Item::NETHER_WART));
    let player = fixture.open_menu();
    let offered = potion(&Item::SPLASH_POTION, &Potion::WATER);
    player.player.inventory.set_slot(9, offered.clone());
    let (sync_id, revision) = {
        let current = player.player.current_screen_handler.lock().unwrap().clone();
        let menu = current.lock().unwrap();
        (menu.sync_id(), menu.get_behaviour().revision.load(Relaxed))
    };
    player.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(i32::from(sync_id)),
            revision: VarInt(revision as i32),
            slot: 5,
            button: 0,
            mode: SlotActionType::QuickMove,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &fixture.host.server,
    );
    assert!(player.player.inventory.get_stack(9).are_equal(&offered));
    assert_eq!(player.player.inventory.get_stack(9).uid, offered.uid);
    fixture.ticks(401);
    for slot in 0..3 {
        assert_eq!(fixture.stand.get_stack(slot).item, &Item::STONE);
        assert_eq!(fixture.stand.get_stack(slot).item_count, 64);
    }
    assert_eq!(fixture.stand.get_stack(3).item_count, 2);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 20);
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    assert!(fixture.dropped_items().is_empty());
    assert!(player.player.inventory.get_stack(9).are_equal(&offered));
    player.player.close_handled_screen();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_fuel_start_and_brew_cancellation_preserve_event_order_and_stock() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::SPLASH_POTION, &Potion::WATER),
        ItemStack::new(2, &Item::DRAGON_BREATH),
    );
    let mode = Arc::new(AtomicU8::new(0));
    let calls = Arc::new(pumpkin_util::sync::Mutex::<_, BrewingEventCalls>::new(
        Vec::new(),
    ));
    let fuel_mode = mode.clone();
    let fuel_calls = calls.clone();
    let stand = fixture.stand.clone();
    register::<BrewingStandFuelEvent>(&fixture.host.server, move |event| {
        assert_inventory_unlocked(&stand);
        assert_eq!(event.fuel_power, 20);
        event.cancelled = fuel_mode.load(Relaxed) == 0;
        fuel_calls.lock().unwrap().push(if event.cancelled {
            "fuel cancelled"
        } else {
            "fuel"
        });
    });
    let start_mode = mode.clone();
    let start_calls = calls.clone();
    let stand = fixture.stand.clone();
    register::<BrewingStartEvent>(&fixture.host.server, move |event| {
        assert_inventory_unlocked(&stand);
        assert_eq!(event.brewing_time, 400);
        event.cancelled = start_mode.load(Relaxed) == 1;
        start_calls.lock().unwrap().push(if event.cancelled {
            "start cancelled"
        } else {
            "start"
        });
    });
    let brew_calls = calls.clone();
    let stand = fixture.stand.clone();
    register::<BrewEvent>(&fixture.host.server, move |event| {
        assert_inventory_unlocked(&stand);
        assert_eq!(event.fuel_level, 19);
        event.cancelled = true;
        brew_calls.lock().unwrap().push("brew cancelled");
    });
    fixture.ticks(1);
    assert_eq!(fixture.stand.get_stack(4).item_count, 2);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 0);
    mode.store(1, Relaxed);
    fixture.ticks(1);
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 20);
    assert_eq!(fixture.stand.brew_time.load(Relaxed), 0);
    mode.store(2, Relaxed);
    fixture.ticks(401);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 19);
    assert_eq!(fixture.stand.get_stack(0).item, &Item::SPLASH_POTION);
    assert_eq!(fixture.stand.get_stack(3).item_count, 2);
    assert!(fixture.dropped_items().is_empty());
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "fuel cancelled",
            "fuel",
            "start cancelled",
            "start",
            "brew cancelled"
        ]
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_brew_callback_replacement_does_not_pay_or_create_a_remainder() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::SPLASH_POTION, &Potion::WATER),
        ItemStack::new(1, &Item::DRAGON_BREATH),
    );
    let stand = fixture.stand.clone();
    register::<BrewEvent>(&fixture.host.server, move |_| {
        assert_inventory_unlocked(&stand);
        stand.set_stack(3, ItemStack::new(5, &Item::COBBLESTONE));
    });
    fixture.ticks(401);
    assert_eq!(fixture.stand.get_stack(0).item, &Item::SPLASH_POTION);
    assert_eq!(fixture.stand.get_stack(3).item, &Item::COBBLESTONE);
    assert_eq!(fixture.stand.get_stack(3).item_count, 5);
    assert!(fixture.dropped_items().is_empty());
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g02_fuel_and_start_callbacks_replacing_equal_stack_identity_reject_payment() {
    let fixture = BrewingFixture::new().await;
    fixture.ingredients(
        potion(&Item::POTION, &Potion::WATER),
        ItemStack::new(1, &Item::NETHER_WART),
    );
    let mut fuel = ItemStack::new(2, &Item::DRAGON_BREATH);
    fuel.set_data_component(BrewingFuelImpl {
        uses: BrewingFuelProvider::Constant(20),
        speed_multiplier: BrewingFuelProvider::Constant(1.0),
    });
    let original_fuel_uid = fuel.uid;
    let original_ingredient_uid = fixture.stand.get_stack(3).uid;
    fixture.stand.set_stack(4, fuel);
    let mode = Arc::new(AtomicU8::new(0));
    let fuel_mode = mode.clone();
    let stand = fixture.stand.clone();
    register::<BrewingStandFuelEvent>(&fixture.host.server, move |_| {
        assert_inventory_unlocked(&stand);
        if fuel_mode.load(Relaxed) == 0 {
            let stock = stand.get_stack(4);
            stand.set_stack(4, stock.copy_with_count(stock.item_count));
        }
    });
    let stand = fixture.stand.clone();
    register::<BrewingStartEvent>(&fixture.host.server, move |_| {
        assert_inventory_unlocked(&stand);
        let stock = stand.get_stack(3);
        stand.set_stack(3, stock.copy_with_count(stock.item_count));
    });
    fixture.ticks(1);
    assert_ne!(fixture.stand.get_stack(4).uid, original_fuel_uid);
    assert_eq!(fixture.stand.get_stack(4).item_count, 2);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 0);
    assert_eq!(fixture.stand.brew_time.load(Relaxed), 0);
    assert!(fixture.dropped_items().is_empty());
    mode.store(1, Relaxed);
    fixture.ticks(1);
    assert_ne!(fixture.stand.get_stack(3).uid, original_ingredient_uid);
    assert_eq!(fixture.stand.get_stack(3).item, &Item::NETHER_WART);
    assert_eq!(fixture.stand.get_stack(3).item_count, 1);
    assert_eq!(fixture.stand.get_stack(4).item_count, 1);
    assert_eq!(fixture.stand.fuel.load(Relaxed), 20);
    assert_eq!(fixture.stand.brew_time.load(Relaxed), 0);
    assert_eq!(
        fixture.dropped_items().len(),
        1,
        "only the admitted refill leaves a bottle"
    );
    assert_eq!(
        fixture
            .stand
            .get_stack(0)
            .get_data_component::<PotionContentsImpl>()
            .unwrap()
            .potion_id,
        Some(i32::from(Potion::WATER.id))
    );
    fixture.shutdown().await;
}
