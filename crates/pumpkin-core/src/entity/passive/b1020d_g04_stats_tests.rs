#![expect(
    clippy::unwrap_used,
    reason = "Real Java interaction regressions use registered bounded fixtures"
)]

use super::{
    b1020d_g04_test_support::Fixture, cow::CowEntity, tamable::TamableAnimal, wolf::WolfEntity,
};
use crate::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_regain_health::EntityRegainHealthEvent,
            player::player_interact_entity_event::PlayerInteractEntityEvent,
        },
    },
    server::Server,
};
use crossbeam::atomic::AtomicCell;
use pumpkin_data::{
    data_component_impl::{BundleContentsImpl, CustomNameImpl},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::{GameMode, Hand, text::TextComponent};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering::Relaxed},
};

fn used(player: &Player, item: &Item) -> i32 {
    player.get_stat(StatisticCategory::Used, i32::from(item.id))
}

fn accepted_food_milk_and_dye(fixture: &Fixture, hand: Hand, at: bool, mode: GameMode) {
    fixture.player.gamemode.store(mode);
    let cow = fixture.mob(&EntityType::COW);
    let cow = cow.cast_any().downcast_ref::<CowEntity>().unwrap();
    fixture.set_hand(hand, ItemStack::new(2, &Item::WHEAT));
    let before = used(&fixture.player, &Item::WHEAT);
    fixture.interact(cow, hand, at);
    assert_eq!(cow.mob_entity.love_ticks.load(Relaxed), 600);
    assert_eq!(
        fixture.hand(hand).item_count,
        if mode == GameMode::Creative { 2 } else { 1 }
    );
    assert_eq!(
        used(&fixture.player, &Item::WHEAT),
        before,
        "accepted food earns no generic Used credit"
    );

    fixture.set_hand(hand, ItemStack::new(1, &Item::BUCKET));
    let before = used(&fixture.player, &Item::BUCKET);
    let milk_before = fixture.inventory_count(&Item::MILK_BUCKET);
    fixture.interact(cow, hand, at);
    if mode == GameMode::Creative {
        assert_eq!(fixture.hand(hand).item, &Item::BUCKET);
        assert!(fixture.inventory_count(&Item::MILK_BUCKET) >= milk_before.max(1));
    } else {
        assert_eq!(fixture.hand(hand).item, &Item::MILK_BUCKET);
    }
    assert_eq!(
        used(&fixture.player, &Item::BUCKET),
        before,
        "milk exchange earns no generic Used credit"
    );

    accepted_dye(fixture, hand, at, mode);
}

fn accepted_dye(fixture: &Fixture, hand: Hand, at: bool, mode: GameMode) {
    let wolf = fixture.mob(&EntityType::WOLF);
    let wolf = wolf.cast_any().downcast_ref::<WolfEntity>().unwrap();
    wolf.tame(fixture.player.gameprofile.id);
    wolf.set_collar_color(14);
    fixture.set_hand(hand, ItemStack::new(2, &Item::BLUE_DYE));
    let before = used(&fixture.player, &Item::BLUE_DYE);
    fixture.interact(wolf, hand, at);
    assert_eq!(wolf.get_collar_color(), 11);
    assert_eq!(
        fixture.hand(hand).item_count,
        if mode == GameMode::Creative { 2 } else { 1 }
    );
    assert_eq!(
        used(&fixture.player, &Item::BLUE_DYE),
        before,
        "collar mutation earns no generic Used credit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_entity_interaction_used_stats_are_item_owned() {
    let fixture = Fixture::new();
    for at in [false, true] {
        for hand in [Hand::Right, Hand::Left] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                accepted_food_milk_and_dye(&fixture, hand, at, mode);
            }
        }
    }
    fixture.finish().await;
}

struct BeforeInteraction {
    hand: Hand,
    edit: AtomicU8,
}

impl EventHandler<PlayerInteractEntityEvent> for BeforeInteraction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        match self.edit.load(Relaxed) {
            1 => event.cancelled = true,
            2 => event
                .player
                .inventory()
                .set_stack_in_hand(self.hand, ItemStack::new(2, &Item::WHEAT)),
            3 => event
                .player
                .inventory()
                .set_stack_in_hand(self.hand, ItemStack::EMPTY.clone()),
            4 => event.player.inventory().set_selected_slot(1),
            _ => {}
        }
        Box::pin(async {})
    }
}

