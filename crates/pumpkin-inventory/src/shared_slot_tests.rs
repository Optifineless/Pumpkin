use crate::{
    SimpleInventory, ext_review_tests::Player,
    generic_container_screen_handler::create_generic_9x3,
    player::player_screen_handler::PlayerScreenHandler, screen_handler::ScreenHandler,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use std::sync::Arc;

#[test]
fn shared_slot_snapshot_preserves_every_index_of_the_same_inventory() {
    let player = Player::new(1);
    let mut closing = create_generic_9x3(
        1,
        &player.inventory,
        Arc::new(SimpleInventory::new(27)),
        &player,
    );
    let mut replacement = PlayerScreenHandler::new(&player.inventory, None, 0, None);
    // These are distinct player indices in the same inventory, and in different menu slots.
    for (index, item, count) in [
        (0, &Item::BREAD, 2),
        (9, &Item::EMERALD, 3),
        (35, &Item::DIRT, 4),
    ] {
        let slot = closing
            .get_behaviour()
            .slots
            .iter()
            .position(|slot| {
                slot.get_index() == index
                    && Arc::ptr_eq(
                        &slot.get_inventory(),
                        &(player.inventory.clone() as Arc<dyn crate::Inventory>),
                    )
            })
            .unwrap();
        let behaviour = closing.get_behaviour_mut();
        behaviour.tracked_stacks[slot] = ItemStack::new(count, item);
        behaviour.previous_tracked_stacks[slot].set_received_stack(ItemStack::new(count + 1, item));
    }
    replacement.copy_shared_slots(closing.shared_slot_state());
    for (index, item, count) in [
        (0, &Item::BREAD, 2),
        (9, &Item::EMERALD, 3),
        (35, &Item::DIRT, 4),
    ] {
        let behaviour = replacement.get_behaviour();
        let slot = behaviour
            .slots
            .iter()
            .position(|slot| {
                slot.get_index() == index
                    && Arc::ptr_eq(
                        &slot.get_inventory(),
                        &(player.inventory.clone() as Arc<dyn crate::Inventory>),
                    )
            })
            .unwrap();
        assert!(behaviour.tracked_stacks[slot].are_equal(&ItemStack::new(count, item)));
        assert!(
            behaviour.previous_tracked_stacks[slot]
                .received_stack
                .as_ref()
                .unwrap()
                .are_equal(&ItemStack::new(count + 1, item))
        );
    }
}
