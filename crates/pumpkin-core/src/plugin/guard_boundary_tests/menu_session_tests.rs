use super::{
    Arc, AtomicUsize, BoxFuture, CustomStatistic, EventHandler, EventPriority, Fixture, Item,
    ItemStack, OptionalItemStackHash, Ordering, PlayerStatisticIncrementEvent, SClickSlot, Server,
    SlotActionType, StatisticCategory, VarInt,
};
use crate::plugin::api::events::enchantment::prepare_item_enchant::PrepareItemEnchantEvent;
use crate::{entity::player::Player, net::PlayerConfig};

struct ReplaceSession {
    original: Arc<Player>,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<PlayerStatisticIncrementEvent> for ReplaceSession {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        assert!(Arc::ptr_eq(&self.original, &event.player));
        super::assert_menu_unlocked(&event.player);
        self.calls.fetch_add(1, Ordering::Relaxed);
        let world = event.player.world();
        let replacement = Arc::new(Player::new(
            event.player.client.clone(),
            event.player.gameprofile.clone(),
            PlayerConfig::default(),
            &world,
            pumpkin_util::GameMode::Survival,
        ));
        world.players.store(Arc::new(vec![replacement]));
        Box::pin(async {})
    }
}

impl EventHandler<PrepareItemEnchantEvent> for ReplaceSession {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut PrepareItemEnchantEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_menu_drop_does_not_notify_replacement_session() {
    let fixture = Fixture::new();
    let menu = fixture.enchantment_menu();
    *menu
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap() = ItemStack::new(1, &Item::DIRT);
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(ReplaceSession {
        original: fixture.player.player.clone(),
        calls: calls.clone(),
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(handler.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<PrepareItemEnchantEvent, _>(handler, EventPriority::Normal, true);
    fixture.player.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(1),
            revision: VarInt(0),
            slot: -999,
            button: 0,
            mode: SlotActionType::Pickup,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &fixture.server,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        fixture.player.player.get_custom_stat(CustomStatistic::Drop),
        1
    );
    assert_eq!(
        fixture
            .world
            .get_player_by_uuid(fixture.player.player.gameprofile.id)
            .unwrap()
            .get_stat(StatisticCategory::Dropped, i32::from(Item::DIRT.id)),
        0
    );
    assert!(
        menu.lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .is_empty()
    );
    fixture.close().await;
}
