use std::sync::{Arc, atomic::Ordering};

use pumpkin_data::{Block, BlockDirection, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory, beacon_screen_handler::BeaconScreenHandler, screen_handler::SharedScreenHandler,
};
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::{
        client::play::CSetContainerProperty,
        server::play::{SClickSlot, SSetBeacon, SUseItemOn, SlotActionType},
    },
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{
    chunk::io::{FileIO, LoadedData},
    world::BlockFlags,
};

use crate::{
    block::entities::beacon::BeaconBlockEntity,
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::World,
};

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    pos: BlockPos,
    beacon: Arc<BeaconBlockEntity>,
    player: TestPlayer,
}

impl Fixture {
    fn new(levels: i32) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let pos = BlockPos::new(0, 64, 0);
        combat_test_support::publish_empty_chunk(&world, pos.chunk_position());
        world.set_block_state(
            &pos,
            Block::BEACON.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        let beacon = Arc::new(BeaconBlockEntity::new(pos));
        beacon.levels.store(levels, Ordering::Relaxed);
        world.add_block_entity(beacon.clone());
        let player = TestPlayer::new(&world);
        player.client().set_player(player.player.clone());
        player
            .player
            .screen_handler_sync_handler
            .store_player(player.player.clone());
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(0.5, 64.0, 2.5));
        Self {
            _directory: directory,
            server,
            world,
            pos,
            beacon,
            player,
        }
    }

    fn use_block(&self, player: &TestPlayer, pos: BlockPos) {
        player
            .client()
            .handle_use_item_on(&player.player, &use_packet(pos), &self.server)
            .unwrap();
    }

    fn use_beacon(&self) {
        self.use_block(&self.player, self.pos);
    }

    async fn finish(self) {
        self.player.client().close();
        self.world.level.shutdown().await.unwrap();
    }
}

