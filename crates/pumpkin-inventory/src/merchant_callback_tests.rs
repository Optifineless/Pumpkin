use crate::{
    Inventory, SimpleInventory,
    ext_review_tests::Player,
    merchant::merchant_screen_handler::MerchantScreenHandler,
    screen_handler::{ScreenHandler, ScreenMutex},
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer,
    java::{client::play::MerchantOffer, server::play::SlotActionType},
};
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn menu(player: &Player) -> Arc<ScreenMutex<MerchantScreenHandler>> {
    let inventory = Arc::new(SimpleInventory::new(3));
    inventory.set_stack(0, ItemStack::new(1, &Item::EMERALD));
    let offer = MerchantOffer {
        base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::EMERALD))),
        output: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::BREAD))),
        cost_b: None,
        reward_exp: false,
        uses: 0,
        max_uses: 1,
        xp: 0,
        special_price: 0,
        price_multiplier: 0.0,
        demand: 0,
    };
    let mut menu = MerchantScreenHandler::new(1, &player.inventory, inventory, vec![offer]);
    menu.update_result_slot();
    Arc::new(ScreenMutex::new(menu))
}

#[test]
fn non_core_player_trade_callback_reenters_unlocked_menu() {
    let player = Player::new(1);
    let menu = menu(&player);
    let calls = Arc::new(AtomicUsize::new(0));
    let weak = Arc::downgrade(&menu);
    let count = calls.clone();
    menu.lock().unwrap().on_trade = Some(Arc::new(move |_| {
        let menu = weak.upgrade().unwrap();
        let menu = menu.try_lock().unwrap();
        assert_eq!(menu.offers[0].uses, 1);
        count.fetch_add(1, Ordering::Relaxed);
    }));
    menu.lock()
        .unwrap()
        .on_slot_click(2, 0, SlotActionType::Pickup, &player);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    player.flush_menu_callbacks();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn non_core_player_close_callback_reenters_unlocked_menu() {
    let player = Player::new(1);
    let menu = menu(&player);
    let calls = Arc::new(AtomicUsize::new(0));
    let weak = Arc::downgrade(&menu);
    let count = calls.clone();
    menu.lock().unwrap().on_close = Some(Arc::new(move || {
        drop(weak.upgrade().unwrap().try_lock().unwrap());
        count.fetch_add(1, Ordering::Relaxed);
    }));
    menu.lock().unwrap().on_closed(&player);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    player.flush_menu_callbacks();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
