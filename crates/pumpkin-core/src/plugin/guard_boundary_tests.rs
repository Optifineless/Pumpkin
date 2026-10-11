//! Each scenario runs a real entry point once (or a bounded number of ticks).
//! Callbacks use `try_lock` so release regressions fail without hanging a test worker.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use pumpkin_data::{
    item::Item,
    item_stack::ItemStack,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_inventory::{
    Inventory, SimpleInventory,
    enchanting::enchanting_screen_handler::EnchantingTableScreenHandler,
    screen_handler::{ScreenMutex, SharedScreenHandler},
};
use pumpkin_protocol::{
    codec::{item_stack_seralizer::OptionalItemStackHash, var_int::VarInt},
    java::server::play::{SClickSlot, SContainerButtonClick, SlotActionType},
};
use pumpkin_util::{PermissionLvl, version::JavaMinecraftVersion};

use super::{
    BoxFuture, EventHandler, EventPriority,
    api::events::{
        enchantment::{
            enchant_item::EnchantItemEvent, prepare_item_enchant::PrepareItemEnchantEvent,
        },
        player::player_statistic_increment::PlayerStatisticIncrementEvent,
        server::packet::PacketSentEvent,
    },
};
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::World,
};

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let player = TestPlayer::new(&world);
        player.player.permission_lvl.store(PermissionLvl::Four);
        player.client().set_player(player.player.clone());
        combat_test_support::publish_empty_chunk(
            &world,
            pumpkin_util::math::vector2::Vector2::new(0, 0),
        );
        Self {
            _directory: directory,
            server,
            world,
            player,
        }
    }

    fn run(&self, input: &str) {
        self.server
            .command_dispatcher
            .load()
            .execute_input(input, &self.player.player.get_command_source(&self.server))
            .unwrap();
    }

    fn translating(&self) {
        self.player
            .client()
            .version
            .store(JavaMinecraftVersion::V_1_21_2);
    }

    fn enchantment_menu(&self) -> SharedScreenHandler {
        let player = &self.player.player;
        player.experience_level.store(30, Ordering::Relaxed);
        let inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(2));
        inventory.set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
        inventory.set_stack(1, ItemStack::new(3, &Item::LAPIS_LAZULI));
        let mut menu = EnchantingTableScreenHandler::new(
            1,
            &player.inventory,
            &inventory,
            player.enchantment_seed.load(Ordering::Relaxed),
            15,
        );
        menu.update_enchantments(player.as_ref());
        let menu: SharedScreenHandler = Arc::new(ScreenMutex::new(menu));
        *player.current_screen_handler.lock().unwrap() = menu.clone();
        menu
    }

    async fn close(self) {
        self.player.client().close();
        self.world.level.shutdown().await.unwrap();
    }
}

fn assert_menu_unlocked(player: &crate::entity::player::Player) {
    let current = player.current_screen_handler.try_lock().unwrap().clone();
    drop(current.try_lock().unwrap());
}

struct CloseEnchant(Arc<AtomicUsize>);
impl EventHandler<EnchantItemEvent> for CloseEnchant {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EnchantItemEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        assert_menu_unlocked(&event.player);
        event.player.close_handled_screen();
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_enchant_button_reentry_closes_without_spending_resources() {
    let fixture = Fixture::new();
    let menu = fixture.enchantment_menu();
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<EnchantItemEvent, _>(
            Arc::new(CloseEnchant(calls.clone())),
            EventPriority::Normal,
            true,
        );
    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(1),
            button_id: VarInt(0),
        });
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        fixture
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        30
    );
    assert!(
        menu.lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .try_lock()
            .unwrap()
            .is_empty()
    );
    assert!(
        !fixture
            .player
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .lock()
            .unwrap()
            .as_any()
            .is::<EnchantingTableScreenHandler>()
    );
    let items: Vec<_> = (0..36)
        .map(|index| fixture.player.player.inventory.get_stack(index))
        .collect();
    assert!(
        items
            .iter()
            .any(|item| item.item == &Item::DIAMOND_SWORD && !item.has_enchantments())
    );
    assert_eq!(
        items
            .iter()
            .filter(|item| item.item == &Item::LAPIS_LAZULI)
            .map(|item| u32::from(item.item_count))
            .sum::<u32>(),
        3
    );
    fixture.close().await;
}

