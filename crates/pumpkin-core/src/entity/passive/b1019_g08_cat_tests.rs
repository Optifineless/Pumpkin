#![expect(
    clippy::unwrap_used,
    reason = "Valid bounded Java interaction fixtures"
)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    data_component::DataComponent, data_component_impl::FoodImpl, entity::EntityType, item::Item,
    item_stack::ItemStack, statistic::StatisticCategory,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand};

use crate::{
    entity::{
        EntityBase,
        mob::Mob,
        passive::{b1019_g08_test_support::Fixture, cat::CatEntity},
        player::Player,
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_regain_health::EntityRegainHealthEvent,
            player::player_interact_entity_event::PlayerInteractEntityEvent,
        },
    },
    server::Server,
};

fn owned_cat(fixture: &Fixture) -> Arc<dyn EntityBase> {
    let entity = fixture.mob(&EntityType::CAT);
    let cat = as_cat(entity.as_ref());
    cat.set_tame(true, Some(fixture.player.gameprofile.id));
    cat.set_sitting(false);
    let living = &cat.mob_entity.living_entity;
    living.set_health(living.get_max_health());
    entity
}

fn as_cat(entity: &dyn EntityBase) -> &CatEntity {
    entity.cast_any().downcast_ref::<CatEntity>().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_handless_dyeing_uses_the_supplied_stack() {
    let fixture = Fixture::new();
    let entity = owned_cat(&fixture);
    let cat = as_cat(entity.as_ref());
    cat.set_collar_color(14);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    let offhand = ItemStack::new(1, &Item::STONE);
    fixture.set_hand(Hand::Right, sword.clone());
    fixture.set_hand(Hand::Left, offhand.clone());
    let mut dye = ItemStack::new(2, &Item::BLUE_DYE);

    assert!(Mob::mob_interact(cat, &fixture.player, &mut dye));

    assert_eq!(cat.get_collar_color(), 11);
    assert_eq!(dye.item_count, 1);
    assert!(fixture.hand(Hand::Right).are_equal(&sword));
    assert!(fixture.hand(Hand::Left).are_equal(&offhand));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_parent_feeding_precedes_owner_sitting_in_both_hands() {
    let fixture = Fixture::new();
    let entity = owned_cat(&fixture);
    let cat = as_cat(entity.as_ref());
    let living = &cat.mob_entity.living_entity;
    for hand in [Hand::Right, Hand::Left] {
        cat.mob_entity.reset_love_ticks();
        cat.set_sitting(false);
        living.set_health(living.get_max_health() - 2.0);
        fixture.set_hand(hand, ItemStack::new(2, &Item::COD));
        fixture.interact(cat, hand);
        assert_eq!(living.health.load(), living.get_max_health());
        assert_eq!(fixture.hand(hand).item_count, 1);
        assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 0);

        fixture.interact(cat, hand);
        assert_eq!(
            cat.mob_entity.love_ticks.load(Relaxed),
            600,
            "healthy owned cat must enter love before the sitting fallback"
        );
        assert!(fixture.hand(hand).is_empty());
        assert!(!cat.is_sitting());

        fixture.set_hand(hand, ItemStack::EMPTY.clone());
        fixture.interact(cat, hand);
        assert!(cat.is_sitting(), "empty owner hand still toggles sitting");
        fixture.interact(cat, hand);
        assert!(!cat.is_sitting());
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_baby_food_and_nonowner_parent_controls() {
    let fixture = Fixture::new();
    let entity = owned_cat(&fixture);
    let cat = as_cat(entity.as_ref());
    for hand in [Hand::Right, Hand::Left] {
        cat.get_entity().set_age(-24000);
        fixture.set_hand(hand, ItemStack::new(1, &Item::SALMON));
        fixture.interact(cat, hand);
        assert_eq!(cat.get_entity().age.load(Relaxed), -21600);
        assert!(fixture.hand(hand).is_empty());
        assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 0);
        assert!(!cat.is_sitting());
    }

    cat.get_entity().set_age(0);
    cat.set_tame(true, Some(uuid::Uuid::new_v4()));
    for hand in [Hand::Right, Hand::Left] {
        cat.mob_entity.reset_love_ticks();
        cat.mob_entity.living_entity.set_health(1.0);
        fixture.set_hand(hand, ItemStack::new(2, &Item::COD));
        fixture.interact(cat, hand);
        assert_eq!(cat.mob_entity.living_entity.health.load(), 1.0);
        assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 600);
        assert_eq!(fixture.hand(hand).item_count, 1);
        assert!(!cat.is_sitting(), "nonowner food uses the Animal parent");
        fixture.set_hand(hand, ItemStack::new(1, &Item::BLUE_DYE));
        fixture.interact(cat, hand);
        assert_eq!(cat.get_collar_color(), 14);
        assert_eq!(fixture.hand(hand).item_count, 1);
        fixture.set_hand(hand, ItemStack::EMPTY.clone());
        fixture.interact(cat, hand);
        assert!(!cat.is_sitting());
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_effective_healing_and_owner_dye_controls() {
    let fixture = Fixture::new();
    let entity = owned_cat(&fixture);
    let cat = as_cat(entity.as_ref());
    for hand in [Hand::Right, Hand::Left] {
        let mut modified_food = ItemStack::new(1, &Item::COD);
        modified_food.set_data_component(FoodImpl {
            nutrition: 3,
            saturation: 0.0,
            can_always_eat: false,
        });
        let mut absent_food = ItemStack::new(1, &Item::COD);
        absent_food.remove_data_component(DataComponent::Food);
        for (food, expected) in [(modified_food, 4.0), (absent_food, 2.0)] {
            cat.mob_entity.living_entity.set_health(1.0);
            fixture.set_hand(hand, food);
            fixture.interact(cat, hand);
            assert_eq!(cat.mob_entity.living_entity.health.load(), expected);
            assert!(fixture.hand(hand).is_empty());
            assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 0);
            assert!(!cat.is_sitting());
        }
        cat.set_collar_color(14);
        fixture.set_hand(hand, ItemStack::new(2, &Item::BLUE_DYE));
        fixture.interact(cat, hand);
        assert_eq!(cat.get_collar_color(), 11);
        assert_eq!(fixture.hand(hand).item_count, 1);
        assert!(!cat.is_sitting());
    }
    fixture.player.gamemode.store(GameMode::Creative);
    cat.mob_entity.living_entity.set_health(1.0);
    let original = ItemStack::new(1, &Item::COD);
    fixture.set_hand(Hand::Left, original.clone());
    fixture.interact(cat, Hand::Left);
    assert_eq!(cat.mob_entity.living_entity.health.load(), 3.0);
    assert!(fixture.hand(Hand::Left).are_equal(&original));
    fixture.finish().await;
}

struct HealingCallback {
    player: Arc<Player>,
    source: AtomicUsize,
    cancel: AtomicBool,
    replace: AtomicBool,
    restore: Mutex<Option<ItemStack>>,
    seen: Mutex<Vec<(ItemStack, f32)>>,
}

impl EventHandler<EntityRegainHealthEvent> for HealingCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityRegainHealthEvent,
    ) -> BoxFuture<'a, ()> {
        let inventory = self.player.inventory();
        let source = self.source.load(Relaxed);
        self.seen
            .lock()
            .unwrap()
            .push((inventory.get_stack(source), event.amount));
        let restore = self.restore.lock().unwrap().clone();
        if let Some(stack) = restore {
            inventory.set_stack(source, stack);
        } else if self.replace.load(Relaxed) {
            inventory.set_stack(source, ItemStack::new(1, &Item::DIAMOND));
            inventory.set_selected_slot(1);
        }
        event.cancelled = self.cancel.load(Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_heal_callback_sees_payment_and_preserves_live_replacements() {
    let fixture = Fixture::new();
    let entity = owned_cat(&fixture);
    let cat = as_cat(entity.as_ref());
    let control = Arc::new(HealingCallback {
        player: fixture.player.clone(),
        source: AtomicUsize::new(0),
        cancel: AtomicBool::new(false),
        replace: AtomicBool::new(false),
        restore: Mutex::default(),
        seen: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntityRegainHealthEvent, _>(control.clone(), EventPriority::Normal, true);
    for hand in [Hand::Right, Hand::Left] {
        let source = if hand == Hand::Right { 0 } else { 40 };
        control.source.store(source, Relaxed);
        for cancelled in [false, true] {
            control.cancel.store(cancelled, Relaxed);
            fixture.player.inventory().set_selected_slot(0);
            cat.mob_entity.living_entity.set_health(1.0);
            fixture.set_hand(hand, ItemStack::new(2, &Item::COD));
            fixture.interact(cat, hand);
            assert_eq!(
                cat.mob_entity.living_entity.health.load(),
                if cancelled { 1.0 } else { 3.0 }
            );
            assert_eq!(fixture.hand(hand).item_count, 1);
        }
        control.cancel.store(false, Relaxed);
        control.replace.store(true, Relaxed);
        fixture.player.inventory().set_selected_slot(0);
        fixture
            .player
            .inventory()
            .set_stack(1, ItemStack::new(1, &Item::GOLD_INGOT));
        cat.mob_entity.living_entity.set_health(1.0);
        fixture.set_hand(hand, ItemStack::new(2, &Item::COD));
        fixture.interact(cat, hand);
        assert_eq!(cat.mob_entity.living_entity.health.load(), 3.0);
        assert_eq!(
            fixture.player.inventory().get_stack(source).item,
            &Item::DIAMOND
        );
        assert_eq!(
            fixture.player.inventory().get_stack(1).item,
            &Item::GOLD_INGOT
        );
        assert_eq!(fixture.player.inventory().get_selected_slot(), 1);
        control.replace.store(false, Relaxed);

        fixture.player.inventory().set_selected_slot(0);
        cat.mob_entity.living_entity.set_health(1.0);
        let restored = ItemStack::new(2, &Item::COD);
        *control.restore.lock().unwrap() = Some(restored.clone());
        fixture.set_hand(hand, restored.clone());
        let used_before = fixture
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::COD.id));
        fixture.interact(cat, hand);
        let actual = fixture.player.inventory().get_stack(source);
        assert_eq!(cat.mob_entity.living_entity.health.load(), 3.0);
        assert!(actual.are_equal(&restored), "callback restoration wins");
        assert_eq!(actual.uid, restored.uid);
        assert_eq!(
            fixture
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Used, i32::from(Item::COD.id)),
            used_before,
            "committed food payment does not award a generic item-use statistic"
        );
        *control.restore.lock().unwrap() = None;
    }
    let seen = control.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 8);
    for (stack, healing) in seen {
        assert_eq!((stack.item, stack.item_count), (&Item::COD, 1));
        assert_eq!(healing, 2.0);
    }
    fixture.finish().await;
}

