use super::*;
use crate::entity::death_test_world::DeathTestWorld;
use pumpkin_inventory::{Inventory, screen_handler::ScreenHandler};
use pumpkin_protocol::{
    codec::item_stack_seralizer::{ItemStackSerializer, OptionalItemStackHash},
    java::client::play::MerchantOffer,
    java::server::play::{SClickSlot, SlotActionType},
    ser::NetworkWriteExt,
};
use std::{borrow::Cow, sync::mpsc, time::Duration};

fn offer(uses: i32, output_count: u8) -> MerchantOffer {
    MerchantOffer {
        base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::EMERALD))),
        output: ItemStackSerializer(Cow::Owned(ItemStack::new(output_count, &Item::REDSTONE))),
        cost_b: None,
        reward_exp: false,
        uses,
        max_uses: 12,
        xp: 2,
        special_price: 0,
        price_multiplier: 0.0,
        demand: 0,
    }
}

fn predicted_stack(item: &Item, count: i32) -> OptionalItemStackHash {
    // Decode actual component-free 26.3 prediction bytes via the public codec.
    let mut bytes = Vec::new();
    bytes.put_bool(true).unwrap();
    bytes.put_var_int(&VarInt(i32::from(item.id))).unwrap();
    bytes.put_var_int(&VarInt(count)).unwrap();
    bytes.put_var_int(&VarInt(0)).unwrap();
    bytes.put_var_int(&VarInt(0)).unwrap();
    OptionalItemStackHash::read(&mut bytes.as_slice()).unwrap()
}

async fn prepare_trade() -> (DeathTestWorld, Arc<VillagerEntity>, Arc<Player>, SClickSlot) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = fixture.player("TradeCallbackTest");
    player.set_client_loaded(true);
    player.get_entity().set_pos(Vector3::new(1.5, 80.0, 0.5));
    let villager = VillagerEntity::new(Entity::new(
        world,
        Vector3::new(0.5, 80.0, 0.5),
        &EntityType::VILLAGER,
    ));
    villager.offers.lock().unwrap().push(offer(0, 2));
    let inventory = player.inventory();
    inventory.set_stack(0, ItemStack::new(64, &Item::EMERALD));
    let screen = villager
        .create_screen_handler(1, inventory, player.as_ref())
        .unwrap();
    *player.current_screen_handler.lock().unwrap() = screen.clone();
    screen
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<MerchantScreenHandler>()
        .unwrap()
        .set_selected_offer(0);
    let revision = {
        let handler = screen.lock().unwrap();
        assert!(handler.can_use(player.as_ref()));
        handler.get_behaviour().revision.load(Ordering::Relaxed)
    };
    let packet = SClickSlot {
        sync_id: VarInt(1),
        revision: VarInt(revision as i32),
        slot: 2,
        button: SClickSlot::BUTTON_LEFT,
        mode: SlotActionType::Pickup,
        length_of_array: VarInt(1),
        array_of_changed_slots: vec![(0, predicted_stack(&Item::EMERALD, 63))],
        carried_item: predicted_stack(&Item::REDSTONE, 2),
    };
    (fixture, villager, player, packet)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_trade_player_click_returns_and_preserves_payment() {
    let (fixture, villager, player, packet) = prepare_trade().await;
    let (sent, received) = mpsc::channel();
    let server = fixture.server.clone();
    let worker = std::thread::spawn(move || {
        player.on_slot_click(packet, &server);
        let screen = player.current_screen_handler.lock().unwrap().clone();
        let screen = screen.lock().unwrap();
        let handler = screen
            .as_any()
            .downcast_ref::<MerchantScreenHandler>()
            .unwrap();
        sent.send((
            handler.get_behaviour().cursor_stack.lock().unwrap().clone(),
            handler.inventory.get_stack(0),
            handler.offers[0].uses,
        ))
        .unwrap();
    });
    let (cursor, payment, handler_uses) = received
        .recv_timeout(Duration::from_secs(5))
        .expect("successful trade player click must return after its callback");
    worker.join().unwrap();
    assert_eq!(cursor.item.id, Item::REDSTONE.id);
    assert_eq!(cursor.item_count, 2);
    assert_eq!(payment.item.id, Item::EMERALD.id);
    assert_eq!(payment.item_count, 63);
    assert_eq!(handler_uses, 1);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 1);
    assert_eq!(villager.xp.load(Ordering::Relaxed), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_assigned_job_sites_restock_and_respect_the_second_restock_cooldown() {
    let fixture = DeathTestWorld::new().await;
    let site = BlockPos::new(0, 80, 0);
    let villager = VillagerEntity::new(Entity::new(
        fixture.world(),
        site.to_centered_f64(),
        &EntityType::VILLAGER,
    ));
    *villager.job_site.lock().unwrap() = Some(site);
    villager.job_site_pending.store(true, Ordering::Relaxed);
    villager.offers.lock().unwrap().push(offer(12, 1));
    villager.work_at_poi(3_000, 0);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 12);
    assert_eq!(villager.restocks_today.load(Ordering::Relaxed), 0);
    assert_eq!(villager.last_restock_time.load(Ordering::Relaxed), 0);
    villager.job_site_pending.store(false, Ordering::Relaxed);
    villager.work_at_poi(3_000, 0);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 0);
    assert_eq!(villager.restocks_today.load(Ordering::Relaxed), 1);
    assert_eq!(villager.last_restock_time.load(Ordering::Relaxed), 3_000);
    villager.offers.lock().unwrap()[0].uses = 12;
    villager.work_at_poi(5_400, 0);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 12);
    villager.work_at_poi(5_401, 0);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 0);
    assert_eq!(villager.restocks_today.load(Ordering::Relaxed), 2);
    villager.offers.lock().unwrap()[0].uses = 12;
    villager.work_at_poi(7_802, 0);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 12);
    assert_eq!(villager.restocks_today.load(Ordering::Relaxed), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_job_site_does_not_consume_work_start_cooldown() {
    let fixture = DeathTestWorld::new().await;
    let site = BlockPos::new(0, 80, 0);
    let villager = VillagerEntity::new(Entity::new(
        fixture.world(),
        site.to_centered_f64(),
        &EntityType::VILLAGER,
    ));
    *villager.job_site.lock().unwrap() = Some(site);
    villager.job_site_pending.store(true, Ordering::Relaxed);
    // Repeated eligible start attempts make the old random branch observable.
    for time in (3_000..30_000).step_by(300) {
        villager.work_at_job_site(time, 3_000, 0);
        assert_eq!(villager.last_worked_at_poi.load(Ordering::Relaxed), 0);
    }
    villager.job_site_pending.store(false, Ordering::Relaxed);
    *villager.job_site.lock().unwrap() = None;
    for time in (3_000..30_000).step_by(300) {
        villager.work_at_job_site(time, 3_000, 0);
        assert_eq!(villager.last_worked_at_poi.load(Ordering::Relaxed), 0);
    }
    fixture.server.shutdown().await;
}
