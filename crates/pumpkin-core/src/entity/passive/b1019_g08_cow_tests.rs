#![expect(clippy::unwrap_used, reason = "Valid bounded gameplay fixtures")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use pumpkin_data::{
    data_component_impl::CustomNameImpl, entity::EntityType, item::Item, item_stack::ItemStack,
    packet::clientbound::play,
};
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_util::{GameMode, Hand, text::TextComponent};

use super::{b1019_g08_test_support::Fixture, cow::CowEntity};
use crate::{
    entity::{ageable::AgeableMob, mob::Mob},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::player_interact_at_entity::PlayerInteractAtEntityEvent,
            server::packet::PacketSentEvent,
        },
    },
    server::Server,
};

fn source_slot(hand: Hand) -> usize {
    if hand == Hand::Left {
        PlayerInventory::OFF_HAND_SLOT
    } else {
        0
    }
}

fn other_hand(hand: Hand) -> Hand {
    if hand == Hand::Left {
        Hand::Right
    } else {
        Hand::Left
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_handless_milking_uses_the_supplied_stack() {
    let fixture = Fixture::new();
    let target = fixture.mob(&EntityType::COW);
    let cow = target.cast_any().downcast_ref::<CowEntity>().unwrap();
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    let offhand = ItemStack::new(1, &Item::STONE);
    fixture.set_hand(Hand::Right, sword.clone());
    fixture.set_hand(Hand::Left, offhand.clone());
    let mut buckets = ItemStack::new(2, &Item::BUCKET);

    assert!(Mob::mob_interact(cow, &fixture.player, &mut buckets));

    assert_eq!(buckets.get_item(), &Item::BUCKET);
    assert_eq!(buckets.item_count, 1);
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
    assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
    assert!(fixture.hand(Hand::Right).are_equal(&sword));
    assert!(fixture.hand(Hand::Left).are_equal(&offhand));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_baby_refuses_and_adult_returns_milk_in_both_hands() {
    for hand in [Hand::Right, Hand::Left] {
        let fixture = Fixture::new();
        let target = fixture.mob(&EntityType::COW);
        let cow = target.cast_any().downcast_ref::<CowEntity>().unwrap();
        let bucket = ItemStack::new(1, &Item::BUCKET);
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        fixture.set_hand(hand, bucket.clone());
        fixture.set_hand(other_hand(hand), sword.clone());
        cow.set_baby(true);
        fixture.interact(target.as_ref(), hand);
        assert!(
            fixture.hand(hand).are_equal(&bucket),
            "baby refusal control"
        );
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 0);
        cow.set_baby(false);
        fixture.interact(target.as_ref(), hand);
        assert_eq!(fixture.hand(hand).get_item(), &Item::MILK_BUCKET);
        assert_eq!(fixture.hand(hand).item_count, 1);
        assert!(fixture.hand(other_hand(hand)).are_equal(&sword));
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_stacked_bucket_preserves_components_and_overflow() {
    for hand in [Hand::Right, Hand::Left] {
        for full in [false, true] {
            let fixture = Fixture::new();
            let target = fixture.mob(&EntityType::COW);
            if full {
                for slot in 0..PlayerInventory::MAIN_SIZE {
                    fixture
                        .player
                        .inventory()
                        .set_stack(slot, ItemStack::new(64, &Item::STONE));
                }
            }
            let mut buckets = ItemStack::new(2, &Item::BUCKET);
            buckets.set_data_component(CustomNameImpl {
                name: TextComponent::text("Retained buckets"),
            });
            let expected = buckets.copy_with_count(1);
            fixture.set_hand(hand, buckets);
            fixture.interact(target.as_ref(), hand);
            assert!(fixture.hand(hand).are_equal(&expected));
            assert_eq!(
                fixture.inventory_count(&Item::MILK_BUCKET),
                u32::from(!full),
            );
            assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), u32::from(full));
            fixture.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_creative_retains_bucket_and_suppresses_duplicate_milk() {
    for hand in [Hand::Right, Hand::Left] {
        let fixture = Fixture::new();
        let target = fixture.mob(&EntityType::COW);
        fixture.player.gamemode.store(GameMode::Creative);
        let bucket = ItemStack::new(2, &Item::BUCKET);
        fixture.set_hand(hand, bucket.clone());
        fixture.interact(target.as_ref(), hand);
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        fixture.interact(target.as_ref(), hand);
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        assert!(fixture.hand(hand).are_equal(&bucket));
        assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
        fixture.finish().await;
    }
}

struct CreativePublication {
    slot: usize,
    syncs: AtomicUsize,
    sounds: AtomicUsize,
}

impl EventHandler<PacketSentEvent> for CreativePublication {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == play::SET_PLAYER_INVENTORY.0 {
            assert_eq!(
                event.player.inventory().get_stack(self.slot).get_item(),
                &Item::BUCKET
            );
            let inventory = event.player.inventory();
            assert!(
                (0..inventory.size())
                    .any(|slot| { inventory.get_stack(slot).get_item() == &Item::MILK_BUCKET })
            );
            self.syncs.fetch_add(1, Ordering::Relaxed);
        } else if event.packet_id == play::SOUND.0 {
            assert!(self.syncs.load(Ordering::Relaxed) > 0);
            self.sounds.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_creative_new_milk_publishes_before_sound() {
    for hand in [Hand::Right, Hand::Left] {
        let fixture = Fixture::new();
        let target = fixture.mob(&EntityType::COW);
        fixture.player.gamemode.store(GameMode::Creative);
        fixture.set_hand(hand, ItemStack::new(1, &Item::BUCKET));
        let hook = Arc::new(CreativePublication {
            slot: source_slot(hand),
            syncs: AtomicUsize::new(0),
            sounds: AtomicUsize::new(0),
        });
        fixture
            .server
            .plugin_manager
            .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
        fixture.interact(target.as_ref(), hand);
        assert!(hook.syncs.load(Ordering::Relaxed) > 0);
        assert_eq!(hook.sounds.load(Ordering::Relaxed), 1);
        let syncs = hook.syncs.load(Ordering::Relaxed);
        fixture.interact(target.as_ref(), hand);
        assert_eq!(hook.syncs.load(Ordering::Relaxed), syncs);
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        fixture.finish().await;
    }
}

#[derive(Clone, Copy)]
enum BeforeEdit {
    Cancel,
    Replace,
    Remove,
    Select,
    Disconnect,
}

struct BeforeUse {
    edit: BeforeEdit,
    slot: usize,
    replacement: ItemStack,
    interactions: AtomicUsize,
    sounds: AtomicUsize,
}

impl EventHandler<PlayerInteractAtEntityEvent> for BeforeUse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractAtEntityEvent,
    ) -> BoxFuture<'a, ()> {
        self.interactions.fetch_add(1, Ordering::Relaxed);
        assert_eq!(event.hand, u8::from(self.slot == 40));
        match self.edit {
            BeforeEdit::Cancel => event.cancelled = true,
            BeforeEdit::Replace => event
                .player
                .inventory()
                .set_stack(self.slot, self.replacement.clone()),
            BeforeEdit::Remove => event
                .player
                .inventory()
                .set_stack(self.slot, ItemStack::EMPTY.clone()),
            BeforeEdit::Select => event.player.inventory().set_selected_slot(1),
            BeforeEdit::Disconnect => {
                let player = event.player.clone();
                let world = player.world();
                return Box::pin(async move {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        world.remove_player(&player, false),
                    )
                    .await
                    .unwrap();
                });
            }
        }
        Box::pin(async {})
    }
}

impl EventHandler<PacketSentEvent> for BeforeUse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == play::SOUND.0 {
            self.sounds.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_pre_use_cancellation_and_stale_sources_have_no_effects() {
    for hand in [Hand::Right, Hand::Left] {
        for edit in [
            BeforeEdit::Cancel,
            BeforeEdit::Replace,
            BeforeEdit::Remove,
            BeforeEdit::Disconnect,
        ] {
            refused_use(hand, edit).await;
        }
    }
    refused_use(Hand::Right, BeforeEdit::Select).await;
}

async fn refused_use(hand: Hand, edit: BeforeEdit) {
    let fixture = Fixture::new();
    let target = fixture.mob(&EntityType::COW);
    let bucket = ItemStack::new(1, &Item::BUCKET);
    fixture.set_hand(hand, bucket.clone());
    let hook = Arc::new(BeforeUse {
        edit,
        slot: source_slot(hand),
        replacement: ItemStack::new(1, &Item::BUCKET),
        interactions: AtomicUsize::new(0),
        sounds: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractAtEntityEvent, _>(hook.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    fixture.interact(target.as_ref(), hand);
    assert_eq!(hook.interactions.load(Ordering::Relaxed), 1);
    assert_eq!(hook.sounds.load(Ordering::Relaxed), 0);
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 0);
    assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
    let current = fixture.player.inventory().get_stack(source_slot(hand));
    match edit {
        BeforeEdit::Remove => assert!(current.is_empty()),
        BeforeEdit::Replace => assert_eq!(current.uid, hook.replacement.uid),
        BeforeEdit::Cancel | BeforeEdit::Select | BeforeEdit::Disconnect => {
            assert_eq!(current.uid, bucket.uid);
        }
    }
    if matches!(edit, BeforeEdit::Disconnect) {
        assert!(
            fixture
                .world
                .get_player_by_uuid(fixture.player.gameprofile.id)
                .is_none(),
        );
    }
    fixture.finish().await;
}

#[derive(Clone, Copy)]
enum AfterEdit {
    Replace,
    RestoreSource,
    Remove,
    Select,
}

struct AfterUse {
    edit: AfterEdit,
    slot: usize,
    packet_id: i32,
    calls: AtomicUsize,
    replacement: ItemStack,
}

impl EventHandler<PacketSentEvent> for AfterUse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == self.packet_id && self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            let inventory = event.player.inventory();
            assert_eq!(
                inventory.get_stack(self.slot).get_item(),
                &Item::MILK_BUCKET
            );
            let owner = inventory.slot_transaction(self.slot);
            assert!(std::thread::scope(|scope| {
                scope
                    .spawn(move || owner.try_lock().is_some())
                    .join()
                    .unwrap()
            }));
            match self.edit {
                AfterEdit::Replace | AfterEdit::RestoreSource => {
                    inventory.set_stack(self.slot, self.replacement.clone());
                }
                AfterEdit::Remove => inventory.set_stack(self.slot, ItemStack::EMPTY.clone()),
                AfterEdit::Select => inventory.set_selected_slot(1),
            }
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_commits_before_sync_and_sound_callbacks_preserving_live_edits() {
    for hand in [Hand::Right, Hand::Left] {
        for packet_id in [play::SET_PLAYER_INVENTORY.0, play::SOUND.0] {
            for edit in [
                AfterEdit::Replace,
                AfterEdit::RestoreSource,
                AfterEdit::Remove,
                AfterEdit::Select,
            ] {
                committed_callback(hand, packet_id, edit).await;
            }
        }
    }
}

async fn committed_callback(hand: Hand, packet_id: i32, edit: AfterEdit) {
    let fixture = Fixture::new();
    let target = fixture.mob(&EntityType::COW);
    let bucket = ItemStack::new(1, &Item::BUCKET);
    fixture.set_hand(hand, bucket.clone());
    let selected = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.player.inventory().set_stack(1, selected.clone());
    let hook = Arc::new(AfterUse {
        edit,
        slot: source_slot(hand),
        packet_id,
        calls: AtomicUsize::new(0),
        replacement: if matches!(edit, AfterEdit::RestoreSource) {
            bucket
        } else {
            ItemStack::new(1, &Item::DIAMOND)
        },
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    fixture.interact(target.as_ref(), hand);
    assert!(hook.calls.load(Ordering::Relaxed) > 0, "live callback ran");
    let current = fixture.player.inventory().get_stack(source_slot(hand));
    match edit {
        AfterEdit::Replace | AfterEdit::RestoreSource => {
            assert_eq!(current.uid, hook.replacement.uid);
        }
        AfterEdit::Remove => assert!(current.is_empty()),
        AfterEdit::Select => assert_eq!(current.get_item(), &Item::MILK_BUCKET),
    }
    assert!(fixture.player.inventory().get_stack(1).are_equal(&selected));
    assert_eq!(
        fixture.inventory_count(&Item::MILK_BUCKET),
        u32::from(matches!(edit, AfterEdit::Select)),
    );
    assert_eq!(
        fixture.inventory_count(&Item::BUCKET),
        u32::from(matches!(edit, AfterEdit::RestoreSource)),
    );
    assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
    fixture.finish().await;
}
