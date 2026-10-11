use super::{
    Arc, BoxFuture, EventHandler, EventPriority, Fixture, Item, ItemStack, OptionalItemStackHash,
    SClickSlot, Server, SlotActionType, VarInt,
};
use crate::plugin::api::events::player::inventory_interact::InventoryClickEvent;
use pumpkin_inventory::screen_handler::ScreenHandler;

struct ReplaceMenu;
impl EventHandler<InventoryClickEvent> for ReplaceMenu {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut InventoryClickEvent,
    ) -> BoxFuture<'a, ()> {
        super::assert_menu_unlocked(&event.player);
        event.player.close_handled_screen();
        event
            .player
            .player_screen_handler
            .try_lock()
            .unwrap()
            .get_behaviour()
            .slots[1]
            .set_stack(ItemStack::new(1, &Item::DIRT));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_container_click_revalidates_menu_after_callback() {
    let fixture = Fixture::new();
    let _menu = fixture.enchantment_menu();
    fixture
        .server
        .plugin_manager
        .register::<InventoryClickEvent, _>(Arc::new(ReplaceMenu), EventPriority::Normal, true);
    fixture.player.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(1),
            revision: VarInt(0),
            slot: 1,
            button: 0,
            mode: SlotActionType::Pickup,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &fixture.server,
    );
    let menu = fixture
        .player
        .player
        .player_screen_handler
        .try_lock()
        .unwrap();
    assert_eq!(menu.get_behaviour().slots[1].get_stack().item, &Item::DIRT);
    assert!(
        menu.get_behaviour()
            .cursor_stack
            .try_lock()
            .unwrap()
            .is_empty()
    );
    drop(menu);
    fixture.close().await;
}
