use std::sync::Arc;

use crate::{
    entity::{EntityBase, item::ItemEntity},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_interact_event::PlayerInteractEvent,
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    data_component::DataComponent,
    data_component_impl::{CustomNameImpl, EquipmentSlot, LodestoneTarget, LodestoneTrackerImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::ItemStackSerializer,
    java::{
        client::play::{CSetEquipment, CSetPlayerInventory},
        server::play::SUseItemOn,
    },
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;

const LODESTONE_POSITION: BlockPos = BlockPos::new(0, 64, 1);
const SECOND_LODESTONE_POSITION: BlockPos = BlockPos::new(1, 64, 1);
const OPEN_OUTPUT_SLOT: usize = 17;
const CUSTOM_NAME: &str = "Survey compass";

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    actor: TestPlayer,
    observer: TestPlayer,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let mut actor = TestPlayer::new(&world);
        let mut observer = TestPlayer::new(&world);
        actor
            .player
            .get_entity()
            .set_pos(Vector3::new(0.5, 64.0, 0.5));
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(2.5, 64.0, 0.5));
        world.players.store(Arc::new(vec![
            actor.player.clone(),
            observer.player.clone(),
        ]));
        for player in [&actor.player, &observer.player] {
            let chunk = player.get_entity().chunk_pos.load();
            player
                .chunk_sender
                .lock()
                .unwrap()
                .mark_sent_out_of_band(chunk);
        }
        world
            .entity_tracker
            .pair_new_player_with_tracked_entities(&observer.player, &world);
        world
            .entity_tracker
            .pair_new_player_with_tracked_entities(&actor.player, &world);
        actor.take_packets();
        observer.take_packets();
        Self {
            _directory: directory,
            server,
            world,
            actor,
            observer,
        }
    }

    fn put_lodestone(&self, position: BlockPos) {
        self.world.set_block_state(
            &position,
            Block::LODESTONE.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
    }

    fn use_on(&self, hand: Hand, position: BlockPos, sequence: i32) {
        self.actor
            .client()
            .handle_use_item_on(
                &self.actor.player,
                &SUseItemOn {
                    hand: VarInt(i32::from(hand == Hand::Left)),
                    position,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(sequence),
                },
                &self.server,
            )
            .unwrap();
    }

    fn finish_equipment_setup(&mut self) {
        self.actor.player.detect_equipment_updates();
        self.actor.take_packets();
        self.observer.take_packets();
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn named_compass(count: u8) -> ItemStack {
    let mut stack = ItemStack::new(count, &Item::COMPASS);
    stack.set_custom_name(CUSTOM_NAME.to_owned());
    stack
}

fn component_rich_compass(count: u8) -> ItemStack {
    let mut stack = named_compass(count);
    stack.remove_data_component(DataComponent::Enchantments);
    stack.set_data_component(old_tracker());
    stack
}

fn old_tracker() -> LodestoneTrackerImpl {
    LodestoneTrackerImpl {
        target: Some(LodestoneTarget {
            dimension: "minecraft:the_nether".to_owned(),
            x: 4,
            y: 72,
            z: -9,
        }),
        tracked: false,
    }
}

fn hand_slot(hand: Hand) -> usize {
    if hand == Hand::Right {
        0
    } else {
        PlayerInventory::OFF_HAND_SLOT
    }
}

fn other_hand(hand: Hand) -> Hand {
    if hand == Hand::Right {
        Hand::Left
    } else {
        Hand::Right
    }
}

fn fill_inventory(fixture: &Fixture, hand: Hand, held: ItemStack, allow_output: bool) -> ItemStack {
    let inventory = fixture.actor.player.inventory();
    for slot in 0..PlayerInventory::MAIN_SIZE {
        if allow_output && slot == OPEN_OUTPUT_SLOT {
            continue;
        }
        inventory.set_stack(slot, ItemStack::new(64, &Item::DIAMOND));
    }
    for (slot, armor) in [
        (36, &Item::DIAMOND_BOOTS),
        (37, &Item::DIAMOND_LEGGINGS),
        (38, &Item::DIAMOND_CHESTPLATE),
        (39, &Item::DIAMOND_HELMET),
    ] {
        inventory.set_stack(slot, ItemStack::new(1, armor));
    }
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    inventory.set_stack_in_hand(hand, held);
    inventory.set_stack_in_hand(other_hand(hand), sword.clone());
    sword
}

fn lodestone_tracker(stack: &ItemStack, fixture: &Fixture, position: BlockPos) {
    let trackers = stack
        .patch
        .iter()
        .filter(|(id, _)| *id == DataComponent::LodestoneTracker)
        .count();
    assert_eq!(trackers, 1, "the patch has exactly one lodestone tracker");
    let tracker = stack
        .get_data_component::<LodestoneTrackerImpl>()
        .expect("bound compass tracker component");
    assert_eq!(
        tracker.target.as_ref(),
        Some(&LodestoneTarget {
            dimension: fixture.world.dimension.minecraft_name.to_string(),
            x: position.0.x,
            y: position.0.y,
            z: position.0.z,
        })
    );
    assert!(tracker.tracked);
}

fn preserved_component_patch(stack: &ItemStack) {
    assert_eq!(
        stack.patch.len(),
        3,
        "only the original name, removal, and tracker patches remain"
    );
    assert_eq!(stack.get_hover_name(), CUSTOM_NAME);
    assert_eq!(
        stack
            .get_data_component::<CustomNameImpl>()
            .expect("custom name component")
            .name
            .clone()
            .get_text(),
        CUSTOM_NAME
    );
    assert_eq!(
        stack
            .patch
            .iter()
            .filter(|(id, _)| *id == DataComponent::CustomName)
            .count(),
        1
    );
    assert_eq!(
        stack
            .patch
            .iter()
            .filter(|(id, _)| *id == DataComponent::Enchantments)
            .count(),
        1
    );
    assert!(
        stack
            .patch
            .iter()
            .find(|(id, _)| *id == DataComponent::Enchantments)
            .unwrap()
            .1
            .is_none(),
        "the enchantment component removal remains in the patch"
    );
}

fn compass_item_entities(world: &World) -> Vec<ItemStack> {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            entity
                .cast_any()
                .downcast_ref::<ItemEntity>()
                .map(|item| item.get_item_stack().lock().unwrap().clone())
        })
        .filter(|stack| stack.item.id == Item::COMPASS.id)
        .collect()
}