fn rejected_food(fixture: &Fixture, hand: Hand, at: bool, control: &BeforeInteraction) {
    for edit in 1..=4 {
        fixture.player.inventory().set_selected_slot(0);
        fixture
            .player
            .inventory()
            .set_stack(1, ItemStack::new(1, &Item::DIAMOND));
        let cow = fixture.mob(&EntityType::COW);
        let cow = cow.cast_any().downcast_ref::<CowEntity>().unwrap();
        fixture.set_hand(hand, ItemStack::new(2, &Item::WHEAT));
        control.edit.store(edit, Relaxed);
        let before = used(&fixture.player, &Item::WHEAT);
        fixture.interact(cow, hand, at);
        let admitted = edit == 4 && hand == Hand::Left;
        assert_eq!(
            cow.mob_entity.love_ticks.load(Relaxed),
            if admitted { 600 } else { 0 }
        );
        let source = fixture
            .player
            .inventory()
            .get_stack(if hand == Hand::Left { 40 } else { 0 });
        assert_eq!(
            source.item_count,
            if edit == 3 {
                0
            } else if admitted {
                1
            } else {
                2
            }
        );
        assert_eq!(
            used(&fixture.player, &Item::WHEAT),
            before,
            "cancelled/stale source cannot earn generic credit"
        );
        assert_eq!(fixture.player.inventory().get_stack(1).item, &Item::DIAMOND);
    }
    control.edit.store(0, Relaxed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_refused_cancelled_and_stale_sources_earn_no_used_credit() {
    for hand in [Hand::Right, Hand::Left] {
        let fixture = Fixture::new();
        let control = Arc::new(BeforeInteraction {
            hand,
            edit: AtomicU8::new(0),
        });
        fixture
            .server
            .plugin_manager
            .register::<PlayerInteractEntityEvent, _>(control.clone(), EventPriority::Normal, true);
        for at in [false, true] {
            rejected_food(&fixture, hand, at, &control);
            fixture.player.inventory().set_selected_slot(0);
            let calf = fixture.mob(&EntityType::COW);
            calf.get_entity().age.store(-24000, Relaxed);
            let bucket = ItemStack::new(1, &Item::BUCKET);
            fixture.set_hand(hand, bucket.clone());
            fixture.interact(calf.as_ref(), hand, at);
            assert!(fixture.hand(hand).are_equal(&bucket));
            assert_eq!(used(&fixture.player, &Item::BUCKET), 0);
        }
        fixture.finish().await;
    }
}

fn bundle_award_control(fixture: &Fixture, hand: Hand) {
    let mut bundle = ItemStack::new(1, &Item::BUNDLE);
    bundle.set_data_component(BundleContentsImpl {
        items: vec![ItemStack::new(3, &Item::STONE)],
        selected_item: -1,
    });
    fixture.set_hand(hand, bundle.clone());
    let before = used(&fixture.player, &Item::BUNDLE);
    fixture.actor.client().handle_use_item(
        &fixture.player,
        &SUseItem {
            hand: VarInt(i32::from(hand == Hand::Left)),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &fixture.server,
    );
    assert_eq!(used(&fixture.player, &Item::BUNDLE), before);
    assert_eq!(
        *fixture.player.living_entity.active_hand.lock().unwrap(),
        Some(hand)
    );
    fixture
        .server
        .item_registry
        .on_use_tick(&bundle, &fixture.player, 200);
    assert_eq!(used(&fixture.player, &Item::BUNDLE), before + 1);
    assert!(
        fixture
            .hand(hand)
            .get_data_component::<BundleContentsImpl>()
            .unwrap()
            .items
            .is_empty()
    );
    fixture
        .server
        .item_registry
        .on_use_tick(&bundle, &fixture.player, 198);
    fixture
        .server
        .item_registry
        .on_use_tick(&bundle, &fixture.player, 188);
    assert_eq!(
        used(&fixture.player, &Item::BUNDLE),
        before + 1,
        "delay and empty bundle add no credit"
    );
    fixture.player.swap_item();
    assert!(
        fixture
            .player
            .living_entity
            .active_hand
            .lock()
            .unwrap()
            .is_none()
    );
}

fn fallback_control(fixture: &Fixture, hand: Hand, at: bool) {
    let cow = fixture.mob(&EntityType::COW);
    let mut name = ItemStack::new(2, &Item::NAME_TAG);
    name.set_data_component(CustomNameImpl {
        name: TextComponent::text("Item-owned fallback"),
    });
    fixture.set_hand(hand, name);
    fixture.interact(cow.as_ref(), hand, at);
    assert_eq!(
        fixture.hand(hand).item_count,
        1,
        "passing mob still reaches item fallback and writes back once"
    );
    assert_eq!(
        cow.get_entity()
            .custom_name
            .load()
            .as_ref()
            .as_ref()
            .unwrap()
            .clone()
            .get_text(),
        "Item-owned fallback"
    );
}

struct RestorePaidFood {
    player: Arc<Player>,
    restore: AtomicCell<Option<(Hand, ItemStack)>>,
}

impl EventHandler<EntityRegainHealthEvent> for RestorePaidFood {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut EntityRegainHealthEvent,
    ) -> BoxFuture<'a, ()> {
        let restore = self.restore.swap(None);
        if let Some((hand, original)) = restore {
            assert_eq!(
                self.player.inventory().get_stack_in_hand(hand).item_count,
                1,
                "healing callback observes the actual payment"
            );
            self.player.inventory().set_stack_in_hand(hand, original);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_payment_receipts_fallback_and_explicit_bundle_awards_survive() {
    let fixture = Fixture::new();
    let restore = Arc::new(RestorePaidFood {
        player: fixture.player.clone(),
        restore: AtomicCell::new(None),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntityRegainHealthEvent, _>(restore.clone(), EventPriority::Normal, true);
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            let wolf = fixture.mob(&EntityType::WOLF);
            let wolf = wolf.cast_any().downcast_ref::<WolfEntity>().unwrap();
            wolf.tame(fixture.player.gameprofile.id);
            wolf.mob_entity.living_entity.set_max_health(40.0);
            wolf.mob_entity.living_entity.set_health(1.0);
            let food = ItemStack::new(2, &Item::COOKED_BEEF);
            fixture.set_hand(hand, food.clone());
            restore.restore.store(Some((hand, food.clone())));
            fixture.interact(wolf, hand, at);
            assert_eq!(wolf.mob_entity.living_entity.health.load(), 17.0);
            let actual = fixture.hand(hand);
            assert!(
                actual.are_equal(&food),
                "committed receipt preserves callback-restored stock"
            );
            assert_eq!(actual.uid, food.uid);
            restore.restore.store(None);
            fallback_control(&fixture, hand, at);
        }
        bundle_award_control(&fixture, hand);
    }
    fixture.finish().await;
}
