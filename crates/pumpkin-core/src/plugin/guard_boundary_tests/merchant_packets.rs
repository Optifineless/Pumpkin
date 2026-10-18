use super::*;
use crate::entity::{Entity, EntityBase, passive::villager::VillagerEntity};
use pumpkin_data::entity::EntityType;
use pumpkin_inventory::{
    beacon_screen_handler::BeaconScreenHandler,
    merchant::merchant_screen_handler::MerchantScreenHandler, screen_handler::ScreenHandlerFactory,
};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer, java::client::play::MerchantOffer,
};
use pumpkin_util::math::vector3::Vector3;
use std::borrow::Cow;

fn merchant(fixture: &Fixture) -> (Arc<VillagerEntity>, SharedScreenHandler) {
    let player = &fixture.player.player;
    player.get_entity().set_pos(Vector3::new(0.5, 80.0, 0.5));
    let villager = VillagerEntity::new(Entity::new(
        fixture.world.clone(),
        player.position(),
        &EntityType::VILLAGER,
    ));
    villager.offers.lock().unwrap().push(MerchantOffer {
        base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::EMERALD))),
        output: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::BREAD))),
        cost_b: None,
        reward_exp: true,
        uses: 0,
        max_uses: 1,
        xp: 0,
        special_price: 0,
        price_multiplier: 0.0,
        demand: 0,
    });
    player
        .inventory
        .set_stack(0, ItemStack::new(2, &Item::EMERALD));
    let screen = villager
        .create_screen_handler(1, &player.inventory, player.as_ref())
        .unwrap();
    *player.current_screen_handler.lock().unwrap() = screen.clone();
    fixture.player.client().handle_select_trade(
        player,
        &pumpkin_protocol::java::server::play::SSelectTrade {
            selected_slot: VarInt(0),
        },
    );
    (villager, screen)
}

fn click(fixture: &Fixture, mode: SlotActionType) {
    fixture.player.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(1),
            revision: VarInt(0),
            slot: 2,
            button: 0,
            mode,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &fixture.server,
    );
}

struct SpawnReentry(Arc<AtomicUsize>);
impl EventHandler<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent>
    for SpawnReentry
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        for player in event.world.players.load().iter() {
            assert_menu_unlocked(player);
            let screen = player.current_screen_handler.lock().unwrap().clone();
            let screen = screen.lock().unwrap();
            let merchant = screen
                .as_any()
                .downcast_ref::<MerchantScreenHandler>()
                .unwrap();
            assert_eq!(merchant.offers[0].uses, 1);
            assert_eq!(merchant.inventory.get_stack(0).item_count, 1);
        }
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

