use crate::{
    entity::EntityBase, net::java::combat_test_support::TestPlayer, server::combat_test_support,
};
use pumpkin_data::{data_component_impl::MapIdImpl, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItem};
use pumpkin_util::{GameMode, Hand, math::vector3::Vector3};

fn filled_map_id(stack: &ItemStack) -> i32 {
    stack.get_data_component::<MapIdImpl>().unwrap().id
}

fn map_center(server: &crate::server::Server, map_id: i32) -> (i32, i32) {
    let map = server.map_manager.get_map(map_id).unwrap();
    let map = map.lock().unwrap();
    (map.center_x, map.center_z)
}

fn find_filled_map(inventory: &PlayerInventory, map_id: i32) -> ItemStack {
    (0..inventory.size())
        .map(|slot| inventory.get_slot(slot))
        .find(|stack| {
            stack.item == &Item::FILLED_MAP
                && stack
                    .get_data_component::<MapIdImpl>()
                    .is_some_and(|component| component.id == map_id)
        })
        .unwrap()
}

fn first_empty_slot(inventory: &PlayerInventory) -> usize {
    (0..inventory.size())
        .find(|slot| inventory.get_slot(*slot).is_empty())
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(clippy::unwrap_used, reason = "Test fixture construction must succeed")]
async fn empty_map_use_aligns_scale_zero_centers_and_preserves_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let client = TestPlayer::new(&world);
    let player = client.player.clone();
    let inventory = player.inventory();
    let use_item = |sequence| SUseItem {
        hand: VarInt(0),
        sequence: VarInt(sequence),
        yaw: 0.0,
        pitch: 0.0,
    };

    // Two origins inside the same vanilla scale-zero tile share the (0, 0) center.
    player.get_entity().set_pos(Vector3::new(10.5, 64.0, 10.5));
    inventory.set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::MAP));
    client
        .client()
        .handle_use_item(&player, &use_item(0), &server);

    assert_eq!(inventory.get_stack_in_hand(Hand::Right).item, &Item::MAP);
    assert_eq!(inventory.get_stack_in_hand(Hand::Right).item_count, 1);
    let first_in_inventory = find_filled_map(inventory.as_ref(), 0);
    assert_eq!(first_in_inventory.item, &Item::FILLED_MAP);
    let first_id = filled_map_id(&first_in_inventory);
    assert_eq!(first_id, 0);
    assert_eq!(map_center(&server, first_id), (0, 0));

    player.get_entity().set_pos(Vector3::new(30.5, 64.0, 45.5));
    client
        .client()
        .handle_use_item(&player, &use_item(1), &server);

    let second_in_hand = inventory.get_stack_in_hand(Hand::Right);
    assert_eq!(second_in_hand.item, &Item::FILLED_MAP);
    let second_id = filled_map_id(&second_in_hand);
    assert_eq!(second_id, 1);
    assert_eq!(map_center(&server, second_id), (0, 0));
    assert_eq!(
        filled_map_id(&find_filled_map(inventory.as_ref(), first_id)),
        first_id
    );

    // The negative boundary distinguishes floor alignment from truncation toward zero.
    let second_storage_slot = first_empty_slot(inventory.as_ref());
    inventory.set_slot(second_storage_slot, second_in_hand);
    inventory.set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::MAP));
    player
        .get_entity()
        .set_pos(Vector3::new(-64.5, 64.0, -64.5));
    client
        .client()
        .handle_use_item(&player, &use_item(2), &server);

    let third_in_hand = inventory.get_stack_in_hand(Hand::Right);
    let third_id = filled_map_id(&third_in_hand);
    assert_eq!(third_id, 2);
    assert_eq!(map_center(&server, third_id), (-128, -128));
    assert_eq!(
        filled_map_id(&find_filled_map(inventory.as_ref(), first_id)),
        first_id
    );
    assert_eq!(
        filled_map_id(&find_filled_map(inventory.as_ref(), second_id)),
        second_id
    );

    // Creative use keeps the empty map and inserts a separate filled map.
    let third_storage_slot = first_empty_slot(inventory.as_ref());
    inventory.set_slot(third_storage_slot, third_in_hand);
    inventory.set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::MAP));
    player.gamemode.store(GameMode::Creative);
    client
        .client()
        .handle_use_item(&player, &use_item(3), &server);

    assert_eq!(inventory.get_stack_in_hand(Hand::Right).item, &Item::MAP);
    assert_eq!(inventory.get_stack_in_hand(Hand::Right).item_count, 1);
    let creative_output = find_filled_map(inventory.as_ref(), 3);
    assert_eq!(creative_output.item, &Item::FILLED_MAP);
    assert_eq!(filled_map_id(&creative_output), 3);

    assert!(world.level.shutdown().await.is_ok());
}
