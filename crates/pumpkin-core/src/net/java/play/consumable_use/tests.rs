use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::{
            player_interact_event::PlayerInteractEvent, player_item_consume::PlayerItemConsumeEvent,
        },
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, biome::Biome, data_component_impl::ConsumableImpl, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::server::play::{SUseItem, SUseItemOn},
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use std::sync::{Arc, Mutex, atomic::Ordering::Relaxed};

const CLICKED: BlockPos = BlockPos::new(8, 64, 8);
const ABOVE: BlockPos = BlockPos::new(8, 65, 8);

struct Scenario {
    server: Arc<Server>,
    world: Arc<World>,
    fixture: TestPlayer,
    hand: Hand,
    source: usize,
    original: ItemStack,
    _dir: tempfile::TempDir,
}

impl Scenario {
    fn new(item: &'static Item, hand: Hand, target: &'static Block) -> Self {
        Self::with_registry(item, hand, target, |_| {})
    }

    fn with_registry(
        item: &'static Item,
        hand: Hand,
        target: &'static Block,
        configure: impl FnOnce(&mut Server),
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(dir.path());
        configure(Arc::get_mut(&mut server).unwrap());
        let world = combat_test_support::world(&server, dir.path());
        let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        chunk.set_block_state(8, 64, 8, target.default_state);
        spawn_test_support::publish(&world, chunk);
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        player.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
        player.advancements.try_lock().unwrap().player = Arc::downgrade(player);
        player
            .permission_lvl
            .store(pumpkin_util::permission::PermissionLvl::Four);
        player.hunger_manager.level.store(10);
        let source = if hand == Hand::Right { 0 } else { 40 };
        let original = ItemStack::new(2, item);
        player.inventory().set_stack(source, original.clone());
        Self {
            server,
            world,
            fixture,
            hand,
            source,
            original,
            _dir: dir,
        }
    }

    fn use_block(&self) {
        let accepted = self.fixture.client().handle_use_item_on(
            &self.fixture.player,
            &SUseItemOn {
                hand: VarInt(i32::from(self.hand == Hand::Left)),
                position: CLICKED,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(17),
            },
            &self.server,
        );
        assert!(accepted.is_ok(), "use-item-on rejected: {accepted:?}");
        assert_eq!(self.fixture.client().packet_sequence.load(Relaxed), 17);
    }

    fn use_air(&self) {
        self.fixture.client().handle_use_item(
            &self.fixture.player,
            &SUseItem {
                hand: VarInt(i32::from(self.hand == Hand::Left)),
                sequence: VarInt(18),
                yaw: 0.0,
                pitch: -90.0,
            },
            &self.server,
        );
        assert_eq!(self.fixture.client().packet_sequence.load(Relaxed), 18);
    }

    fn active(&self) -> Option<Hand> {
        *self
            .fixture
            .player
            .living_entity
            .active_hand
            .lock()
            .unwrap()
    }

    fn tick(&self, count: usize) {
        assert!(count <= 80, "item-use ticks must stay bounded");
        for _ in 0..count {
            self.fixture
                .player
                .living_entity
                .tick(self.fixture.player.as_ref(), &self.server);
        }
    }

    fn assert_stock_and_food(&self, count: u8, food: u8) {
        let stack = self.fixture.player.inventory().get_stack(self.source);
        assert_eq!(stack.item_count, count, "source item count");
        assert_eq!(stack.uid, self.original.uid, "source identity changed");
        assert_eq!(
            self.fixture.player.hunger_manager.level.load(),
            food,
            "hunger level"
        );
    }

    async fn finish(self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.world.level.shutdown(),
        )
        .await
        .unwrap()
        .unwrap();
    }
}

