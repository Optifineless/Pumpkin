use super::*;
use crate::entity::{
    Entity, EntityBase,
    passive::{villager::VillagerEntity, wandering_trader::WanderingTraderEntity},
};
use pumpkin_data::entity::EntityType;
use pumpkin_inventory::{
    merchant::merchant_screen_handler::MerchantScreenHandler, screen_handler::ScreenHandlerFactory,
};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer, java::client::play::MerchantOffer,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::{borrow::Cow, sync::Mutex};

enum Merchant {
    Villager(Arc<VillagerEntity>),
    Wanderer(Arc<WanderingTraderEntity>),
}

impl Merchant {
    fn new(fixture: &Fixture, wandering: bool) -> Self {
        let player = &fixture.player.player;
        player.get_entity().set_pos(Vector3::new(0.5, 80.0, 0.5));
        let merchant = if wandering {
            Self::Wanderer(WanderingTraderEntity::new(Entity::new(
                fixture.world.clone(),
                player.position(),
                &EntityType::WANDERING_TRADER,
            )))
        } else {
            let villager = VillagerEntity::new(Entity::new(
                fixture.world.clone(),
                player.position(),
                &EntityType::VILLAGER,
            ));
            villager.gossips.lock().unwrap().insert(
                player.gameprofile.id,
                std::iter::once((crate::entity::passive::villager::GossipType::Trading, 10))
                    .collect(),
            );
            Self::Villager(villager)
        };
        merchant.offers().lock().unwrap().push(MerchantOffer {
            base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(2, &Item::EMERALD))),
            output: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::BREAD))),
            cost_b: None,
            reward_exp: true,
            uses: 0,
            max_uses: 4,
            xp: 0,
            special_price: 0,
            price_multiplier: 0.1,
            demand: 0,
        });
        player
            .inventory
            .set_stack(0, ItemStack::new(4, &Item::EMERALD));
        merchant
    }

    fn factory(&self) -> &dyn ScreenHandlerFactory {
        match self {
            Self::Villager(merchant) => merchant.as_ref(),
            Self::Wanderer(merchant) => merchant.as_ref(),
        }
    }

    fn offers(&self) -> &Mutex<Vec<MerchantOffer>> {
        match self {
            Self::Villager(merchant) => &merchant.offers,
            Self::Wanderer(merchant) => &merchant.offers,
        }
    }

    fn assert_session(&self, player: &crate::entity::player::Player, sync_id: Option<u8>) {
        let (owner, trading) = match self {
            Self::Villager(merchant) => (&merchant.trading_player, &merchant.is_trading),
            Self::Wanderer(merchant) => (&merchant.trading_player, &merchant.is_trading),
        };
        assert_eq!(
            *owner.lock().unwrap(),
            sync_id.map(|id| (player.gameprofile.id, id))
        );
        assert_eq!(trading.load(Ordering::Relaxed), sync_id.is_some());
        let special_price = if sync_id.is_some() && matches!(self, Self::Villager(_)) {
            -1
        } else {
            0
        };
        assert_eq!(
            self.offers().lock().unwrap()[0].special_price,
            special_price
        );
        if let Some(sync_id) = sync_id {
            let screen = player.current_screen_handler.lock().unwrap().clone();
            let screen = screen.lock().unwrap();
            assert_eq!(screen.sync_id(), sync_id);
            assert!(screen.can_use(player));
            let menu = screen
                .as_any()
                .downcast_ref::<MerchantScreenHandler>()
                .unwrap();
            assert_eq!(menu.offers[0].special_price, special_price);
        }
    }
}

struct ReopenTrade {
    merchant: Arc<Merchant>,
    calls: AtomicUsize,
}

impl EventHandler<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent>
    for ReopenTrade
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(self.merchant.offers().lock().unwrap()[0].uses, 1);
        let players = event.world.players.load();
        let player = &players[0];
        assert_menu_unlocked(player);
        player
            .open_handled_screen(self.merchant.factory(), None)
            .unwrap();
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

