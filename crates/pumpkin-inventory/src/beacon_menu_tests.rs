use crate::{
    Inventory, beacon_screen_handler::create_beacon_handler, ext_review_tests::Player,
    screen_handler::ScreenHandler, window_property::PropertyDelegate,
};
use pumpkin_data::{data_component_impl::CustomNameImpl, item::Item, item_stack::ItemStack};
use pumpkin_protocol::java::server::play::SlotActionType;
use pumpkin_util::text::TextComponent;
use std::sync::Arc;

struct BeaconData;

impl PropertyDelegate for BeaconData {
    fn get_property(&self, _index: i32) -> i32 {
        0
    }

    fn set_property(&self, _index: i32, _value: i32) {}

    fn get_properties_size(&self) -> i32 {
        3
    }
}

fn named_stone(count: u8, name: &str) -> ItemStack {
    let mut stack = ItemStack::new(count, &Item::STONE);
    stack.set_data_component(CustomNameImpl {
        name: TextComponent::text(name.to_owned()),
    });
    stack
}

#[test]
fn b1020b_g03_beacon_shift_click_partial_fit_conserves_components() {
    let player = Player::new(0);
    let mut menu = create_beacon_handler(1, &player.inventory, Arc::new(BeaconData));
    let source = named_stone(10, "matching component");
    let mut matching = source.clone();
    matching.item_count = 60;
    player.inventory.set_stack(9, source.clone());
    player.inventory.set_stack(0, matching);
    for index in 1..9 {
        player
            .inventory
            .set_stack(index, named_stone(64, "different component"));
    }
    menu.on_slot_click(1, 0, SlotActionType::QuickMove, &player);
    let remaining = player.inventory.get_stack(9);
    assert_eq!(
        remaining.item_count, 6,
        "only destination capacity may move"
    );
    assert!(remaining.are_items_and_components_equal(&source));
    let filled = player.inventory.get_stack(0);
    assert_eq!(filled.item_count, 64);
    assert!(filled.are_items_and_components_equal(&source));
    for index in 1..9 {
        assert!(
            player
                .inventory
                .get_stack(index)
                .are_equal(&named_stone(64, "different component"))
        );
    }
    assert!(menu.inventory.get_stack(0).is_empty());
    assert!(player.drops.lock().unwrap().is_empty());
}

#[test]
fn b1020b_g03_beacon_singleton_payment_preserves_stock_control() {
    let player = Player::new(0);
    let mut menu = create_beacon_handler(1, &player.inventory, Arc::new(BeaconData));
    let mut payment = ItemStack::new(1, &Item::DIAMOND);
    payment.set_data_component(CustomNameImpl {
        name: TextComponent::text("payment component"),
    });
    player.inventory.set_stack(0, payment.clone());
    menu.on_slot_click(28, 0, SlotActionType::QuickMove, &player);
    assert!(player.inventory.get_stack(0).is_empty());
    assert!(menu.inventory.get_stack(0).are_equal(&payment));
    menu.on_slot_click(0, 0, SlotActionType::QuickMove, &player);
    assert!(menu.inventory.get_stack(0).is_empty());
    assert!(player.inventory.get_stack(8).are_equal(&payment));
    assert!(player.drops.lock().unwrap().is_empty());
}
