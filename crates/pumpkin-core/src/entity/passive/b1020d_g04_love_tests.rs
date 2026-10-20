#![expect(
    clippy::unwrap_used,
    reason = "Valid bounded Java interaction fixtures"
)]

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering::Relaxed},
};

use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;

use super::{
    b1020d_g04_test_support::{Fixture, feedback},
    cow::CowEntity,
};
use crate::{
    entity::EntityBase,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_interact_entity_event::PlayerInteractEntityEvent,
    },
    server::Server,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_love_status_has_no_duplicate_heart_packet() {
    let mut fixture = Fixture::new();
    let entity = fixture.mob(&EntityType::COW);
    let cow = entity.cast_any().downcast_ref::<CowEntity>().unwrap();
    let id = entity.get_entity().entity_id;
    for at in [false, true] {
        for hand in [Hand::Right, Hand::Left] {
            cow.mob_entity.reset_love_ticks();
            fixture.set_hand(hand, ItemStack::new(2, &Item::WHEAT));
            fixture.drain_packets();
            fixture.interact(entity.as_ref(), hand, at);
            assert_eq!(
                fixture.hand(hand).item_count,
                1,
                "one successful food payment"
            );
            assert_eq!(cow.mob_entity.love_ticks.load(Relaxed), 600);
            for observer in &mut fixture.observers {
                let (statuses, particles) = feedback(&observer.take_packets(), id);
                assert_eq!(statuses, 1, "one client love-status expansion");
                assert!(
                    particles.is_empty(),
                    "no duplicate immediate Heart packet: {particles:?}"
                );
            }
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_love_periodic_hearts_remain_visible_to_packet_observers() {
    let mut fixture = Fixture::new();
    let entity = fixture.mob(&EntityType::COW);
    let cow = entity.cast_any().downcast_ref::<CowEntity>().unwrap();
    cow.mob_entity.set_no_ai(true);
    cow.mob_entity
        .set_love_ticks(600, Some(fixture.player.gameprofile.id));
    fixture.drain_packets();
    EntityBase::tick(cow, entity.as_ref(), &fixture.server);
    for observer in &mut fixture.observers {
        let (statuses, particles) =
            feedback(&observer.take_packets(), entity.get_entity().entity_id);
        assert_eq!(
            statuses, 0,
            "periodic particles do not repeat the entry status"
        );
        assert_eq!(particles, [1], "real mob tick retains its legitimate Heart");
    }
    fixture.finish().await;
}

struct FeedingCallback {
    mode: AtomicU8,
    hand: AtomicU8,
}

impl EventHandler<PlayerInteractEntityEvent> for FeedingCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        let inventory = event.player.inventory();
        let source = if self.hand.load(Relaxed) == 0 { 0 } else { 40 };
        match self.mode.load(Relaxed) {
            1 => event.cancelled = true,
            2 => inventory.set_stack(source, ItemStack::new(2, &Item::WHEAT)),
            3 => inventory.set_stack(source, ItemStack::EMPTY.clone()),
            4 => inventory.set_selected_slot(1),
            _ => {}
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_love_refused_cancelled_and_stale_feeding_has_no_burst() {
    let mut fixture = Fixture::new();
    let entity = fixture.mob(&EntityType::COW);
    let cow = entity.cast_any().downcast_ref::<CowEntity>().unwrap();
    let control = Arc::new(FeedingCallback {
        mode: AtomicU8::new(0),
        hand: AtomicU8::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEntityEvent, _>(control.clone(), EventPriority::Normal, true);
    for at in [false, true] {
        for hand in [Hand::Right, Hand::Left] {
            control.hand.store(u8::from(hand == Hand::Left), Relaxed);
            for mode in 0..=4 {
                if mode == 4 && hand == Hand::Left {
                    continue;
                }
                control.mode.store(mode, Relaxed);
                fixture.player.inventory().set_selected_slot(0);
                cow.mob_entity.reset_love_ticks();
                let before = ItemStack::new(
                    2,
                    if mode == 0 {
                        &Item::DIAMOND
                    } else {
                        &Item::WHEAT
                    },
                );
                fixture.set_hand(hand, before.clone());
                fixture.drain_packets();
                fixture.interact(entity.as_ref(), hand, at);
                assert_eq!(cow.mob_entity.love_ticks.load(Relaxed), 0);
                for observer in &mut fixture.observers {
                    assert_eq!(
                        feedback(&observer.take_packets(), entity.get_entity().entity_id),
                        (0, vec![])
                    );
                }
                let actual = fixture
                    .player
                    .inventory()
                    .get_stack(if hand == Hand::Right { 0 } else { 40 });
                if mode == 3 {
                    assert!(actual.is_empty(), "callback removal is retained");
                } else {
                    assert!(actual.are_equal(&before), "refusal never charges stock");
                    if mode == 2 {
                        assert_ne!(
                            actual.uid, before.uid,
                            "equal callback replacement survives"
                        );
                    }
                }
            }
        }
    }
    fixture.finish().await;
}
