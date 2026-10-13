use crate::entity::death_test_world::DeathTestWorld;
use crate::entity::mob::interaction::MobInteraction;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mob_use_player_item_applies_the_component_remainder_to_its_source_hand() {
    let fixture = DeathTestWorld::new().await;
    let player = TestPlayer::new(&fixture.world());
    let mut input = ItemStack::new(2, &Item::HONEY_BOTTLE);
    assert!(
        input
            .get_data_component::<pumpkin_data::data_component_impl::UseRemainderImpl>()
            .is_some()
    );
    player.player.inventory().set_stack(40, input.clone());

    let mut interaction = MobInteraction::new(&player.player, &input, Hand::Left);
    assert!(interaction.use_player_item(&player.player, &mut input));

    assert_eq!(input.item, &Item::HONEY_BOTTLE);
    assert_eq!(input.item_count, 1);
    assert!(player.player.inventory().get_stack(40).are_equal(&input));
    let glass_bottles = (0..player.player.inventory().size())
        .map(|slot| player.player.inventory().get_stack(slot))
        .filter(|stack| stack.item == &Item::GLASS_BOTTLE)
        .map(|stack| u32::from(stack.item_count))
        .sum::<u32>();
    assert_eq!(glass_bottles, 1);

    fixture.server.shutdown().await;
}
