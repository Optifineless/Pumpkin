use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{dimension::Dimension, item::Item};
use pumpkin_inventory::{
    Inventory, cartography_table_screen_handler::CartographyTableScreenHandler,
    screen_handler::ScreenHandler,
};
use pumpkin_protocol::java::server::play::SlotActionType;

fn menu(player: &Player, count: u8) -> CartographyTableScreenHandler {
    let mut map = ItemStack::new(count, &Item::FILLED_MAP);
    map.set_data_component(MapIdImpl { id: 47 });
    let mut menu = CartographyTableScreenHandler::new(1, &player.inventory);
    menu.input_inventory.set_stack(0, map);
    menu.input_inventory
        .set_stack(1, ItemStack::new(count, &Item::GLASS_PANE));
    menu.slots_changed(player);
    menu
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cartography_drag_allocates_nothing_and_creative_clone_processes_owned_result() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    server
        .map_manager
        .create_map(47, Dimension::OVERWORLD, 64, 64, 0)
        .lock()
        .unwrap()
        .colors[5] = 73;
    let mut menu = menu(&player, 1);
    // An empty-cursor selection used to post-process and discard the result anyway.
    for _ in 0..3 {
        menu.on_slot_click(2, 1, SlotActionType::QuickCraft, player.as_ref());
    }
    assert_eq!(server.map_manager.maps.len(), 1);
    player.gamemode.store(pumpkin_util::GameMode::Creative);
    menu.on_slot_click(2, 2, SlotActionType::Clone, player.as_ref());
    let cursor = menu.get_behaviour().cursor_stack.lock().unwrap().clone();
    assert!(
        cursor
            .get_data_component::<MapPostProcessingImpl>()
            .is_none()
    );
    let id = cursor.get_data_component::<MapIdImpl>().unwrap().id;
    assert_ne!(id, 47);
    let map = server.map_manager.get_map(id).unwrap();
    assert!(map.lock().unwrap().locked);
    assert_eq!(map.lock().unwrap().colors[5], 73);
    assert_eq!(menu.input_inventory.get_stack(0).item_count, 1);
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cartography_shift_click_repeats_across_map_ids_and_stops_when_full() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    let _ = server
        .map_manager
        .create_map(47, Dimension::OVERWORLD, 64, 64, 0);
    for space in [3, 1] {
        for index in 0..36 {
            player.inventory.set_stack(
                index,
                if index < space {
                    ItemStack::EMPTY.clone()
                } else {
                    ItemStack::new(64, &Item::DIRT)
                },
            );
        }
        let mut menu = menu(&player, 3);
        menu.on_slot_click(2, 0, SlotActionType::QuickMove, player.as_ref());
        let count = (0..36)
            .map(|i| player.inventory.get_stack(i))
            .filter(|s| s.item == &Item::FILLED_MAP)
            .map(|s| usize::from(s.item_count))
            .sum::<usize>();
        assert_eq!(count, space);
        assert_eq!(
            usize::from(menu.input_inventory.get_stack(0).item_count),
            3 - space
        );
        assert_eq!(
            usize::from(menu.input_inventory.get_stack(1).item_count),
            3 - space
        );
    }
    assert!(world.level.shutdown().await.is_ok());
}
