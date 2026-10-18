#![expect(
    clippy::unwrap_used,
    reason = "Packet dispatcher regression uses a valid in-memory server fixture"
)]

use std::sync::Arc;

use bytes::Bytes;
use pumpkin_data::{Block, item::Item, packet::CURRENT_MC_VERSION};
use pumpkin_inventory::{
    Inventory, SimpleInventory,
    generic_container_screen_handler::create_generic_3x3,
    screen_handler::{ScreenMutex, SharedScreenHandler},
};
use pumpkin_protocol::{
    ClientPacket, RawPacket, VarInt,
    java::{client::play::CSetContainerProperty, server::play::SContainerSlotStateChanged},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{GameMode, math::position::BlockPos, math::vector2::Vector2};
use pumpkin_world::chunk::ChunkData;

use crate::{
    block::{
        BlockBehaviour, GetScreenHandlerFactoryArgs, blocks::redstone::crafter::CrafterBlock,
        entities::crafter::CrafterBlockEntity,
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};

fn dispatch_slot_state(
    fixture: &TestPlayer,
    server: &Arc<crate::server::Server>,
    slot_id: i32,
    container_id: i32,
    new_state: bool,
) {
    let mut payload = Vec::new();
    SContainerSlotStateChanged {
        slot_id: VarInt(slot_id),
        container_id: VarInt(container_id),
        new_state,
    }
    .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
    .unwrap();
    fixture
        .client()
        .handle_play_packet(
            &fixture.player,
            server,
            &RawPacket {
                id: SContainerSlotStateChanged::to_id(CURRENT_MC_VERSION),
                payload: Bytes::from(payload),
            },
        )
        .unwrap();
}

fn open_crafter(
    server: &Arc<crate::server::Server>,
    world: &Arc<crate::world::World>,
    fixture: &TestPlayer,
    position: &BlockPos,
) -> u8 {
    let factory = CrafterBlock
        .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server,
            world,
            block: &Block::CRAFTER,
            position,
            player: &fixture.player,
        })
        .unwrap();
    fixture
        .player
        .open_handled_screen(factory.as_ref(), Some(*position))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crafter_slot_state_packet_updates_empty_slot() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut fixture = TestPlayer::new(&world);
    fixture
        .player
        .screen_handler_sync_handler
        .store_player(fixture.player.clone());
    let position = BlockPos::new(8, 64, 8);
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::CRAFTER.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let crafter = Arc::new(CrafterBlockEntity::new(position));
    world.add_block_entity(crafter.clone());
    crafter.set_stack(1, pumpkin_data::item_stack::ItemStack::new(1, &Item::STONE));

    let menu_id = open_crafter(&server, &world, &fixture, &position);
    fixture.take_packets();

    dispatch_slot_state(&fixture, &server, 1, i32::from(menu_id), false);
    assert!(!crafter.is_slot_disabled(1), "occupied slots stay enabled");
    dispatch_slot_state(&fixture, &server, 2, i32::from(menu_id) + 1, false);
    dispatch_slot_state(&fixture, &server, -1, i32::from(menu_id), false);
    dispatch_slot_state(&fixture, &server, 9, i32::from(menu_id), false);
    assert!(!crafter.is_slot_disabled(2));

    dispatch_slot_state(&fixture, &server, 0, i32::from(menu_id), false);
    assert!(crafter.is_slot_disabled(0));
    let screen = fixture
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let property = screen
        .lock()
        .unwrap()
        .get_behaviour()
        .properties
        .first()
        .map(pumpkin_inventory::screen_handler::ScreenProperty::get);
    assert_eq!(property, Some(1));
    let expected_property = fixture
        .client()
        .serialize_packet(&CSetContainerProperty::new(
            VarInt(i32::from(menu_id)),
            0,
            1,
        ))
        .unwrap();
    assert!(fixture.take_packets().contains(&expected_property));

    let non_crafter_inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(9));
    let non_crafter_menu = create_generic_3x3(
        menu_id,
        &fixture.player.inventory,
        non_crafter_inventory,
        fixture.player.as_ref(),
    );
    let non_crafter_menu: SharedScreenHandler =
        Arc::new(ScreenMutex::new(non_crafter_menu)) as SharedScreenHandler;
    *fixture.player.current_screen_handler.lock().unwrap() = non_crafter_menu;
    dispatch_slot_state(&fixture, &server, 2, i32::from(menu_id), false);
    assert!(
        !crafter.is_slot_disabled(2),
        "a non-crafter menu is ignored"
    );

    fixture.player.set_gamemode(GameMode::Spectator);
    let spectator_menu_id = open_crafter(&server, &world, &fixture, &position);
    dispatch_slot_state(&fixture, &server, 2, i32::from(spectator_menu_id), false);
    assert!(
        !crafter.is_slot_disabled(2),
        "Spectators cannot toggle slots"
    );

    server.shutdown().await;
}
