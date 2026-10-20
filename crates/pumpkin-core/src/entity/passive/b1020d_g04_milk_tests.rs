#![expect(
    clippy::unwrap_used,
    reason = "Bounded registered interaction fixtures"
)]

use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{CustomDataImpl, DataComponentImpl},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    packet::clientbound::play,
    sound::{Sound, SoundCategory},
};
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{GameMode, Hand, math::vector3::Vector3, version::JavaMinecraftVersion};

use super::{b1020d_g04_test_support::Fixture, cow::CowEntity};
use crate::{
    entity::{ageable::AgeableMob, player::Player},
    net::java::sound_test_support::decode_sounds,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::{
                player_interact_at_entity::PlayerInteractAtEntityEvent,
                player_interact_entity_event::PlayerInteractEntityEvent,
            },
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

fn reset(fixture: &mut Fixture) {
    let inventory = fixture.player.inventory();
    inventory.set_selected_slot(0);
    for slot in 0..inventory.size() {
        inventory.set_stack(slot, ItemStack::EMPTY.clone());
    }
    fixture.player.gamemode.store(GameMode::Survival);
    fixture.drain_packets();
}

fn assert_owner_free(player: &Player, slot: usize) {
    let inventory = player.inventory();
    let owner = inventory.slot_transaction(slot);
    std::thread::scope(|scope| {
        assert!(
            scope
                .spawn(move || owner.try_lock().is_some())
                .join()
                .unwrap(),
            "milk callback retained the source owner"
        );
    });
}

struct MilkPublication {
    enabled: AtomicBool,
    actor: Arc<Player>,
    observer: uuid::Uuid,
    slot: usize,
    original: ItemStack,
    sounds: AtomicUsize,
    inventory: AtomicUsize,
}

impl EventHandler<PacketSentEvent> for MilkPublication {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Box::pin(async {});
        }
        if event.packet_id == play::SOUND.0 && event.player.gameprofile.id == self.observer {
            assert_owner_free(&self.actor, self.slot);
            let inventory = self.actor.inventory();
            let source = inventory.get_stack(self.slot);
            assert_eq!(
                source.uid, self.original.uid,
                "milk sound must publish before exchanging the original bucket"
            );
            assert!(
                source.are_equal(&self.original),
                "milk sound must publish while the original bucket remains"
            );
            assert!(
                (0..inventory.size())
                    .all(|slot| inventory.get_stack(slot).get_item() != &Item::MILK_BUCKET),
                "milk sound must precede the result exchange"
            );
            assert_eq!(self.inventory.load(Ordering::Relaxed), 0);
            self.sounds.fetch_add(1, Ordering::Relaxed);
        } else if event.packet_id == play::SET_PLAYER_INVENTORY.0
            && event.player.gameprofile.id == self.actor.gameprofile.id
        {
            assert_owner_free(&self.actor, self.slot);
            assert_eq!(
                self.sounds.load(Ordering::Relaxed),
                1,
                "milk inventory publication must follow sound publication"
            );
            self.inventory.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

fn observe_publication(
    fixture: &Fixture,
    hand: Hand,
    original: &ItemStack,
) -> Arc<MilkPublication> {
    fixture
        .actor
        .client()
        .version
        .store(JavaMinecraftVersion::V_1_21_2);
    fixture.observers[0]
        .client()
        .version
        .store(JavaMinecraftVersion::V_1_21_2);
    let hook = Arc::new(MilkPublication {
        enabled: AtomicBool::new(true),
        actor: fixture.player.clone(),
        observer: fixture.observers[0].player.gameprofile.id,
        slot: source_slot(hand),
        original: original.clone(),
        sounds: AtomicUsize::new(0),
        inventory: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    hook
}

fn successful_milk(fixture: &mut Fixture, hand: Hand, at: bool, creative: bool) {
    reset(fixture);
    let target = fixture.mob(&EntityType::COW);
    fixture.player.gamemode.store(if creative {
        GameMode::Creative
    } else {
        GameMode::Survival
    });
    let bucket = ItemStack::new(1, &Item::BUCKET);
    let unrelated = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.set_hand(hand, bucket.clone());
    fixture.set_hand(other_hand(hand), unrelated.clone());
    fixture.drain_packets();
    let hook = observe_publication(fixture, hand, &bucket);
    fixture.interact(target.as_ref(), hand, at);
    assert_eq!(hook.sounds.load(Ordering::Relaxed), 1);
    assert!(hook.inventory.load(Ordering::Relaxed) > 0);
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
    if creative {
        assert!(fixture.hand(hand).are_equal(&bucket));
    } else {
        assert_eq!(fixture.hand(hand).get_item(), &Item::MILK_BUCKET);
    }
    assert!(fixture.hand(other_hand(hand)).are_equal(&unrelated));
    assert!(
        decode_sounds(&fixture.actor.take_packets()).is_empty(),
        "the milking actor must be excluded from the server sound"
    );
    let position = fixture.player.position();
    let expected = Vector3::new(
        (position.x * 8.0) as i32,
        (position.y * 8.0) as i32,
        (position.z * 8.0) as i32,
    );
    for observer in &mut fixture.observers {
        let sounds = decode_sounds(&observer.take_packets());
        assert_eq!(sounds.len(), 1, "each nearby observer hears milk once");
        assert_eq!(sounds[0].sound_id, Sound::EntityCowMilk as u16);
        assert_eq!(sounds[0].category, SoundCategory::Players as i32);
        assert_eq!(
            sounds[0].position, expected,
            "milk sound originates at player"
        );
        assert_eq!(sounds[0].volume, 1.0);
        assert_eq!(sounds[0].pitch, 1.0);
    }
    hook.enabled.store(false, Ordering::Relaxed);
}

fn calf_refuses(fixture: &mut Fixture, hand: Hand, at: bool) {
    reset(fixture);
    let target = fixture.mob(&EntityType::COW);
    target
        .cast_any()
        .downcast_ref::<CowEntity>()
        .unwrap()
        .set_baby(true);
    let bucket = ItemStack::new(1, &Item::BUCKET);
    fixture.set_hand(hand, bucket.clone());
    fixture.drain_packets();
    fixture.interact(target.as_ref(), hand, at);
    assert!(fixture.hand(hand).are_equal(&bucket));
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 0);
    for observer in &mut fixture.observers {
        assert!(decode_sounds(&observer.take_packets()).is_empty());
    }
}

#[derive(Clone, Copy, Debug)]
enum Edit {
    Cancel,
    Replace,
    Remove,
    Select,
    OtherHand,
}

struct BeforeMilk {
    enabled: AtomicBool,
    hand: Hand,
    edit: Edit,
    replacement: ItemStack,
    calls: AtomicUsize,
}

impl BeforeMilk {
    fn apply(&self, player: &Player) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let slot = source_slot(self.hand);
        assert_owner_free(player, slot);
        let inventory = player.inventory();
        match self.edit {
            Edit::Cancel => return true,
            Edit::Replace => inventory.set_stack(slot, self.replacement.clone()),
            Edit::Remove => inventory.set_stack(slot, ItemStack::EMPTY.clone()),
            Edit::Select => inventory.set_selected_slot(1),
            Edit::OtherHand => {
                inventory.set_stack_in_hand(other_hand(self.hand), self.replacement.clone());
            }
        }
        false
    }
}

impl EventHandler<PlayerInteractEntityEvent> for BeforeMilk {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        if self.enabled.load(Ordering::Relaxed) {
            event.cancelled = self.apply(&event.player);
        }
        Box::pin(async {})
    }
}

impl EventHandler<PlayerInteractAtEntityEvent> for BeforeMilk {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractAtEntityEvent,
    ) -> BoxFuture<'a, ()> {
        if self.enabled.load(Ordering::Relaxed) {
            assert_eq!(event.hand, u8::from(self.hand == Hand::Left));
            event.cancelled = self.apply(&event.player);
        }
        Box::pin(async {})
    }
}

