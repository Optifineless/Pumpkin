use super::MobInteraction;
use crate::entity::death_test_world::DeathTestWorld;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interaction_result_returns_to_the_captured_hotbar_slot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let before = ItemStack::new(1, &Item::BUCKET);
    player.player.inventory().set_stack(0, before.clone());
    player.player.inventory().set_selected_slot(0);

    let interaction = MobInteraction::new(&player.player, &before, Hand::Right);
    let after = ItemStack::new(1, &Item::MILK_BUCKET);
    let other_slot = ItemStack::new(1, &Item::DIAMOND);
    player.player.inventory().set_stack(1, other_slot.clone());
    player.player.inventory().set_selected_slot(1);

    assert!(interaction.finish(&player.player, &after));
    assert!(player.player.inventory().get_stack(0).are_equal(&after));
    assert!(
        player
            .player
            .inventory()
            .get_stack(1)
            .are_equal(&other_slot)
    );
    fixture.server.shutdown().await;
}