fn assert_owner_hand_packet(fixture: &mut Fixture, hand: Hand) {
    let slot = hand_slot(hand);
    let stack = fixture.actor.player.inventory().get_stack(slot);
    let serializer = ItemStackSerializer::from(stack);
    let packet = CSetPlayerInventory::new((slot as i32).into(), &serializer);
    let expected = fixture.actor.client().serialize_packet(&packet).unwrap();
    assert!(fixture.actor.take_packets().contains(&expected));
}

fn assert_observer_equipment_packet(fixture: &mut Fixture, hand: Hand) {
    let slot = if hand == Hand::Left {
        EquipmentSlot::OFF_HAND.discriminant()
    } else {
        EquipmentSlot::MAIN_HAND.discriminant()
    };
    let stack = fixture.actor.player.inventory().get_stack_in_hand(hand);
    let packet = CSetEquipment::new(
        fixture.actor.player.entity_id().into(),
        vec![(slot, ItemStackSerializer::from(stack))],
    );
    let expected = fixture.observer.client().serialize_packet(&packet).unwrap();
    assert!(fixture.observer.take_packets().contains(&expected));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g02_named_singleton_stays_in_requested_hand() {
    for hand in [Hand::Right, Hand::Left] {
        let mut fixture = Fixture::new();
        fixture.put_lodestone(LODESTONE_POSITION);
        let source = named_compass(1);
        let source_uid = source.uid;
        let sword = fill_inventory(&fixture, hand, source.clone(), false);
        let other_before = fixture
            .actor
            .player
            .inventory()
            .get_stack_in_hand(other_hand(hand));
        assert_eq!(other_before.uid, sword.uid);
        fixture.finish_equipment_setup();
        let inventory = fixture.actor.player.inventory();

        fixture.use_on(hand, LODESTONE_POSITION, 1);
        let held = inventory.get_stack_in_hand(hand);
        assert_eq!(held.item, &Item::COMPASS);
        assert_eq!(held.item_count, 1);
        assert_eq!(held.uid, source_uid, "the same stack identity remains held");
        assert!(
            held.patch
                .iter()
                .map(|(id, _)| *id)
                .eq([DataComponent::CustomName, DataComponent::LodestoneTracker]),
            "the source patch changes only by adding the tracker"
        );
        assert_eq!(
            inventory.get_stack_in_hand(other_hand(hand)).uid,
            other_before.uid
        );
        assert!(
            inventory
                .get_stack_in_hand(other_hand(hand))
                .are_equal(&other_before)
        );
        assert_eq!(held.get_hover_name(), CUSTOM_NAME);
        lodestone_tracker(&held, &fixture, LODESTONE_POSITION);
        assert!(compass_item_entities(&fixture.world).is_empty());
        assert_owner_hand_packet(&mut fixture, hand);

        fixture.actor.player.detect_equipment_updates();
        assert_observer_equipment_packet(&mut fixture, hand);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g02_stack_binding_copies_components_and_replaces_tracker() {
    for hand in [Hand::Right, Hand::Left] {
        let mut fixture = Fixture::new();
        fixture.put_lodestone(LODESTONE_POSITION);
        let source = component_rich_compass(2);
        let source_uid = source.uid;
        let sword = fill_inventory(&fixture, hand, source, true);
        fixture.finish_equipment_setup();
        let inventory = fixture.actor.player.inventory();
        fixture.use_on(hand, LODESTONE_POSITION, 1);

        let held = inventory.get_stack_in_hand(hand);
        assert_eq!(held.item_count, 1, "one source compass is consumed");
        assert_eq!(held.uid, source_uid);
        assert!(
            inventory
                .get_stack_in_hand(other_hand(hand))
                .are_equal(&sword)
        );
        let output = inventory.get_stack(OPEN_OUTPUT_SLOT);
        assert_eq!(output.item, &Item::COMPASS);
        assert_eq!(output.item_count, 1);
        assert_ne!(output.uid, source_uid, "the copied output has a fresh UID");
        preserved_component_patch(&output);
        lodestone_tracker(&output, &fixture, LODESTONE_POSITION);
        assert!(compass_item_entities(&fixture.world).is_empty());
        fixture.finish().await;

        let mut fixture = Fixture::new();
        fixture.put_lodestone(LODESTONE_POSITION);
        let source = component_rich_compass(2);
        let source_uid = source.uid;
        let sword = fill_inventory(&fixture, hand, source, false);
        fixture.finish_equipment_setup();
        let inventory = fixture.actor.player.inventory();
        fixture.use_on(hand, LODESTONE_POSITION, 1);

        let held = inventory.get_stack_in_hand(hand);
        assert_eq!(held.item_count, 1, "one source compass is consumed");
        assert_eq!(held.uid, source_uid);
        assert!(
            inventory
                .get_stack_in_hand(other_hand(hand))
                .are_equal(&sword)
        );
        let drops = compass_item_entities(&fixture.world);
        assert_eq!(drops.len(), 1, "a full Survival inventory drops one output");
        assert_eq!(drops[0].item_count, 1);
        assert_ne!(drops[0].uid, source_uid);
        preserved_component_patch(&drops[0]);
        lodestone_tracker(&drops[0], &fixture, LODESTONE_POSITION);
        fixture.finish().await;
    }

    let mut fixture = Fixture::new();
    fixture.put_lodestone(LODESTONE_POSITION);
    fixture.put_lodestone(SECOND_LODESTONE_POSITION);
    let source = component_rich_compass(1);
    let source_uid = source.uid;
    fixture
        .actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, source);
    fixture.finish_equipment_setup();

    fixture.use_on(Hand::Right, LODESTONE_POSITION, 1);
    let first_binding = fixture.actor.player.inventory().held_item();
    assert_eq!(first_binding.uid, source_uid);
    preserved_component_patch(&first_binding);
    lodestone_tracker(&first_binding, &fixture, LODESTONE_POSITION);

    fixture.use_on(Hand::Right, SECOND_LODESTONE_POSITION, 2);
    let rebound = fixture.actor.player.inventory().held_item();
    assert_eq!(rebound.uid, source_uid);
    preserved_component_patch(&rebound);
    lodestone_tracker(&rebound, &fixture, SECOND_LODESTONE_POSITION);
    assert!(compass_item_entities(&fixture.world).is_empty());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g02_creative_binding_uses_component_preserving_copy() {
    for hand in [Hand::Right, Hand::Left] {
        let mut fixture = Fixture::new();
        fixture.put_lodestone(LODESTONE_POSITION);
        fixture.actor.player.gamemode.store(GameMode::Creative);
        let source = component_rich_compass(2);
        let original_tracker = old_tracker();
        let source_uid = source.uid;
        let sword = fill_inventory(&fixture, hand, source.clone(), true);
        fixture.finish_equipment_setup();
        fixture.use_on(hand, LODESTONE_POSITION, 1);

        let held = fixture.actor.player.inventory().get_stack_in_hand(hand);
        assert_eq!(held.item_count, 2, "Creative retains the source count");
        assert_eq!(held.uid, source_uid);
        preserved_component_patch(&held);
        assert_eq!(
            held.get_data_component::<LodestoneTrackerImpl>(),
            Some(&original_tracker)
        );
        let output = fixture.actor.player.inventory().get_stack(OPEN_OUTPUT_SLOT);
        assert_eq!(output.item_count, 1);
        assert_ne!(output.uid, source_uid);
        preserved_component_patch(&output);
        lodestone_tracker(&output, &fixture, LODESTONE_POSITION);
        assert!(
            fixture
                .actor
                .player
                .inventory()
                .get_stack_in_hand(other_hand(hand))
                .are_equal(&sword)
        );
        assert!(compass_item_entities(&fixture.world).is_empty());
        fixture.finish().await;

        let mut fixture = Fixture::new();
        fixture.put_lodestone(LODESTONE_POSITION);
        fixture.actor.player.gamemode.store(GameMode::Creative);
        let source = component_rich_compass(2);
        let original_tracker = old_tracker();
        let source_uid = source.uid;
        let sword = fill_inventory(&fixture, hand, source, false);
        fixture.finish_equipment_setup();
        fixture.use_on(hand, LODESTONE_POSITION, 1);

        let held = fixture.actor.player.inventory().get_stack_in_hand(hand);
        assert_eq!(held.item_count, 2, "Creative retains the source count");
        assert_eq!(held.uid, source_uid);
        preserved_component_patch(&held);
        assert_eq!(
            held.get_data_component::<LodestoneTrackerImpl>(),
            Some(&original_tracker)
        );
        assert!(
            fixture
                .actor
                .player
                .inventory()
                .get_stack_in_hand(other_hand(hand))
                .are_equal(&sword)
        );
        assert!(compass_item_entities(&fixture.world).is_empty());
        fixture.finish().await;
    }
}

struct CancelInteraction;

impl EventHandler<PlayerInteractEvent> for CancelInteraction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            event.cancelled = true;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g02_refusal_and_cancellation_controls_preserve_inventory() {
    let mut fixture = Fixture::new();
    let ordinary_block = LODESTONE_POSITION;
    let named = named_compass(1);
    let named_uid = named.uid;
    fixture
        .actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, named);
    fixture
        .actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::new(1, &Item::RECOVERY_COMPASS));
    fixture.finish_equipment_setup();

    fixture.use_on(Hand::Right, ordinary_block, 1);
    let ordinary = fixture.actor.player.inventory().held_item();
    assert_eq!(ordinary.uid, named_uid);
    assert_eq!(ordinary.item_count, 1);
    assert_eq!(ordinary.get_hover_name(), CUSTOM_NAME);
    assert!(
        ordinary
            .get_data_component::<LodestoneTrackerImpl>()
            .is_none()
    );

    fixture.put_lodestone(ordinary_block);
    let recovery = fixture.actor.player.inventory().off_hand_item();
    let recovery_uid = recovery.uid;
    fixture.use_on(Hand::Left, ordinary_block, 2);
    let recovery_after = fixture.actor.player.inventory().off_hand_item();
    assert_eq!(recovery_after.uid, recovery_uid);
    assert_eq!(recovery_after.item, &Item::RECOVERY_COMPASS);
    assert_eq!(recovery_after.item_count, 1);
    assert!(
        recovery_after
            .get_data_component::<LodestoneTrackerImpl>()
            .is_none()
    );
    assert!(compass_item_entities(&fixture.world).is_empty());
    fixture.finish().await;

    let mut fixture = Fixture::new();
    fixture.put_lodestone(LODESTONE_POSITION);
    let source = component_rich_compass(1);
    let source_uid = source.uid;
    let old = old_tracker();
    fixture
        .actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, source);
    fixture.finish_equipment_setup();
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(
            Arc::new(CancelInteraction),
            EventPriority::Normal,
            true,
        );

    fixture.use_on(Hand::Right, LODESTONE_POSITION, 1);
    let held = fixture.actor.player.inventory().held_item();
    assert_eq!(held.uid, source_uid);
    assert_eq!(held.item_count, 1);
    preserved_component_patch(&held);
    assert_eq!(
        held.get_data_component::<LodestoneTrackerImpl>(),
        Some(&old)
    );
    assert!(compass_item_entities(&fixture.world).is_empty());
    fixture.finish().await;
}

#[path = "compass_conservation_tests.rs"]
mod conservation;