async fn reopen(wandering: bool, trade_callback: bool, wrap: bool) {
    let fixture = Fixture::new();
    let merchant = Arc::new(Merchant::new(&fixture, wandering));
    let player = &fixture.player.player;
    let old_id = player
        .open_handled_screen(merchant.factory(), None)
        .unwrap();
    let old_menu = player.current_screen_handler.lock().unwrap().clone();
    merchant.assert_session(player, Some(old_id));
    if trade_callback {
        let hook = Arc::new(ReopenTrade {
            merchant: merchant.clone(),
            calls: AtomicUsize::new(0),
        });
        fixture
            .server
            .plugin_manager
            .register::<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        fixture.player.client().handle_select_trade(
            player,
            &pumpkin_protocol::java::server::play::SSelectTrade {
                selected_slot: VarInt(0),
            },
        );
        player.on_slot_click(
            SClickSlot {
                sync_id: VarInt(i32::from(old_id)),
                revision: VarInt(0),
                slot: 2,
                button: 0,
                mode: SlotActionType::Pickup,
                length_of_array: VarInt(0),
                array_of_changed_slots: Vec::new(),
                carried_item: OptionalItemStackHash(None),
            },
            &fixture.server,
        );
        assert_eq!(hook.calls.load(Ordering::Relaxed), 1);
    } else if wrap {
        // Model several menu opens inside one callback batch, including a reused window ID.
        let _effects = crate::world::guard_effects::GuardEffects::new();
        for _ in 0..100 {
            player
                .open_handled_screen(merchant.factory(), None)
                .unwrap();
        }
        assert_eq!(
            player.screen_handler_sync_id.load(Ordering::Relaxed),
            old_id
        );
    } else {
        player
            .open_handled_screen(merchant.factory(), None)
            .unwrap();
    }
    let replacement = player.current_screen_handler.lock().unwrap().clone();
    assert!(!Arc::ptr_eq(&old_menu, &replacement));
    let new_id = replacement.lock().unwrap().sync_id();
    merchant.assert_session(player, Some(new_id));
    player.close_handled_screen();
    merchant.assert_session(player, None);
    let emeralds = (0..36)
        .map(|index| player.inventory.get_stack(index))
        .filter(|stack| stack.item == &Item::EMERALD)
        .map(|stack| u32::from(stack.item_count))
        .sum::<u32>();
    let spent = if trade_callback {
        if wandering { 2 } else { 1 }
    } else {
        0
    };
    assert_eq!(emeralds, 4 - spent);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_villager_reopen_retains_replacement_session() {
    reopen(false, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_wanderer_reopen_retains_replacement_session() {
    reopen(true, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_villager_trade_callback_reopens_same_merchant() {
    reopen(false, true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_wanderer_trade_callback_reopens_same_merchant() {
    reopen(true, true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_villager_reopen_survives_window_id_wrap() {
    reopen(false, false, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_wanderer_reopen_survives_window_id_wrap() {
    reopen(true, false, true).await;
}

struct ReopenClosingTrade {
    merchant: Arc<Merchant>,
    closing: SharedScreenHandler,
    replacement: Mutex<Option<(u8, SharedScreenHandler)>>,
    position: BlockPos,
    calls: AtomicUsize,
}

impl EventHandler<crate::plugin::api::events::player::inventory_close::InventoryCloseEvent>
    for ReopenClosingTrade
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::inventory_close::InventoryCloseEvent,
    ) -> BoxFuture<'a, ()> {
        if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            assert_menu_unlocked(&event.player);
            let selected = event.player.current_screen_handler.lock().unwrap().clone();
            assert!(Arc::ptr_eq(&selected, &self.closing));
            let id = event
                .player
                .open_handled_screen(self.merchant.factory(), Some(self.position))
                .unwrap();
            let selected = event.player.current_screen_handler.lock().unwrap().clone();
            *self.replacement.lock().unwrap() = Some((id, selected));
        }
        Box::pin(async {})
    }
}

async fn close_event_reopen(wandering: bool) {
    let fixture = Fixture::new();
    let merchant = Arc::new(Merchant::new(&fixture, wandering));
    let player = &fixture.player.player;
    let old_id = player
        .open_handled_screen(merchant.factory(), None)
        .unwrap();
    merchant.assert_session(player, Some(old_id));
    let closing = player.current_screen_handler.lock().unwrap().clone();
    let hook = Arc::new(ReopenClosingTrade {
        merchant: merchant.clone(),
        closing: closing.clone(),
        replacement: Mutex::new(None),
        position: BlockPos::new(2, 80, 2),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::player::inventory_close::InventoryCloseEvent, _>(
        hook.clone(),
        EventPriority::Normal,
        true,
    );
    player.close_handled_screen();
    assert_eq!(
        hook.calls.load(Ordering::Relaxed),
        1,
        "closed the same menu recursively"
    );
    let (id, expected) = hook.replacement.lock().unwrap().clone().unwrap();
    let selected = player.current_screen_handler.lock().unwrap().clone();
    assert!(!Arc::ptr_eq(&selected, &closing));
    assert!(
        Arc::ptr_eq(&selected, &expected),
        "outer close discarded replacement"
    );
    assert_eq!(player.open_container_pos.load(), Some(hook.position));
    merchant.assert_session(player, Some(id));
    player.close_handled_screen();
    assert_eq!(hook.calls.load(Ordering::Relaxed), 2);
    merchant.assert_session(player, None);
    let inventory: SharedScreenHandler = player.player_screen_handler.clone();
    assert!(Arc::ptr_eq(
        &*player.current_screen_handler.lock().unwrap(),
        &inventory
    ));
    assert_eq!(player.open_container_pos.load(), None);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_villager_close_event_reopen_retains_replacement() {
    close_event_reopen(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_wanderer_close_event_reopen_retains_replacement() {
    close_event_reopen(true).await;
}
