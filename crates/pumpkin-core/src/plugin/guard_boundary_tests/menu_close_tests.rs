use super::*;
use crate::plugin::api::events::player::inventory_close::InventoryCloseEvent;
use pumpkin_inventory::{
    generic_container_screen_handler::create_generic_9x3,
    player::player_inventory::PlayerInventory,
    screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerFactory},
};
use pumpkin_util::text::TextComponent;

struct Chest(Arc<SimpleInventory>);

impl ScreenHandlerFactory for Chest {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        inventory: &Arc<PlayerInventory>,
        player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        Some(Arc::new(ScreenMutex::new(create_generic_9x3(
            sync_id,
            inventory,
            self.0.clone(),
            player,
        ))))
    }
    fn get_display_name(&self) -> TextComponent {
        TextComponent::text("closing chest")
    }
}

struct InspectClosing {
    expected: SharedScreenHandler,
    calls: AtomicUsize,
}

impl EventHandler<InventoryCloseEvent> for InspectClosing {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut InventoryCloseEvent,
    ) -> BoxFuture<'a, ()> {
        assert_menu_unlocked(&event.player);
        let selected = event
            .player
            .current_screen_handler
            .try_lock()
            .unwrap()
            .clone();
        assert!(Arc::ptr_eq(&selected, &self.expected));
        let menu = selected.try_lock().unwrap();
        assert_eq!(menu.window_type(), event.window_type);
        assert_eq!(
            menu.get_behaviour().slots[0].get_stack().item,
            &Item::DIAMOND
        );
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_close_event_observes_closing_inventory() {
    let fixture = Fixture::new();
    let chest = Chest(Arc::new(SimpleInventory::new(27)));
    chest.0.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
    let player = &fixture.player.player;
    player.open_handled_screen(&chest, None).unwrap();
    let hook = Arc::new(InspectClosing {
        expected: player.current_screen_handler.lock().unwrap().clone(),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<InventoryCloseEvent, _>(hook.clone(), EventPriority::Normal, true);
    player.close_handled_screen();
    assert_eq!(hook.calls.load(Ordering::Relaxed), 1);
    let inventory: SharedScreenHandler = player.player_screen_handler.clone();
    assert!(Arc::ptr_eq(
        &*player.current_screen_handler.lock().unwrap(),
        &inventory
    ));
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_closing_menu_transfers_all_shared_player_slots() {
    let fixture = Fixture::new();
    let chest = Chest(Arc::new(SimpleInventory::new(27)));
    let player = &fixture.player.player;
    player.open_handled_screen(&chest, None).unwrap();
    let closing = player.current_screen_handler.lock().unwrap().clone();
    let inventory: Arc<dyn Inventory> = player.inventory.clone();
    let expected = [
        (0, &Item::BREAD, 2),
        (9, &Item::EMERALD, 3),
        (35, &Item::DIRT, 4),
    ];
    {
        let mut closing = closing.lock().unwrap();
        for (index, item, count) in expected {
            let behaviour = closing.get_behaviour_mut();
            let slot = behaviour
                .slots
                .iter()
                .position(|slot| {
                    slot.get_index() == index && Arc::ptr_eq(&slot.get_inventory(), &inventory)
                })
                .unwrap();
            behaviour.tracked_stacks[slot] = ItemStack::new(count, item);
            behaviour.previous_tracked_stacks[slot]
                .set_received_stack(ItemStack::new(count + 1, item));
        }
    }
    player.close_handled_screen();
    {
        let replacement = player.player_screen_handler.lock().unwrap();
        let behaviour = replacement.get_behaviour();
        for (index, item, count) in expected {
            let slot = behaviour
                .slots
                .iter()
                .position(|slot| {
                    slot.get_index() == index && Arc::ptr_eq(&slot.get_inventory(), &inventory)
                })
                .unwrap();
            assert!(behaviour.tracked_stacks[slot].are_equal(&ItemStack::new(count, item)));
            assert!(
                behaviour.previous_tracked_stacks[slot]
                    .received_stack
                    .as_ref()
                    .unwrap()
                    .are_equal(&ItemStack::new(count + 1, item))
            );
        }
    }
    fixture.close().await;
}