struct ClosePrepare(Arc<AtomicUsize>);

struct SpendEnchantLevels;
impl EventHandler<EnchantItemEvent> for SpendEnchantLevels {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EnchantItemEvent,
    ) -> BoxFuture<'a, ()> {
        event.player.add_experience_levels(-30);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_enchant_revalidates_experience_after_callback() {
    let fixture = Fixture::new();
    let menu = fixture.enchantment_menu();
    let seed = fixture
        .player
        .player
        .enchantment_seed
        .load(Ordering::Relaxed);
    fixture
        .server
        .plugin_manager
        .register::<EnchantItemEvent, _>(Arc::new(SpendEnchantLevels), EventPriority::Normal, true);
    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(1),
            button_id: VarInt(0),
        });
    let guard = menu.lock().unwrap();
    let table = guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap();
    assert!(!table.inventory.get_stack(0).has_enchantments());
    assert_eq!(table.inventory.get_stack(1).item_count, 3);
    drop(guard);
    assert_eq!(
        fixture
            .player
            .player
            .enchantment_seed
            .load(Ordering::Relaxed),
        seed
    );
    fixture.close().await;
}

impl EventHandler<PrepareItemEnchantEvent> for ClosePrepare {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PrepareItemEnchantEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        assert_menu_unlocked(&event.player);
        event.player.close_handled_screen();
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_preparation_callback_can_close_menu_during_container_click() {
    let fixture = Fixture::new();
    let menu = fixture.enchantment_menu();
    let guard = menu.lock().unwrap();
    guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap()
        .inventory
        .set_stack(0, ItemStack::EMPTY.clone());
    *guard.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::new(1, &Item::DIAMOND_SWORD);
    drop(guard);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PrepareItemEnchantEvent, _>(
            Arc::new(ClosePrepare(calls.clone())),
            EventPriority::Normal,
            true,
        );
    fixture.player.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(1),
            revision: VarInt(0),
            slot: 0,
            button: 0,
            mode: SlotActionType::Pickup,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &fixture.server,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    fixture.close().await;
}

struct CloseDrop(Arc<AtomicUsize>);
impl EventHandler<PlayerStatisticIncrementEvent> for CloseDrop {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        let current = event
            .player
            .current_screen_handler
            .try_lock()
            .unwrap()
            .clone();
        let menu = current.try_lock().unwrap();
        assert!(
            menu.get_behaviour()
                .cursor_stack
                .try_lock()
                .unwrap()
                .is_empty()
        );
        drop(menu);
        self.0.fetch_add(1, Ordering::Relaxed);
        assert_menu_unlocked(&event.player);
        event.player.close_handled_screen();
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_cursor_drop_statistic_callback_can_close_menu() {
    let fixture = Fixture::new();
    let menu = fixture.enchantment_menu();
    *menu
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap() = ItemStack::new(2, &Item::DIRT);
    let calls = Arc::new(AtomicUsize::new(0));
    let preparations = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PrepareItemEnchantEvent, _>(
            Arc::new(ClosePrepare(preparations.clone())),
            EventPriority::Normal,
            true,
        );
    fixture
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(CloseDrop(calls.clone())),
            EventPriority::Normal,
            true,
        );
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
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert_eq!(preparations.load(Ordering::Relaxed), 0);
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

#[derive(Clone, Copy)]
enum Owner {
    Weather,
    Scoreboard,
    Border,
    Advancements,
}
struct ReenterPacket {
    owner: Owner,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<PacketSentEvent> for ReenterPacket {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if matches!(self.owner, Owner::Advancements)
            && event.packet_id != pumpkin_data::packet::clientbound::play::UPDATE_ADVANCEMENTS.0
        {
            return Box::pin(async {});
        }
        let world = event.player.world();
        let first = self.calls.fetch_add(1, Ordering::Relaxed) == 0;
        match self.owner {
            Owner::Weather => {
                let weather = world.weather.try_lock().unwrap();
                assert!(weather.rain_level > 0.0);
            }
            Owner::Scoreboard => {
                assert_eq!(
                    world
                        .scoreboard
                        .try_lock()
                        .unwrap()
                        .get_score_value(&event.player.gameprofile.name, "dropped"),
                    Some(1)
                );
            }
            Owner::Border => {
                assert_eq!(world.worldborder.try_lock().unwrap().new_diameter, 64.0);
            }
            Owner::Advancements => {
                assert!(
                    !event
                        .player
                        .advancements
                        .try_lock()
                        .unwrap()
                        .is_first_packet
                );
                if first {
                    server
                        .command_dispatcher
                        .load()
                        .execute_input(
                            "advancement revoke @s only minecraft:story/root",
                            &event.player.get_command_source(server),
                        )
                        .unwrap();
                }
            }
        }
        Box::pin(async {})
    }
}

fn packet_hook(fixture: &Fixture, owner: Owner) -> Arc<AtomicUsize> {
    fixture.translating();
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(ReenterPacket {
                owner,
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    calls
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_weather_world_tick_allows_packet_hook_to_read_weather() {
    let fixture = Fixture::new();
    fixture
        .world
        .weather
        .lock()
        .unwrap()
        .set_weather_parameters(&fixture.world, 0, 100, true, false);
    let calls = packet_hook(&fixture, Owner::Weather);
    fixture.world.tick_environment();
    assert!(calls.load(Ordering::Relaxed) > 0);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_statistic_award_allows_packet_hook_to_read_scoreboard() {
    let fixture = Fixture::new();
    fixture.world.scoreboard.lock().unwrap().add_objective(
        &crate::world::scoreboard::NoTarget,
        crate::world::scoreboard::ScoreboardObjective::new(
            "dropped",
            pumpkin_util::text::TextComponent::text("dropped"),
            pumpkin_protocol::java::client::play::RenderType::Integer,
            None,
            "minecraft.custom:minecraft.drop",
        ),
    );
    let calls = packet_hook(&fixture, Owner::Scoreboard);
    fixture
        .player
        .player
        .award_stat(StatisticCategory::Custom, CustomStatistic::Drop as i32, 1);
    assert!(calls.load(Ordering::Relaxed) > 0);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_worldborder_dispatch_allows_packet_hook_to_read_border() {
    let fixture = Fixture::new();
    let calls = packet_hook(&fixture, Owner::Border);
    fixture.run("worldborder set 64");
    fixture.run("worldborder center 2 3");
    fixture.run("worldborder warning distance 7");
    assert!(calls.load(Ordering::Relaxed) >= 3);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_advancement_player_tick_preserves_reentrant_revoke_for_next_flush() {
    let fixture = Fixture::new();
    fixture.run("advancement grant @s only minecraft:story/root");
    let calls = packet_hook(&fixture, Owner::Advancements);
    fixture.player.player.tick(&fixture.server);
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(
        !fixture
            .player
            .player
            .advancements
            .lock()
            .unwrap()
            .progress_changed
            .is_empty()
    );
    fixture.player.player.tick(&fixture.server);
    assert!(
        fixture
            .player
            .player
            .advancements
            .lock()
            .unwrap()
            .progress_changed
            .is_empty()
    );
    fixture.close().await;
}

mod bedrock_packets;
mod menu_close_tests;
mod menu_level_tests;
mod menu_revalidation_tests;
mod menu_session_tests;
mod merchant_packets;
mod merchant_reopen;
mod ordered_delivery;
mod saved_login_tests;
mod unbatched_owners;
