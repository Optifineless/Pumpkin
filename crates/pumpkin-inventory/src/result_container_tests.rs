use crate::{
    Inventory, cartography_table_screen_handler::CartographyTableScreenHandler,
    ext_review_tests::Player, screen_handler::ScreenHandler,
    stonecutter_screen_handler::StonecutterScreenHandler,
};
use pumpkin_data::{data_component_impl::MapIdImpl, item::Item, item_stack::ItemStack};
use pumpkin_protocol::java::server::play::SlotActionType;

fn check_result(menu: &mut dyn ScreenHandler, player: &Player, slot: i32, item: &'static Item) {
    // Independent expectations from ResultContainer.removeItem / Slot.tryRemove.
    *menu.get_behaviour().cursor_stack.lock().unwrap() = menu.get_behaviour().slots[slot as usize]
        .get_stack()
        .copy_with_count(63);
    menu.on_slot_click(slot, 1, SlotActionType::Pickup, player);
    assert_eq!(
        menu.get_behaviour().cursor_stack.lock().unwrap().item_count,
        63
    );
    assert_eq!(
        menu.get_behaviour().slots[slot as usize]
            .get_stack()
            .item_count,
        2
    );
    *menu.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::EMPTY.clone();
    menu.on_slot_click(slot, 0, SlotActionType::Throw, player);
    assert_eq!(player.drops.lock().unwrap()[0].item_count, 2);
    menu.on_slot_click(slot, 1, SlotActionType::Pickup, player);
    let cursor = menu.get_behaviour().cursor_stack.lock().unwrap();
    assert_eq!((cursor.item, cursor.item_count), (item, 2));
}

#[test]
fn cartography_whole_result_q_right_click_and_capacity() {
    let player = Player::new(1);
    let mut menu = CartographyTableScreenHandler::new(1, &player.inventory);
    let mut map = ItemStack::new(3, &Item::FILLED_MAP);
    map.set_data_component(MapIdImpl { id: 0 });
    menu.input_inventory.set_stack(0, map.clone());
    menu.input_inventory
        .set_stack(1, ItemStack::new(3, &Item::MAP));
    menu.slots_changed(&player);
    let mut cursor = map.copy_with_count(63);
    *menu.get_behaviour().cursor_stack.lock().unwrap() = cursor.clone();
    menu.on_slot_click(2, 1, SlotActionType::Pickup, &player);
    assert_eq!(menu.input_inventory.get_stack(0).item_count, 3);
    cursor.item_count = 0;
    *menu.get_behaviour().cursor_stack.lock().unwrap() = cursor;
    check_result(&mut menu, &player, 2, &Item::FILLED_MAP);
    assert_eq!(menu.input_inventory.get_stack(0).item_count, 1);
    assert_eq!(menu.input_inventory.get_stack(1).item_count, 1);
}

#[test]
fn stonecutter_whole_result_q_right_click_and_capacity() {
    let player = Player::new(1);
    let mut menu = StonecutterScreenHandler::new(1, &player.inventory);
    menu.input_inventory
        .set_stack(0, ItemStack::new(3, &Item::STONE));
    let index = pumpkin_data::recipes::RECIPES_STONECUTTING
        .iter()
        .filter(|recipe| recipe.ingredient.match_item(&Item::STONE))
        .position(|recipe| recipe.result.assemble(None, 0).item == &Item::STONE_SLAB)
        .unwrap();
    assert!(menu.on_button_click(&player, index as i32));
    check_result(&mut menu, &player, 1, &Item::STONE_SLAB);
    assert_eq!(menu.input_inventory.get_stack(0).item_count, 1);
}
