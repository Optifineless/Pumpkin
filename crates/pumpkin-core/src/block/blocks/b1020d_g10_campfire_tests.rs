use crate::block::entities::campfire::CampfireBlockEntity;
use crate::{
    entity::EntityBase,
    net::java::{combat_test_support::TestPlayer, sound_test_support::decode_sounds},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::player_interact_event::PlayerInteractEvent,
            world::generic_game::GenericGameEvent,
        },
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, biome::Biome, block_properties::CampfireLikeProperties, game_event::GameEvent,
    item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn, ser::NetworkReadExt};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
    sync::{LockClass, Mutex},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

enum TestCaptureClass {}
impl LockClass for TestCaptureClass {
    const NAME: &'static str = "campfire test capture";
    const LEVEL: u8 = 40;
}

const POSITION: BlockPos = BlockPos::new(0, 64, 1);
struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    observer: TestPlayer,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let player = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(0.5, 64.0, 0.5));
        player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);
        world.players.store(Arc::new(vec![
            player.player.clone(),
            observer.player.clone(),
        ]));
        Self {
            server,
            world,
            player,
            observer,
            _directory: directory,
        }
    }
    fn prepare(&mut self, hand: Hand, sneaking: bool, lit: bool) {
        let mut properties = CampfireLikeProperties::default(&Block::CAMPFIRE);
        properties.lit = lit;
        self.world.set_block_state(
            &POSITION,
            properties.to_state_id(&Block::CAMPFIRE),
            BlockFlags::FORCE_STATE,
        );
        self.player.player.get_entity().set_sneaking(sneaking);
        let other = if hand == Hand::Right {
            Hand::Left
        } else {
            Hand::Right
        };
        self.player
            .player
            .inventory()
            .set_stack_in_hand(other, ItemStack::new(1, &Item::SHIELD));
        self.player
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::IRON_SHOVEL));
        self.player.take_packets();
        self.observer.take_packets();
    }
    fn use_on(&self, hand: Hand) {
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(i32::from(hand == Hand::Left)),
                    position: POSITION,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }
    fn lit(&self) -> bool {
        CampfireLikeProperties::from_state_id(self.world.get_block_state_id(&POSITION)).lit
    }
    fn damage(&self, hand: Hand) -> i32 {
        self.player
            .player
            .inventory()
            .get_stack_in_hand(hand)
            .get_damage()
    }
    async fn close(self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.world.level.shutdown(),
        )
        .await
        .unwrap()
        .unwrap();
    }
}

struct DouseEvents {
    world: std::sync::Weak<World>,
    events: Mutex<Vec<(String, bool)>, TestCaptureClass>,
}
impl EventHandler<GenericGameEvent> for DouseEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        let world = self.world.upgrade().unwrap();
        let lit = CampfireLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).lit;
        self.events
            .lock()
            .unwrap()
            .push((event.event_key.clone(), lit));
        Box::pin(async {})
    }
}
fn extinguish_events(packets: &[bytes::Bytes]) -> usize {
    packets
        .iter()
        .filter(|packet| {
            let mut data = packet.as_ref();
            data.get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::LEVEL_EVENT
                    .to_id(pumpkin_data::packet::CURRENT_MC_VERSION)
                && data.get_i32_be().unwrap() == 1009
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_campfire_shovel_shield() {
    let mut fixture = Fixture::new();
    let events = Arc::new(DouseEvents {
        world: Arc::downgrade(&fixture.world),
        events: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(events.clone(), EventPriority::Normal, true);
    for hand in [Hand::Right, Hand::Left] {
        for sneaking in [false, true] {
            fixture.prepare(hand, sneaking, true);
            events.events.lock().unwrap().clear();
            fixture.use_on(hand);
            assert!(
                !fixture.lit(),
                "block-first campfire dousing must bypass later main-hand shield intent"
            );
            assert_eq!(
                fixture.damage(hand),
                1,
                "the actual dispatched hand loses one durability"
            );
            let other = if hand == Hand::Right {
                Hand::Left
            } else {
                Hand::Right
            };
            assert_eq!(fixture.damage(other), 0);
            let packets = fixture.observer.take_packets();
            assert_eq!(extinguish_events(&packets), 1);
            assert_eq!(decode_sounds(&packets).len(), 1);
            if !sneaking {
                assert_eq!(
                    events
                        .events
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(event, _)| event == GameEvent::BlockChange.name())
                        .cloned()
                        .collect::<Vec<_>>(),
                    vec![(GameEvent::BlockChange.name().to_owned(), true)]
                );
            }
            fixture.use_on(hand);
            assert_eq!(
                fixture.damage(hand),
                1,
                "unlit campfire cannot douse or damage twice"
            );
            assert_eq!(extinguish_events(&fixture.observer.take_packets()), 0);
        }
    }
    fixture.prepare(Hand::Right, false, true);
    fixture.player.player.gamemode.store(GameMode::Creative);
    fixture.use_on(Hand::Right);
    assert!(!fixture.lit());
    assert_eq!(fixture.damage(Hand::Right), 0);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_campfire_food_full_slots_and_shield_guard_controls() {
    let mut fixture = Fixture::new();
    fixture.prepare(Hand::Right, false, true);
    let campfire = fixture.world.get_block_entity(&POSITION).unwrap();
    let inventory = campfire
        .as_any()
        .downcast_ref::<CampfireBlockEntity>()
        .unwrap();
    for slot in 0..4 {
        fixture
            .player
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::BEEF));
        fixture.use_on(Hand::Right);
        assert_eq!(inventory.get_stack(slot).item, &Item::BEEF);
        assert_eq!(fixture.player.player.inventory().held_item().item_count, 1);
        assert!(fixture.lit(), "food insertion precedes dousing");
    }
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::BEEF));
    fixture.use_on(Hand::Right);
    assert_eq!(
        fixture.player.player.inventory().held_item().item_count,
        2,
        "full campfire consumes the interaction without consuming food"
    );
    assert!(fixture.lit());
    fixture.world.set_block_state(
        &POSITION,
        Block::OAK_LOG.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::IRON_AXE));
    fixture.use_on(Hand::Right);
    assert_eq!(
        fixture.world.get_block(&POSITION),
        &Block::OAK_LOG,
        "generic axe shield-intent guard stays active"
    );
    assert_eq!(fixture.damage(Hand::Right), 0);
    fixture.close().await;
}

