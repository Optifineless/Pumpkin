use crate::{
    Inventory, ext_review_tests::Player, screen_handler::ScreenHandler,
    smithing_table_screen_handler::SmithingTableScreenHandler,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_protocol::java::server::play::SlotActionType;

#[test]
fn smithing_occupied_inputs_use_inventory_fallback() {
    for (input, item) in [
        (0, &Item::NETHERITE_UPGRADE_SMITHING_TEMPLATE),
        (1, &Item::DIAMOND_CHESTPLATE),
        (2, &Item::NETHERITE_INGOT),
    ] {
        for hotbar in [false, true] {
            let player = Player::new(0);
            let mut menu = SmithingTableScreenHandler::new(1, &player.inventory);
            menu.input_inventory
                .set_stack(input, ItemStack::new(1, item));
            let index = if hotbar { 0 } else { 9 };
            player.inventory.set_stack(index, ItemStack::new(1, item));
            menu.on_slot_click(
                if hotbar { 31 } else { 4 },
                0,
                SlotActionType::QuickMove,
                &player,
            );
            assert_eq!(menu.input_inventory.get_stack(input).item_count, 1);
            assert!(player.inventory.get_stack(index).is_empty());
            assert_eq!(
                player.inventory.get_stack(if hotbar { 9 } else { 0 }).item,
                item
            );
        }
    }
}