async fn reward_trade(mode: SlotActionType, reentry: bool) {
    let fixture = Fixture::new();
    let (villager, screen) = merchant(&fixture);
    let calls = Arc::new(AtomicUsize::new(0));
    if reentry {
        fixture
            .server
            .plugin_manager
            .register::<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent, _>(
            Arc::new(SpawnReentry(calls.clone())),
            EventPriority::Normal,
            true,
        );
    }
    click(&fixture, mode);
    assert_eq!(villager.offers.lock().unwrap()[0].uses, 1);
    assert_eq!(
        screen
            .lock()
            .unwrap()
            .as_any()
            .downcast_ref::<MerchantScreenHandler>()
            .unwrap()
            .inventory
            .get_stack(0)
            .item_count,
        1
    );
    assert_eq!(
        fixture
            .world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_type == &EntityType::EXPERIENCE_ORB)
            .count(),
        1
    );
    assert_eq!(calls.load(Ordering::Relaxed), usize::from(reentry));
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_merchant_pickup_rewards_without_plugins() {
    reward_trade(SlotActionType::Pickup, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_merchant_quick_move_reward_callback_reenters_menu() {
    reward_trade(SlotActionType::QuickMove, true).await;
}

struct MenuPacketReentry(Arc<AtomicUsize>);
impl EventHandler<PacketSentEvent> for MenuPacketReentry {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        assert_menu_unlocked(&event.player);
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}
fn attach(fixture: &Fixture, screen: &SharedScreenHandler) {
    let sync = Arc::new(pumpkin_inventory::sync_handler::SyncHandler::new());
    sync.store_player(fixture.player.player.clone());
    let _effects = crate::world::guard_effects::GuardEffects::new();
    screen.lock().unwrap().update_sync_handler(sync);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_select_trade_defers_updates_and_sound() {
    let fixture = Fixture::new();
    let (villager, screen) = merchant(&fixture);
    villager.trade_sound_cooldown.store(0, Ordering::Relaxed);
    attach(&fixture, &screen);
    fixture.translating();
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(MenuPacketReentry(calls.clone())),
            EventPriority::Normal,
            true,
        );
    fixture.player.client().handle_select_trade(
        &fixture.player.player,
        &pumpkin_protocol::java::server::play::SSelectTrade {
            selected_slot: VarInt(0),
        },
    );
    assert!(calls.load(Ordering::Relaxed) > 0);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_beacon_payment_sync_reenters_menu() {
    let fixture = Fixture::new();
    let beacon = Arc::new(crate::block::entities::beacon::BeaconBlockEntity::new(
        pumpkin_util::math::position::BlockPos::new(0, 80, 0),
    ));
    beacon.levels.store(1, Ordering::Relaxed);
    let beacon_data: Arc<dyn pumpkin_inventory::window_property::PropertyDelegate> = beacon.clone();
    let handler = pumpkin_inventory::beacon_screen_handler::create_beacon_handler(
        1,
        &fixture.player.player.inventory,
        beacon_data,
    );
    handler
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::EMERALD));
    let screen: SharedScreenHandler = Arc::new(ScreenMutex::new(handler));
    *fixture.player.player.current_screen_handler.lock().unwrap() = screen.clone();
    attach(&fixture, &screen);
    fixture.translating();
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(MenuPacketReentry(calls.clone())),
            EventPriority::Normal,
            true,
        );
    fixture.player.client().handle_set_beacon(
        &fixture.player.player,
        &pumpkin_protocol::java::server::play::SSetBeacon {
            primary_effect: None,
            secondary_effect: None,
        },
    );
    assert!(
        screen
            .lock()
            .unwrap()
            .as_any()
            .downcast_ref::<BeaconScreenHandler>()
            .unwrap()
            .inventory
            .get_stack(0)
            .is_empty()
    );
    assert!(calls.load(Ordering::Relaxed) > 0);
    fixture.close().await;
}

struct ScorePacketReentry(Arc<AtomicUsize>);
impl EventHandler<PacketSentEvent> for ScorePacketReentry {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        drop(event.player.world().scoreboard.try_lock().unwrap());
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}
fn score_hook(fixture: &Fixture) -> Arc<AtomicUsize> {
    fixture.translating();
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(ScorePacketReentry(calls.clone())),
            EventPriority::Normal,
            true,
        );
    calls
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_execute_store_callback_reenters_scoreboard() {
    let fixture = Fixture::new();
    fixture.run("scoreboard objectives add stored dummy");
    let calls = score_hook(&fixture);
    fixture.run("execute store result score value stored run time query daytime");
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(
        fixture
            .world
            .scoreboard
            .lock()
            .unwrap()
            .get_score("value", "stored")
            .is_some()
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_mooshroom_team_transfer_reenters_scoreboard() {
    use crate::entity::shearable::shear_by_player;
    let fixture = Fixture::new();
    fixture.run("team add cows");
    let mushroom = crate::entity::passive::mooshroom::MooshroomEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(0.5, 80.0, 0.5),
        &EntityType::MOOSHROOM,
    ));
    fixture.world.scoreboard.lock().unwrap().add_player_to_team(
        &crate::world::scoreboard::NoTarget,
        "cows",
        mushroom.get_scoreboard_name(),
    );
    fixture.world.spawn_entity(mushroom.clone());
    let calls = score_hook(&fixture);
    let mut tool = ItemStack::new(1, &Item::SHEARS);
    assert!(shear_by_player(
        mushroom.as_ref(),
        &fixture.player.player,
        &mut tool
    ));
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(
        fixture
            .world
            .scoreboard
            .lock()
            .unwrap()
            .get_team("cows")
            .unwrap()
            .players
            .iter()
            .any(|name| name != &mushroom.get_scoreboard_name())
    );
    fixture.close().await;
}
