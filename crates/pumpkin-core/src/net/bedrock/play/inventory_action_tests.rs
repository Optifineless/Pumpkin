use super::*;
use pumpkin_data::item::Item;
use pumpkin_protocol::bedrock::server::inventory_transaction::InventoryAction;
use pumpkin_protocol::codec::var_int::VarInt;

fn descriptor(id: i32, stack_size: u16) -> NetworkItemDescriptor {
    NetworkItemDescriptor {
        id: VarInt(id),
        stack_size,
        ..NetworkItemDescriptor::default()
    }
}

fn action(
    window_id: Option<i32>,
    inventory_slot: u32,
    old_item: NetworkItemDescriptor,
    new_item: NetworkItemDescriptor,
) -> InventoryAction {
    InventoryAction {
        source_type: 0,
        window_id,
        source_flags: None,
        inventory_slot,
        old_item,
        new_item,
    }
}

#[test]
fn selected_hotbar_transaction_action_is_not_consumed_twice() {
    assert!(transaction_consumes_selected_hotbar(
        &[action(Some(0), 4, descriptor(5, 2), descriptor(5, 1))],
        4
    ));
    assert!(transaction_consumes_selected_hotbar(
        &[action(Some(0), 4, descriptor(5, 1), descriptor(0, 0))],
        4
    ));
    assert!(!transaction_consumes_selected_hotbar(
        &[action(Some(0), 3, descriptor(5, 2), descriptor(5, 1))],
        4
    ));
    assert!(!transaction_consumes_selected_hotbar(
        &[action(Some(119), 0, descriptor(5, 2), descriptor(5, 1))],
        4
    ));
    assert!(!transaction_consumes_selected_hotbar(
        &[action(Some(0), 4, descriptor(5, 1), descriptor(5, 1))],
        4
    ));
    assert!(!transaction_consumes_selected_hotbar(
        &[action(None, 4, descriptor(5, 2), descriptor(5, 1))],
        4
    ));
}

#[test]
fn consumed_hand_stack_is_written_back_only_when_server_authoritative() {
    let before = ItemStack::new(2, &Item::GLOWSTONE);
    let after = ItemStack::new(1, &Item::GLOWSTONE);

    assert!(should_write_back_consumed_stack(
        &before, &after, true, false
    ));
    assert!(!should_write_back_consumed_stack(
        &before, &after, true, true
    ));
    assert!(!should_write_back_consumed_stack(
        &before, &after, false, false
    ));
    assert!(!should_write_back_consumed_stack(
        &before, &before, true, false
    ));
}
