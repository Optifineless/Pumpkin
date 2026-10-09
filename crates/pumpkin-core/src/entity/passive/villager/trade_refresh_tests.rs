use super::*;
use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_inventory::{Inventory, screen_handler::ScreenHandler};
use pumpkin_protocol::java::server::play::SlotActionType;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn level_up_with_open_screen_can_execute_new_trade_immediately() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let villager = VillagerEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::VILLAGER,
    ));
    villager.set_villager_data(VillagerData::new(
        VillagerType::Plains,
        VillagerProfession::Farmer,
        1,
    ));
    villager.generate_trades(VillagerProfession::Farmer, 1);
    // VillagerData.getMaxXpPerLevel(1), from NEXT_LEVEL_XP_THRESHOLDS (Vanilla line 19).
    villager.offers.lock().unwrap()[0].xp = 10;
    let screen = villager
        .create_screen_handler(1, player.player.inventory(), player.player.as_ref())
        .unwrap();
    *player.player.current_screen_handler.lock().unwrap() = screen.clone();
    {
        let mut guard = screen.lock().unwrap();
        let handler = guard
            .as_any_mut()
            .downcast_mut::<MerchantScreenHandler>()
            .unwrap();
        let original_count = handler.offers.len();
        let cost = handler.offers[0].base_cost_a.0.as_ref().clone();
        player.player.inventory().set_stack(9, cost);
        handler.set_selected_offer(0);
        handler.on_slot_click(2, 0, SlotActionType::Pickup, player.player.as_ref());
        assert_eq!(villager.villager_data.lock().unwrap().level.0, 2);
        assert!(
            handler.offers.len() > original_count,
            "open handler did not gain level-up offers"
        );
        let new_offer = handler.offers[original_count].clone();
        player
            .player
            .inventory()
            .set_stack(10, new_offer.base_cost_a.0.as_ref().clone());
        if let Some(cost) = &new_offer.cost_b {
            player
                .player
                .inventory()
                .set_stack(11, cost.0.as_ref().clone());
        }
        handler.set_selected_offer(original_count);
        assert!(
            villager
                .merchant_inventory
                .get_stack(2)
                .are_equal(&new_offer.output.0)
        );
        handler.on_slot_click(2, 0, SlotActionType::QuickMove, player.player.as_ref());
        assert_eq!(villager.offers.lock().unwrap()[original_count].uses, 1);
        assert_eq!(handler.offers[original_count].uses, 1);
    };
    fixture.server.shutdown().await;
}
