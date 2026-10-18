#![expect(
    clippy::unwrap_used,
    reason = "Registered menu packet regression uses an in-memory server fixture"
)]

use std::sync::Arc;

use bytes::Bytes;
use pumpkin_data::{
    Block, data_component_impl::BannerPatternsImpl, item::Item, item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::{
    Inventory,
    loom_screen_handler::LoomScreenHandler,
    screen_handler::{ScreenHandler, SharedScreenHandler},
};
use pumpkin_protocol::{
    ClientPacket, RawPacket, VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{GameMode, math::position::BlockPos, math::vector2::Vector2};
use pumpkin_world::chunk::ChunkData;

use crate::{
    block::{BlockBehaviour, GetScreenHandlerFactoryArgs, blocks::loom::LoomBlock},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
};

fn open_registered_loom() -> (
    tempfile::TempDir,
    Arc<Server>,
    TestPlayer,
    u8,
    SharedScreenHandler,
) {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .screen_handler_sync_handler
        .store_player(fixture.player.clone());
    fixture.player.set_gamemode(GameMode::Survival);

    let position = BlockPos::new(8, 64, 8);
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::LOOM.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let factory = LoomBlock
        .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: &server,
            world: &world,
            block: &Block::LOOM,
            position: &position,
            player: &fixture.player,
        })
        .unwrap();
    let menu_id = fixture
        .player
        .open_handled_screen(factory.as_ref(), Some(position))
        .unwrap();
    let screen = fixture
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    (directory, server, fixture, menu_id, screen)
}

fn prepare_loom(fixture: &TestPlayer, screen: &SharedScreenHandler) -> (u32, ItemStack) {
    let mut handler = screen.lock().unwrap();
    let revision = handler
        .get_behaviour()
        .revision
        .load(std::sync::atomic::Ordering::Relaxed);
    let loom = handler
        .as_any_mut()
        .downcast_mut::<LoomScreenHandler>()
        .unwrap();
    loom.input_inventory
        .set_stack(0, ItemStack::new(3, &Item::WHITE_BANNER));
    loom.input_inventory
        .set_stack(1, ItemStack::new(3, &Item::RED_DYE));
    loom.input_inventory
        .set_stack(2, ItemStack::new(1, &Item::FLOWER_BANNER_PATTERN));
    loom.slots_changed();
    assert!(loom.on_button_click(fixture.player.as_ref(), 0));
    let preview = loom.output_inventory.get_stack(0);
    assert_eq!(preview.item, &Item::WHITE_BANNER);
    assert_eq!(
        preview
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .len(),
        1
    );
    (revision, preview)
}

fn dispatch_shift_click(fixture: &TestPlayer, server: &Arc<Server>, menu_id: u8, revision: u32) {
    let mut payload = Vec::new();
    SClickSlot {
        sync_id: VarInt(i32::from(menu_id)),
        revision: VarInt(revision as i32),
        slot: 3,
        button: SClickSlot::BUTTON_LEFT,
        mode: SlotActionType::QuickMove,
        length_of_array: VarInt(0),
        array_of_changed_slots: Vec::new(),
        carried_item: OptionalItemStackHash(None),
    }
    .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
    .unwrap();
    fixture
        .client()
        .handle_play_packet(
            &fixture.player,
            server,
            &RawPacket {
                id: SClickSlot::to_id(CURRENT_MC_VERSION),
                payload: Bytes::from(payload),
            },
        )
        .unwrap();
}

fn assert_loom_conserved(
    fixture: &TestPlayer,
    screen: &SharedScreenHandler,
    expected_preview: &ItemStack,
) {
    let patterned_banners: u32 = (0..fixture.player.inventory.size())
        .map(|slot| fixture.player.inventory.get_stack(slot))
        .filter(|stack| stack.are_items_and_components_equal(expected_preview))
        .map(|stack| u32::from(stack.item_count))
        .sum();
    assert_eq!(patterned_banners, 3);

    let handler = screen.lock().unwrap();
    let loom = handler
        .as_any()
        .downcast_ref::<LoomScreenHandler>()
        .unwrap();
    assert!(loom.input_inventory.get_stack(0).is_empty());
    assert!(loom.input_inventory.get_stack(1).is_empty());
    assert_eq!(loom.input_inventory.get_stack(2).item_count, 1);
    assert!(loom.output_inventory.get_stack(0).is_empty());
    assert!(
        handler
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loom_shift_click_crafts_every_available_banner() {
    let (_directory, server, fixture, menu_id, screen) = open_registered_loom();
    let (revision, expected_preview) = prepare_loom(&fixture, &screen);
    dispatch_shift_click(&fixture, &server, menu_id, revision);
    assert_loom_conserved(&fixture, &screen, &expected_preview);
    server.shutdown().await;
}