fn callback_milk(fixture: &mut Fixture, hand: Hand, at: bool, edit: Edit) {
    reset(fixture);
    let target = fixture.mob(&EntityType::COW);
    let bucket = ItemStack::new(1, &Item::BUCKET);
    let selected = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.set_hand(hand, bucket.clone());
    fixture.player.inventory().set_stack(1, selected.clone());
    let hook = Arc::new(BeforeMilk {
        enabled: AtomicBool::new(true),
        hand,
        edit,
        replacement: if matches!(edit, Edit::Replace) {
            ItemStack::new(1, &Item::BUCKET)
        } else {
            ItemStack::new(1, &Item::DIAMOND)
        },
        calls: AtomicUsize::new(0),
    });
    if at {
        fixture
            .server
            .plugin_manager
            .register::<PlayerInteractAtEntityEvent, _>(hook.clone(), EventPriority::Normal, true);
    } else {
        fixture
            .server
            .plugin_manager
            .register::<PlayerInteractEntityEvent, _>(hook.clone(), EventPriority::Normal, true);
    }
    fixture.drain_packets();
    fixture.interact(target.as_ref(), hand, at);
    assert_eq!(hook.calls.load(Ordering::Relaxed), 1);
    let accepted =
        matches!(edit, Edit::OtherHand) || (matches!(edit, Edit::Select) && hand == Hand::Left);
    assert_eq!(
        fixture.inventory_count(&Item::MILK_BUCKET),
        u32::from(accepted)
    );
    let source = fixture.player.inventory().get_stack(source_slot(hand));
    match edit {
        Edit::Replace => assert_eq!(source.uid, hook.replacement.uid),
        Edit::Remove => assert!(source.is_empty()),
        Edit::Cancel => assert!(source.are_equal(&bucket)),
        Edit::Select if hand == Hand::Right => assert!(source.are_equal(&bucket)),
        Edit::Select | Edit::OtherHand => assert_eq!(source.get_item(), &Item::MILK_BUCKET),
    }
    if matches!(edit, Edit::OtherHand) {
        assert_eq!(fixture.hand(other_hand(hand)).uid, hook.replacement.uid);
    }
    assert!(fixture.player.inventory().get_stack(1).are_equal(&selected));
    for observer in &mut fixture.observers {
        assert_eq!(
            decode_sounds(&observer.take_packets()).len(),
            usize::from(accepted),
            "unexpected milk sound for callback {edit:?}"
        );
    }
    hook.enabled.store(false, Ordering::Relaxed);
}