fn use_packet(position: BlockPos) -> SUseItemOn {
    SUseItemOn {
        hand: VarInt(0),
        position,
        face: VarInt(BlockDirection::Up as i32),
        cursor_pos: Vector3::new(0.5, 0.5, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(1),
    }
}

fn current_screen(player: &Player) -> SharedScreenHandler {
    player
        .current_screen_handler
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn beacon_payment(player: &Player) -> Arc<dyn Inventory> {
    let screen = current_screen(player);
    let screen = screen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    screen
        .as_any()
        .downcast_ref::<BeaconScreenHandler>()
        .expect("beacon menu should be open")
        .inventory
        .clone()
}

fn assert_beacon_menu(player: &Player) {
    let screen = current_screen(player);
    assert!(
        screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_any()
            .is::<BeaconScreenHandler>()
    );
}

fn click(player: &Arc<Player>, server: &Arc<Server>, slot: i16, button: i8, mode: SlotActionType) {
    let screen = current_screen(player);
    let (sync_id, revision) = {
        let screen = screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let behaviour = screen.get_behaviour();
        (
            behaviour.sync_id,
            behaviour.revision.load(Ordering::Relaxed) as i32,
        )
    };
    player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(i32::from(sync_id)),
            revision: VarInt(revision),
            slot,
            button,
            mode,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        server,
    );
}

fn property_packet(player: &TestPlayer, sync_id: u8, index: i16, value: i16) -> bytes::Bytes {
    player
        .client()
        .serialize_packet(&CSetContainerProperty::new(
            VarInt(i32::from(sync_id)),
            index,
            value,
        ))
        .unwrap()
}

fn menu_sync_id(player: &Player) -> u8 {
    current_screen(player)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .sync_id()
}

fn set_cursor(player: &Player, stack: ItemStack) {
    let screen = current_screen(player);
    let screen = screen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *screen
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = stack;
}

fn cursor_stack(player: &Player) -> ItemStack {
    let screen = current_screen(player);
    let screen = screen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    screen
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn emerald_count(player: &Player) -> u32 {
    (0..player.inventory.size())
        .map(|index| player.inventory.get_stack(index))
        .filter(|stack| stack.item == &Item::EMERALD)
        .map(|stack| u32::from(stack.item_count))
        .sum()
}

fn set_beacon(player: &TestPlayer, primary: Option<i32>, secondary: Option<i32>) {
    player.client().handle_set_beacon(
        &player.player,
        &SSetBeacon {
            primary_effect: primary.map(VarInt),
            secondary_effect: secondary.map(VarInt),
        },
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_use_item_on_opens_powered_and_unpowered_menus_only_for_beacons() {
    let fixture = Fixture::new(0);
    for levels in [0, 4] {
        fixture.beacon.levels.store(levels, Ordering::Relaxed);
        fixture.use_beacon();
        assert_beacon_menu(&fixture.player.player);
        fixture.player.player.close_handled_screen();
    }

    let stone_pos = BlockPos::new(1, 64, 0);
    fixture.world.set_block_state(
        &stone_pos,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    fixture.use_block(&fixture.player, stone_pos);
    let screen = current_screen(&fixture.player.player);
    let player_menu: SharedScreenHandler = fixture.player.player.player_screen_handler.clone();
    assert!(Arc::ptr_eq(&screen, &player_menu));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_click_validation_and_set_beacon_packet_sync_properties() {
    let mut fixture = Fixture::new(4);
    fixture.beacon.primary_effect.store(0, Ordering::Relaxed);
    fixture.beacon.secondary_effect.store(9, Ordering::Relaxed);
    fixture.use_beacon();
    let sync_id = menu_sync_id(&fixture.player.player);
    let initial = fixture.player.take_packets();
    for (index, value) in [(0, 4), (1, 1), (2, 10)] {
        let expected = property_packet(&fixture.player, sync_id, index, value);
        assert!(
            initial.contains(&expected),
            "missing property {index}={value}; actual={initial:?}; expected={expected:?}"
        );
    }

    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::EMERALD));
    click(
        &fixture.player.player,
        &fixture.server,
        28,
        0,
        SlotActionType::QuickMove,
    );
    let payment = beacon_payment(&fixture.player.player);
    assert_eq!(payment.get_stack(0).item_count, 1);
    assert!(fixture.player.player.inventory.get_stack(0).is_empty());

    // A server-side cursor click exercises Slot::can_insert with a non-payment item.
    set_cursor(&fixture.player.player, ItemStack::new(1, &Item::STONE));
    click(
        &fixture.player.player,
        &fixture.server,
        0,
        0,
        SlotActionType::Pickup,
    );
    assert_eq!(payment.get_stack(0).item, &Item::EMERALD);
    assert_eq!(payment.get_stack(0).item_count, 1);

    // A click can split one valid item from the cursor stack, and the slot still caps at one.
    payment.remove_stack(0);
    set_cursor(&fixture.player.player, ItemStack::new(2, &Item::EMERALD));
    click(
        &fixture.player.player,
        &fixture.server,
        0,
        0,
        SlotActionType::Pickup,
    );
    assert_eq!(payment.get_stack(0).item_count, 1);
    assert_eq!(cursor_stack(&fixture.player.player).item_count, 1);

    // Quick move accepts only a singleton eligible stack, matching BeaconMenu.
    payment.remove_stack(0);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(2, &Item::DIAMOND));
    click(
        &fixture.player.player,
        &fixture.server,
        28,
        0,
        SlotActionType::QuickMove,
    );
    assert_eq!(fixture.player.player.inventory.get_stack(0).item_count, 2);
    assert!(payment.get_stack(0).is_empty());
    payment.set_stack(0, ItemStack::new(1, &Item::EMERALD));

    fixture.player.take_packets();
    set_beacon(&fixture.player, Some(2), Some(2));
    assert!(payment.get_stack(0).is_empty());
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.beacon.secondary_effect.load(Ordering::Relaxed), 2);
    let updated = fixture.player.take_packets();
    for (index, value) in [(0, 4), (1, 3), (2, 3)] {
        assert!(updated.contains(&property_packet(&fixture.player, sync_id, index, value)));
    }

    // A valid selection without another payment leaves the current effects untouched.
    set_beacon(&fixture.player, Some(0), None);
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.beacon.secondary_effect.load(Ordering::Relaxed), 2);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_rejects_invalid_items_and_set_beacon_effects_without_consuming_payment() {
    let fixture = Fixture::new(4);
    fixture.use_beacon();
    let payment = beacon_payment(&fixture.player.player);
    payment.set_stack(0, ItemStack::new(1, &Item::STONE));
    set_beacon(&fixture.player, Some(0), None);
    assert_eq!(payment.get_stack(0).item, &Item::STONE);
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), -1);

    payment.set_stack(0, ItemStack::new(2, &Item::EMERALD));
    set_beacon(&fixture.player, Some(0), None);
    assert_eq!(payment.get_stack(0).item_count, 2);
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), -1);

    payment.set_stack(0, ItemStack::new(1, &Item::EMERALD));
    set_beacon(&fixture.player, Some(99), None);
    assert_eq!(payment.get_stack(0).item_count, 1);
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), -1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_beacon_menus_own_separate_payment_inventories() {
    let fixture = Fixture::new(4);
    let second = TestPlayer::new(&fixture.world);
    second.client().set_player(second.player.clone());
    second
        .player
        .screen_handler_sync_handler
        .store_player(second.player.clone());
    second
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 2.5));
    fixture.world.players.store(Arc::new(vec![
        fixture.player.player.clone(),
        second.player.clone(),
    ]));

    fixture.use_beacon();
    fixture.use_block(&second, fixture.pos);
    let first_payment = beacon_payment(&fixture.player.player);
    let second_payment = beacon_payment(&second.player);
    assert!(!Arc::ptr_eq(&first_payment, &second_payment));

    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::EMERALD));
    click(
        &fixture.player.player,
        &fixture.server,
        28,
        0,
        SlotActionType::QuickMove,
    );
    assert_eq!(first_payment.get_stack(0).item_count, 1);
    assert!(second_payment.get_stack(0).is_empty());
    second.client().close();
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_unpaid_payment_returns_once_on_close_replacement_and_disconnect() {
    let fixture = Fixture::new(1);
    fixture.use_beacon();
    beacon_payment(&fixture.player.player).set_stack(0, ItemStack::new(1, &Item::EMERALD));
    fixture.player.player.close_handled_screen();
    fixture.player.player.close_handled_screen();
    let after_close = emerald_count(&fixture.player.player);

    fixture.use_beacon();
    beacon_payment(&fixture.player.player).set_stack(0, ItemStack::new(1, &Item::EMERALD));
    fixture.use_beacon(); // Opening the replacement closes and returns the first menu's item.
    let after_replacement = emerald_count(&fixture.player.player);
    assert!(
        beacon_payment(&fixture.player.player)
            .get_stack(0)
            .is_empty()
    );

    beacon_payment(&fixture.player.player).set_stack(0, ItemStack::new(1, &Item::EMERALD));
    fixture.player.player.remove().await;
    let after_disconnect = emerald_count(&fixture.player.player);
    assert_eq!(
        (after_close, after_replacement, after_disconnect),
        (1, 2, 2)
    );
    // BeaconMenu.removed drops a departing player's unpaid payment into the world.
    let dropped = fixture
        .world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| item.get_item_stack().lock().unwrap().clone())
        .filter(|stack| stack.item == &Item::EMERALD)
        .map(|stack| u32::from(stack.item_count))
        .sum::<u32>();
    assert_eq!(dropped, 1);
    assert_eq!(after_disconnect + dropped, 3);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_close_drops_unpaid_payment_when_player_inventory_is_full() {
    let fixture = Fixture::new(1);
    fixture.use_beacon();
    beacon_payment(&fixture.player.player).set_stack(0, ItemStack::new(1, &Item::EMERALD));
    for slot in 0..fixture.player.player.inventory.size() {
        fixture
            .player
            .player
            .inventory
            .set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    fixture.player.player.close_handled_screen();
    let dropped = fixture
        .world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| item.get_item_stack().lock().unwrap().clone())
        .filter(|stack| stack.item == &Item::EMERALD)
        .map(|stack| u32::from(stack.item_count))
        .sum::<u32>();
    assert_eq!(dropped, 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_open_menu_payment_stays_transient_across_chunk_save_and_reload() {
    let fixture = Fixture::new(3);
    fixture.use_beacon();
    beacon_payment(&fixture.player.player).set_stack(0, ItemStack::new(1, &Item::EMERALD));

    let chunk_pos = fixture.pos.chunk_position();
    let chunk = fixture
        .world
        .level
        .loaded_chunks
        .get(&chunk_pos)
        .unwrap()
        .clone();
    fixture
        .world
        .level
        .chunk_saver
        .save_chunks(&fixture.world.level.level_folder, vec![(chunk_pos, chunk)])
        .await
        .unwrap();
    fixture.world.block_entities.remove(&chunk_pos);
    fixture.world.level.loaded_chunks.remove(&chunk_pos);
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    fixture
        .world
        .level
        .chunk_saver
        .fetch_chunks(&fixture.world.level.level_folder, &[chunk_pos], send)
        .await;
    let LoadedData::Loaded(reloaded) =
        tokio::time::timeout(std::time::Duration::from_secs(10), receive.recv())
            .await
            .expect("beacon chunk reload timed out")
            .expect("beacon chunk reload channel closed")
    else {
        panic!("saved beacon chunk missing")
    };
    fixture
        .world
        .level
        .loaded_chunks
        .insert(chunk_pos, reloaded);

    let reloaded_entity = fixture.world.get_block_entity(&fixture.pos).unwrap();
    let reloaded_beacon = reloaded_entity
        .as_any()
        .downcast_ref::<BeaconBlockEntity>()
        .unwrap();
    assert_eq!(reloaded_beacon.levels.load(Ordering::Relaxed), 3);
    let nbt = {
        let chunk = fixture.world.level.loaded_chunks.get(&chunk_pos).unwrap();
        chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&fixture.pos)
            .unwrap()
            .clone()
    };
    assert!(nbt.get_list("Items").is_none());

    fixture.use_beacon();
    assert!(
        beacon_payment(&fixture.player.player)
            .get_stack(0)
            .is_empty()
    );
    assert_eq!(emerald_count(&fixture.player.player), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_broken_while_open_keeps_payment_and_invalidates_menu() {
    let fixture = Fixture::new(4);
    fixture.use_beacon();
    let payment = beacon_payment(&fixture.player.player);
    payment.set_stack(0, ItemStack::new(1, &Item::EMERALD));

    // Another player breaks the beacon while this menu is still open.
    fixture.world.set_block_state(
        &fixture.pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(fixture.world.get_block_entity(&fixture.pos).is_none());

    // BeaconMenu.stillValid fails, so the selection must not spend the payment.
    set_beacon(&fixture.player, Some(0), None);
    assert_eq!(payment.get_stack(0).item_count, 1);
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), -1);
    assert!(
        !current_screen(&fixture.player.player)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .can_use(fixture.player.player.as_ref())
    );

    // The invalid menu closes through the normal path and hands the payment back.
    fixture.player.player.close_handled_screen();
    assert_eq!(emerald_count(&fixture.player.player), 1);
    fixture.finish().await;
}

#[path = "beacon_persistence_tests.rs"]
mod persistence;

#[path = "beacon_followup_tests.rs"]
mod followups;
