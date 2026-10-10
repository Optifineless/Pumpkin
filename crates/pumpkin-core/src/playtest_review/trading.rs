use super::*;
use crate::{
    entity::passive::villager::{VillagerEntity, data::VillagerData},
    net::java::combat_test_support::TestPlayer,
};
use pumpkin_data::villager::{VillagerProfession, VillagerType};
use pumpkin_inventory::{
    merchant::merchant_screen_handler::MerchantScreenHandler,
    screen_handler::{ScreenHandler, ScreenHandlerFactory},
};
use pumpkin_protocol::java::server::play::SlotActionType;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn level_up_offer_is_immediately_tradeable_in_the_open_menu() {
    level_up_trade(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hero_discount_applies_to_newly_unlocked_trade_in_open_menu() {
    level_up_trade(true).await;
}

async fn level_up_trade(hero: bool) {
    let fixture = Fixture::new();
    let player = TestPlayer::new(&fixture.world);
    fixture
        .world
        .players
        .store(Arc::new(vec![player.player.clone()]));
    if hero {
        player
            .player
            .living_entity
            .add_effect(pumpkin_data::potion::Effect {
                effect_type: &pumpkin_data::effect::StatusEffect::HERO_OF_THE_VILLAGE,
                duration: 600,
                amplifier: 0,
                ambient: false,
                show_particles: false,
                show_icon: false,
                blend: false,
            });
    }
    let entity = fixture.entity(&EntityType::VILLAGER);
    let merchant = entity.cast_any().downcast_ref::<VillagerEntity>().unwrap();
    merchant.set_villager_data(VillagerData::new(
        VillagerType::Plains,
        VillagerProfession::Fletcher,
        1,
    ));
    let trade = &pumpkin_data::villager::TRADES_FLETCHER_LEVEL_1[0];
    *merchant.offers.lock().unwrap() = vec![pumpkin_protocol::java::client::play::MerchantOffer {
        base_cost_a: ItemStack::new(trade.wants.count as u8, trade.wants.item).into(),
        output: ItemStack::new(trade.gives.count as u8, trade.gives.item).into(),
        cost_b: None,
        reward_exp: true,
        uses: 0,
        max_uses: trade.max_uses,
        xp: trade.xp,
        special_price: 0,
        price_multiplier: trade.price_multiplier,
        demand: 0,
    }];
    let novice = 0;
    merchant.xp.store(8, std::sync::atomic::Ordering::Relaxed);
    let initial_count = merchant.offers.lock().unwrap().len();
    player
        .player
        .inventory
        .set_stack(9, ItemStack::new(32, &Item::STICK));
    let handler = merchant
        .create_screen_handler(1, &player.player.inventory, player.player.as_ref())
        .unwrap();
    *player.player.current_screen_handler.lock().unwrap() = handler.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    let owner = player.player.clone();
    let worker = std::thread::spawn(move || {
        let _entered = runtime.enter();
        let mut guard = handler.lock().unwrap();
        let screen = guard
            .as_any_mut()
            .downcast_mut::<MerchantScreenHandler>()
            .unwrap();
        screen.set_selected_offer(novice);
        screen.on_slot_click(2, 0, SlotActionType::QuickMove, owner.as_ref());
        assert!(screen.offers.len() > initial_count);
        assert_eq!(screen.offers[novice].uses, 1);
        if hero {
            assert!(
                screen.offers[initial_count..]
                    .iter()
                    .all(|offer| offer.special_price < 0)
            );
        }
        // Apprentice fletcher offers include flint -> emerald. Feed its actual live cost.
        let index = (initial_count..screen.offers.len())
            .find(|&i| screen.offers[i].base_cost_a.0.item == &Item::FLINT)
            .unwrap();
        owner
            .inventory
            .set_stack(10, screen.offers[index].base_cost_a.0.as_ref().clone());
        screen.set_selected_offer(index);
        assert!(!screen.inventory.get_stack(2).is_empty());
        screen.on_slot_click(2, 0, SlotActionType::QuickMove, owner.as_ref());
        assert_eq!(screen.offers[index].uses, 1);
        if hero {
            // Vanilla apprentice flint trade: 26 minus Hero I's 7-item discount costs 19.
            assert_eq!(screen.inventory.get_stack(0).item_count, 7);
        } else {
            assert!(screen.inventory.get_stack(0).is_empty());
        }
        send.send(index).unwrap();
    });
    let new_index = receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    worker.join().unwrap();
    assert_eq!(merchant.offers.lock().unwrap()[novice].uses, 1);
    assert_eq!(merchant.offers.lock().unwrap()[new_index].uses, 1);
    assert_eq!(merchant.villager_data.lock().unwrap().level.0, 2);
    fixture.shutdown().await;
}