struct CancelUse;
impl EventHandler<PlayerInteractEvent> for CancelUse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_campfire_cancelled_and_replaced_sources() {
    let mut cancelled = Fixture::new();
    cancelled.prepare(Hand::Right, false, true);
    cancelled
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(Arc::new(CancelUse), EventPriority::Normal, true);
    cancelled.use_on(Hand::Right);
    assert!(cancelled.lit());
    assert_eq!(cancelled.damage(Hand::Right), 0);
    assert_eq!(extinguish_events(&cancelled.observer.take_packets()), 0);
    cancelled.close().await;

    let mut fixture = Fixture::new();
    fixture.prepare(Hand::Left, false, true);
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let weak = Arc::downgrade(&fixture.world);
    *fixture
        .world
        .persistence_test_hooks
        .transform_write
        .lock()
        .unwrap() = Some(Arc::new(move |position| {
        if position == POSITION && !observed.swap(true, Ordering::Relaxed) {
            weak.upgrade().unwrap().set_block_state(
                &POSITION,
                Block::DIAMOND_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE,
            );
        }
    }));
    fixture.use_on(Hand::Left);
    assert!(called.load(Ordering::Relaxed));
    assert_eq!(fixture.world.get_block(&POSITION), &Block::DIAMOND_BLOCK);
    assert_eq!(fixture.damage(Hand::Left), 0);
    let packets = fixture.observer.take_packets();
    assert_eq!(extinguish_events(&packets), 0);
    assert!(decode_sounds(&packets).is_empty());
    fixture.close().await;
}

struct ReplaceDuringDouse(std::sync::Weak<World>);
impl EventHandler<GenericGameEvent> for ReplaceDuringDouse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::BlockChange.name() {
            let world = self.0.upgrade().unwrap();
            assert!(
                CampfireLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).lit,
                "douse game event precedes the LIT write"
            );
            world.set_block_state(
                &POSITION,
                Block::DIAMOND_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE,
            );
        }
        Box::pin(async {})
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_campfire_game_event_replacement_retains_callback_state() {
    let mut fixture = Fixture::new();
    fixture.prepare(Hand::Right, false, true);
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(
            Arc::new(ReplaceDuringDouse(Arc::downgrade(&fixture.world))),
            EventPriority::Normal,
            true,
        );
    fixture.use_on(Hand::Right);
    assert_eq!(fixture.world.get_block(&POSITION), &Block::DIAMOND_BLOCK);
    assert_eq!(fixture.damage(Hand::Right), 0);
    assert_eq!(
        extinguish_events(&fixture.observer.take_packets()),
        1,
        "vanilla publishes the douse event before callbacks can interrupt the later state write"
    );
    fixture.close().await;
}