#[derive(Clone)]
struct NativeBucket {
    data: CustomDataImpl,
    player: Weak<Player>,
    hand: Hand,
    armed: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    replacement: ItemStack,
}

impl DataComponentImpl for NativeBucket {
    fn write_data(&self) -> NbtTag {
        self.data.write_data()
    }

    fn equal(&self, other: &dyn DataComponentImpl) -> bool {
        if self.armed.load(Ordering::Relaxed) {
            let player = self.player.upgrade().unwrap();
            let inventory = player.inventory();
            let owner = inventory.slot_transaction(source_slot(self.hand));
            // Exercise the guard-free comparison in MobInteraction::new; the
            // generic admission/commit comparisons have their own existing owner.
            let free = std::thread::scope(|scope| {
                scope
                    .spawn(move || owner.try_lock().is_some())
                    .join()
                    .unwrap()
            });
            if free && self.armed.swap(false, Ordering::Relaxed) {
                assert_owner_free(&player, source_slot(self.hand));
                self.calls.fetch_add(1, Ordering::Relaxed);
                inventory.set_stack(source_slot(self.hand), self.replacement.clone());
            }
        }
        self.data.equal(other)
    }

    fn get_enum() -> DataComponent {
        DataComponent::CustomData
    }

    fn get_self_enum(&self) -> DataComponent {
        Self::get_enum()
    }

    fn to_dyn(self) -> Box<dyn DataComponentImpl> {
        Box::new(self)
    }

    fn clone_dyn(&self) -> Box<dyn DataComponentImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        &self.data
    }

    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        &mut self.data
    }
}

fn native_component_replaces_source(fixture: &mut Fixture, hand: Hand, at: bool) {
    reset(fixture);
    let target = fixture.mob(&EntityType::COW);
    let armed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let replacement = ItemStack::new(1, &Item::BUCKET);
    let mut bucket = ItemStack::new(1, &Item::BUCKET);
    bucket.set_data_component(NativeBucket {
        data: CustomDataImpl::new(NbtCompound::new()),
        player: Arc::downgrade(&fixture.player),
        hand,
        armed: armed.clone(),
        calls: calls.clone(),
        replacement: replacement.clone(),
    });
    fixture.set_hand(hand, bucket);
    fixture.drain_packets();
    armed.store(true, Ordering::Relaxed);
    fixture.interact(target.as_ref(), hand, at);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "native equality boundary ran"
    );
    assert_eq!(fixture.hand(hand).uid, replacement.uid);
    assert_eq!(
        fixture.hand(hand).item_count,
        1,
        "replacement cannot pay for milk"
    );
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_cow_milk_sound_precedes_exchange_at_player() {
    let mut fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            successful_milk(&mut fixture, hand, at, false);
            successful_milk(&mut fixture, hand, at, true);
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_cow_milk_refusal_and_callback_sources_survive() {
    let mut fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            calf_refuses(&mut fixture, hand, at);
            native_component_replaces_source(&mut fixture, hand, at);
            for edit in [
                Edit::Cancel,
                Edit::Replace,
                Edit::Remove,
                Edit::Select,
                Edit::OtherHand,
            ] {
                callback_milk(&mut fixture, hand, at, edit);
            }
        }
    }
    fixture.finish().await;
}
