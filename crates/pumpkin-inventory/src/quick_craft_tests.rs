use crate::{
    Inventory, SimpleInventory, ext_review_tests::Player, screen_handler::ScreenHandler,
    shulker_box_screen_handler::ShulkerBoxScreenHandler,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_protocol::java::server::play::SlotActionType;
use std::sync::Arc;

#[test]
fn quick_craft_requires_start_stores_mode_and_bounds_unique_destinations() {
    let player = Player::new(0);
    let inventory = Arc::new(SimpleInventory::new(27));
    let mut menu = ShulkerBoxScreenHandler::new(1, &player.inventory, inventory.clone(), &player);
    *menu.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::new(8, &Item::STONE);
    menu.on_slot_click(0, 1, SlotActionType::QuickCraft, &player);
    assert!(menu.get_behaviour().drag_slots.is_empty());
    menu.on_slot_click(-999, 0, SlotActionType::QuickCraft, &player);
    for _ in 0..1000 {
        menu.on_slot_click(0, 1, SlotActionType::QuickCraft, &player);
    }
    assert_eq!(menu.get_behaviour().drag_slots.len(), 1);
    menu.on_slot_click(1, 5, SlotActionType::QuickCraft, &player);
    // Later packets' mode bits cannot switch an even drag to a one-per-slot drag.
    menu.on_slot_click(-999, 6, SlotActionType::QuickCraft, &player);
    assert_eq!(
        (
            inventory.get_stack(0).item_count,
            inventory.get_stack(1).item_count
        ),
        (4, 4)
    );
    assert!(menu.get_behaviour().cursor_stack.lock().unwrap().is_empty());
    *menu.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::new(2, &Item::STONE);
    menu.on_slot_click(-999, 0, SlotActionType::QuickCraft, &player);
    for i in 2..27 {
        menu.on_slot_click(i, 1, SlotActionType::QuickCraft, &player);
    }
    assert_eq!(menu.get_behaviour().drag_slots.len(), 2);
    menu.on_slot_click(0, 0, SlotActionType::Pickup, &player);
    assert!(menu.get_behaviour().drag_slots.is_empty());
    assert_eq!(inventory.get_stack(0).item_count, 4);
    menu.on_slot_click(-999, 2, SlotActionType::QuickCraft, &player);
    assert!(inventory.get_stack(2).is_empty());
}
