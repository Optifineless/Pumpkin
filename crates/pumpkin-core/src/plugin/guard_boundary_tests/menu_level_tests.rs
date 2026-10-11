use super::{
    Arc, AtomicUsize, BoxFuture, EventHandler, EventPriority, Fixture, Item, ItemStack,
    OptionalItemStackHash, Ordering, PacketSentEvent, SClickSlot, SContainerButtonClick,
    ScreenMutex, Server, SharedScreenHandler, SlotActionType, VarInt,
};
use crate::plugin::api::events::player::player_level_change::PlayerLevelChangeEvent;
use pumpkin_inventory::{
    Inventory, SimpleInventory, anvil::anvil_screen_handler::AnvilScreenHandler,
};
use pumpkin_protocol::{ClientPacket, java::client::play::CLevelEvent};
use pumpkin_util::math::position::BlockPos;

struct CloseOnLevel(Arc<AtomicUsize>);
impl EventHandler<PlayerLevelChangeEvent> for CloseOnLevel {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerLevelChangeEvent,
    ) -> BoxFuture<'a, ()> {
        super::assert_menu_unlocked(&event.player);
        assert_eq!(event.old_level, 30);
        assert_eq!(event.new_level, 29);
        assert_eq!(event.player.experience_level.load(Ordering::Relaxed), 29);
        self.0.fetch_add(1, Ordering::Relaxed);
        event.player.close_handled_screen();
        Box::pin(async {})
    }
}

fn close_on_level(fixture: &Fixture) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerLevelChangeEvent, _>(
            Arc::new(CloseOnLevel(calls.clone())),
            EventPriority::Normal,
            true,
        );
    calls
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_successful_enchant_level_callback_can_close_menu() {
    let fixture = Fixture::new();
    let _menu = fixture.enchantment_menu();
    let calls = close_on_level(&fixture);
    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(1),
            button_id: VarInt(0),
        });
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let inventory = &fixture.player.player.inventory;
    assert!((0..36).any(|index| {
        let item = inventory.get_stack(index);
        item.item == &Item::DIAMOND_SWORD && item.has_enchantments()
    }));
    assert_eq!(
        (0..36)
            .map(|index| inventory.get_stack(index))
            .filter(|item| item.item == &Item::LAPIS_LAZULI)
            .map(|item| u32::from(item.item_count))
            .sum::<u32>(),
        2
    );
    fixture.close().await;
}

struct ObserveAnvil {
    expected_payload: Vec<u8>,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<PacketSentEvent> for ObserveAnvil {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == pumpkin_data::packet::clientbound::play::LEVEL_EVENT.0 {
            super::assert_menu_unlocked(&event.player);
            assert_eq!(&event.payload[..], &self.expected_payload);
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_anvil_effect_keeps_menu_position_after_level_callback() {
    let fixture = Fixture::new();
    let player = &fixture.player.player;
    player.experience_level.store(30, Ordering::Relaxed);
    let inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(3));
    inventory.set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    inventory.set_stack(2, ItemStack::new(1, &Item::DIAMOND_SWORD));
    let menu = AnvilScreenHandler::new(1, &player.inventory, inventory);
    menu.repair_cost.store(1, Ordering::Relaxed);
    menu.only_renaming.store(true, Ordering::Relaxed);
    let menu: SharedScreenHandler = Arc::new(ScreenMutex::new(menu));
    *player.current_screen_handler.lock().unwrap() = menu;
    let pos = BlockPos::new(0, 64, 0);
    player.open_container_pos.store(Some(pos));
    let level_calls = close_on_level(&fixture);
    fixture.translating();
    let sounds = Arc::new(AtomicUsize::new(0));
    let mut expected_payload = Vec::new();
    CLevelEvent::new(
        pumpkin_data::world::WorldEvent::SoundAnvilUsed as i32,
        pos,
        0,
        false,
    )
    .write_packet_data(
        &mut expected_payload,
        &pumpkin_data::packet::CURRENT_MC_VERSION,
    )
    .unwrap();
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(ObserveAnvil {
                expected_payload,
                calls: sounds.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(1),
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
    assert_eq!(level_calls.load(Ordering::Relaxed), 1);
    assert!(player.open_container_pos.load().is_none());
    assert_eq!(sounds.load(Ordering::Relaxed), 1);
    fixture.close().await;
}