async fn stone_eating(item: &'static Item, hand: Hand, nutrition: u8) {
    let scenario = Scenario::new(item, hand, &Block::STONE);
    let events = register_consume_callback(&scenario, Edit::None);
    scenario.use_block();
    assert_eq!(
        scenario.active(),
        Some(hand),
        "refused crop placement must start eating"
    );
    scenario.assert_stock_and_food(2, 10);
    scenario.tick(5);
    let remaining = scenario
        .fixture
        .player
        .living_entity
        .item_use_time
        .load(Relaxed);
    scenario.use_air();
    assert_eq!(
        scenario
            .fixture
            .player
            .living_entity
            .item_use_time
            .load(Relaxed),
        remaining,
        "later use-item packet restarted eating"
    );
    scenario.tick(27);
    assert_eq!(scenario.active(), None);
    scenario.assert_stock_and_food(1, 10 + nutrition);
    scenario.tick(32);
    scenario.assert_stock_and_food(1, 10 + nutrition);
    assert_eq!(scenario.world.get_block(&CLICKED), &Block::STONE);
    assert_eq!(scenario.world.get_block(&ABOVE), &Block::AIR);
    assert_eq!(
        &*events.lock().unwrap(),
        &["interact", "consume", "consume", "interact"]
    );
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_carrot_stone_main_hand_eats_once() {
    stone_eating(&Item::CARROT, Hand::Right, 3).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_carrot_stone_offhand_eats_once() {
    stone_eating(&Item::CARROT, Hand::Left, 3).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_potato_stone_main_hand_eats_once() {
    stone_eating(&Item::POTATO, Hand::Right, 1).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_potato_stone_offhand_eats_once() {
    stone_eating(&Item::POTATO, Hand::Left, 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_air_eating_control() {
    for (item, nutrition) in [(&Item::CARROT, 3), (&Item::POTATO, 1)] {
        for hand in [Hand::Right, Hand::Left] {
            let scenario = Scenario::new(item, hand, &Block::STONE);
            scenario.use_air();
            assert_eq!(scenario.active(), Some(hand));
            scenario.tick(32);
            scenario.assert_stock_and_food(1, 10 + nutrition);
            scenario.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_farmland_plants_without_eating() {
    for (item, crop) in [
        (&Item::CARROT, &Block::CARROTS),
        (&Item::POTATO, &Block::POTATOES),
    ] {
        for hand in [Hand::Right, Hand::Left] {
            let scenario = Scenario::new(item, hand, &Block::FARMLAND);
            let events = register_consume_callback(&scenario, Edit::None);
            scenario.use_block();
            assert_eq!(scenario.world.get_block(&ABOVE), crop);
            assert_eq!(scenario.active(), None);
            scenario.tick(64);
            scenario.assert_stock_and_food(1, 10);
            assert_eq!(&*events.lock().unwrap(), &["interact"]);
            scenario.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_full_hunger_and_cooldown_refuse_eating() {
    for item in [&Item::CARROT, &Item::POTATO] {
        for hand in [Hand::Right, Hand::Left] {
            let scenario = Scenario::new(item, hand, &Block::STONE);
            scenario.fixture.player.hunger_manager.level.store(20);
            scenario.use_block();
            assert_eq!(scenario.active(), None);
            scenario.tick(64);
            scenario.assert_stock_and_food(2, 20);
            scenario.fixture.player.hunger_manager.level.store(10);
            scenario
                .fixture
                .player
                .start_cooldown(item.registry_key.to_owned(), 100);
            scenario.use_block();
            assert_eq!(scenario.active(), None);
            scenario.tick(64);
            scenario.assert_stock_and_food(2, 10);
            scenario.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_consuming_composter_interaction_takes_precedence() {
    use pumpkin_data::block_properties::ComposterLikeProperties;
    for hand in [Hand::Right, Hand::Left] {
        let scenario = Scenario::new(&Item::CARROT, hand, &Block::COMPOSTER);
        let events = register_consume_callback(&scenario, Edit::None);
        scenario.use_block();
        assert_eq!(
            ComposterLikeProperties::from_state_id(scenario.world.get_block_state_id(&CLICKED))
                .level,
            1
        );
        assert_eq!(scenario.active(), None);
        scenario.tick(64);
        scenario.assert_stock_and_food(1, 10);
        assert_eq!(&*events.lock().unwrap(), &["interact"]);
        scenario.finish().await;
    }
}

#[derive(Clone, Copy)]
enum Edit {
    None,
    Replace,
    Select,
    Cancel,
}
struct ConsumeCallback {
    edit: Edit,
    source: usize,
    item_name: &'static str,
    events: Arc<Mutex<Vec<&'static str>>>,
}
impl EventHandler<PlayerInteractEvent> for ConsumeCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push("interact");
        Box::pin(async {})
    }
}
impl EventHandler<PlayerItemConsumeEvent> for ConsumeCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerItemConsumeEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push("consume");
        assert_eq!(event.item_name, self.item_name);
        let inventory = event.player.inventory();
        let owner = inventory.slot_transaction(self.source);
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    assert!(
                        owner.try_lock().is_some(),
                        "inventory owner held across consume callback"
                    );
                })
                .join()
                .unwrap();
        });
        match self.edit {
            Edit::None => {}
            Edit::Replace => {
                let observed = inventory.get_stack(self.source);
                inventory.set_stack(
                    self.source,
                    ItemStack::new(observed.item_count, observed.item),
                );
            }
            Edit::Select => {
                inventory.set_stack(1, ItemStack::new(2, &Item::POTATO));
                inventory.set_selected_slot(1);
            }
            Edit::Cancel => event.cancelled = true,
        }
        Box::pin(async {})
    }
}

fn register_consume_callback(scenario: &Scenario, edit: Edit) -> Arc<Mutex<Vec<&'static str>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let callback = Arc::new(ConsumeCallback {
        edit,
        source: scenario.source,
        item_name: scenario.original.item.registry_key,
        events: events.clone(),
    });
    scenario
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(callback.clone(), EventPriority::Normal, true);
    scenario
        .server
        .plugin_manager
        .register::<PlayerItemConsumeEvent, _>(callback, EventPriority::Normal, true);
    events
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_consume_callback_replacement_is_preserved() {
    for hand in [Hand::Right, Hand::Left] {
        let scenario = Scenario::new(&Item::CARROT, hand, &Block::STONE);
        let events = register_consume_callback(&scenario, Edit::Replace);
        scenario.use_block();
        assert_eq!(scenario.active(), None);
        scenario.tick(64);
        let current = scenario
            .fixture
            .player
            .inventory()
            .get_stack(scenario.source);
        assert_eq!(current.item_count, 2);
        assert_ne!(current.uid, scenario.original.uid);
        assert_eq!(scenario.fixture.player.hunger_manager.level.load(), 10);
        assert_eq!(&*events.lock().unwrap(), &["interact", "consume"]);
        scenario.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_consume_callback_selection_preserves_both_sources() {
    for hand in [Hand::Right, Hand::Left] {
        let scenario = Scenario::new(&Item::CARROT, hand, &Block::STONE);
        register_consume_callback(&scenario, Edit::Select);
        scenario.use_block();
        assert_eq!(scenario.active(), (hand == Hand::Left).then_some(hand));
        scenario.tick(64);
        scenario.assert_stock_and_food(
            if hand == Hand::Left { 1 } else { 2 },
            if hand == Hand::Left { 13 } else { 10 },
        );
        assert_eq!(scenario.fixture.player.inventory().get_selected_slot(), 1);
        assert_eq!(
            scenario.fixture.player.inventory().get_stack(1).item_count,
            2
        );
        scenario.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_consume_event_cancellation_denies_fallback() {
    let scenario = Scenario::new(&Item::POTATO, Hand::Left, &Block::STONE);
    let events = register_consume_callback(&scenario, Edit::Cancel);
    scenario.use_block();
    assert_eq!(scenario.active(), None);
    scenario.tick(64);
    scenario.assert_stock_and_food(2, 10);
    assert_eq!(&*events.lock().unwrap(), &["interact", "consume"]);
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_existing_other_hand_use_is_not_replaced() {
    let scenario = Scenario::new(&Item::CARROT, Hand::Left, &Block::STONE);
    let bow = ItemStack::new(1, &Item::BOW);
    scenario
        .fixture
        .player
        .inventory()
        .set_stack(0, bow.clone());
    scenario
        .fixture
        .player
        .living_entity
        .set_active_hand(Hand::Right, bow, 72_000);
    scenario.use_block();
    assert_eq!(scenario.active(), Some(Hand::Right));
    scenario.tick(64);
    scenario.assert_stock_and_food(2, 10);
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_zero_duration_component_uses_real_consumption() {
    let mut scenario = Scenario::new(&Item::CARROT, Hand::Left, &Block::STONE);
    scenario
        .original
        .get_data_component_mut::<ConsumableImpl>()
        .unwrap()
        .consume_seconds = 0.0;
    scenario
        .fixture
        .player
        .inventory()
        .set_stack(scenario.source, scenario.original.clone());
    scenario.use_block();
    assert_eq!(scenario.active(), None);
    scenario.assert_stock_and_food(1, 13);
    scenario.tick(64);
    scenario.assert_stock_and_food(1, 13);
    scenario.finish().await;
}

mod denials;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_occupied_destination_still_allows_eating() {
    let scenario = Scenario::new(&Item::POTATO, Hand::Left, &Block::STONE);
    scenario.world.set_block_state(
        &ABOVE,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    scenario.use_block();
    assert_eq!(scenario.active(), Some(Hand::Left));
    scenario.tick(32);
    scenario.assert_stock_and_food(1, 11);
    assert_eq!(scenario.world.get_block(&ABOVE), &Block::STONE);
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_fallback_triggers_block_use_criterion_without_early_statistic() {
    use pumpkin_data::{Advancement, statistic::StatisticCategory};
    let scenario = advancement_callbacks::jukebox_scenario(Hand::Right);
    scenario.use_block();
    assert_eq!(scenario.active(), Some(Hand::Right));
    assert!(
        scenario
            .fixture
            .player
            .has_advancement(Advancement::ADVENTURE_PLAY_JUKEBOX_IN_MEADOWS)
    );
    assert_eq!(
        scenario
            .fixture
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::CARROT.id)),
        0
    );
    scenario.assert_stock_and_food(2, 10);
    scenario.tick(32);
    scenario.assert_stock_and_food(1, 13);
    assert_eq!(
        scenario
            .fixture
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::CARROT.id)),
        1
    );
    scenario.finish().await;
}

mod advancement_callbacks;