struct BeforeInteraction {
    player: Arc<Player>,
    source: AtomicUsize,
    mode: AtomicU8,
    calls: AtomicUsize,
}

impl EventHandler<PlayerInteractEntityEvent> for BeforeInteraction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Relaxed);
        let inventory = self.player.inventory();
        match self.mode.load(Relaxed) {
            0 => event.cancelled = true,
            1 => inventory.set_stack(self.source.load(Relaxed), ItemStack::new(2, &Item::COD)),
            2 => inventory.set_stack(self.source.load(Relaxed), ItemStack::EMPTY.clone()),
            _ => inventory.set_selected_slot(1),
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_rejects_cancelled_replaced_removed_or_reselected_admission() {
    let fixture = Fixture::new();
    let entity = owned_cat(&fixture);
    let cat = as_cat(entity.as_ref());
    let control = Arc::new(BeforeInteraction {
        player: fixture.player.clone(),
        source: AtomicUsize::new(0),
        mode: AtomicU8::new(0),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEntityEvent, _>(control.clone(), EventPriority::Normal, true);
    for hand in [Hand::Right, Hand::Left] {
        let source = if hand == Hand::Right { 0 } else { 40 };
        control.source.store(source, Relaxed);
        for mode in 0..3 {
            control.mode.store(mode, Relaxed);
            let before = ItemStack::new(2, &Item::COD);
            fixture.set_hand(hand, before.clone());
            fixture.interact(cat, hand);
            assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 0);
            assert!(
                !cat.is_sitting(),
                "rejected admission has no sitting effect"
            );
            let actual = fixture.hand(hand);
            if mode == 2 {
                assert!(actual.is_empty());
            } else {
                assert!(actual.are_equal(&before));
                if mode == 1 {
                    assert_ne!(actual.uid, before.uid, "fresh equal replacement survives");
                }
            }
        }
    }
    control.source.store(0, Relaxed);
    control.mode.store(3, Relaxed);
    fixture.set_hand(Hand::Right, ItemStack::new(2, &Item::COD));
    fixture
        .player
        .inventory()
        .set_stack(1, ItemStack::new(1, &Item::GOLD_INGOT));
    fixture.interact(cat, Hand::Right);
    assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 0);
    assert!(!cat.is_sitting());
    assert_eq!(fixture.player.inventory().get_stack(0).item_count, 2);
    assert_eq!(fixture.hand(Hand::Right).item, &Item::GOLD_INGOT);
    assert_eq!(control.calls.load(Relaxed), 7);
    fixture.finish().await;
}
