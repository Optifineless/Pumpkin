#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use crate::{
    entity::EntityBase,
    net::java::{
        combat_test_support::TestPlayer,
        sound_test_support::{SoundPacket, decode_sounds},
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::{
                player_interact_event::{InteractAction, PlayerInteractEvent},
                player_statistic_increment::PlayerStatisticIncrementEvent,
            },
            world::generic_game::GenericGameEvent,
        },
    },
    server::{Server, combat_test_support},
    world::{BlockFlags, World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockState, biome::Biome, item::Item, item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};

pub(super) enum BottleTestLockClass {}

impl pumpkin_util::sync::LockClass for BottleTestLockClass {
    const NAME: &'static str = "glass-bottle test recorder";
    const LEVEL: u8 = 40;
}

pub(super) type RecorderMutex<T> = pumpkin_util::sync::Mutex<T, BottleTestLockClass>;

pub(super) const WATER_POSITION: BlockPos = BlockPos::new(8, 65, 11);

pub(super) struct Fixture {
    _directory: tempfile::TempDir,
    pub(super) server: Arc<Server>,
    pub(super) world: Arc<World>,
    pub(super) actor: TestPlayer,
    pub(super) observer: Arc<RecorderMutex<TestPlayer>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let mut actor = TestPlayer::new(&world);
        let mut observer = TestPlayer::new(&world);
        actor
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        actor.player.get_entity().set_rotation(0.0, 0.0);
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(9.5, 64.0, 8.5));
        world.players.store(Arc::new(vec![
            actor.player.clone(),
            observer.player.clone(),
        ]));
        actor.take_packets();
        observer.take_packets();
        Self {
            _directory: directory,
            server,
            world,
            actor,
            observer: Arc::new(RecorderMutex::new(observer)),
        }
    }

    pub(super) fn put_state(&self, position: BlockPos, state: &'static BlockState) {
        self.world
            .set_block_state(&position, state.id, BlockFlags::FORCE_STATE);
    }

    pub(super) fn put_source_water(&self, position: BlockPos) {
        self.put_state(position, Block::WATER.default_state);
    }

    pub(super) fn hold_bottle(&self, hand: Hand, count: u8) {
        self.actor
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(count, &Item::GLASS_BOTTLE));
    }

    pub(super) fn use_item(&self, hand: Hand) {
        self.actor.client().handle_use_item(
            &self.actor.player,
            &SUseItem {
                hand: VarInt(i32::from(hand == Hand::Left)),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            &self.server,
        );
    }

    pub(super) fn used_bottle_stat(&self) -> i32 {
        self.actor
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id))
    }

    pub(super) fn potion_count(&self) -> u32 {
        (0..self.actor.player.inventory().size())
            .map(|slot| self.actor.player.inventory().get_stack(slot))
            .filter(|stack| stack.item == &Item::POTION)
            .map(|stack| u32::from(stack.item_count))
            .sum()
    }

    pub(super) async fn finish(self) {
        assert!(self.world.level.shutdown().await.is_ok());
    }
}

pub(super) struct InteractSnapshot {
    pub(super) action: InteractAction,
    pub(super) clicked_pos: Option<BlockPos>,
    pub(super) block: &'static Block,
}

pub(super) struct InteractRecorder {
    hand: Hand,
    cancel: bool,
    replacement: Option<ItemStack>,
    pub(super) events: RecorderMutex<Vec<InteractSnapshot>>,
}

impl InteractRecorder {
    fn new(hand: Hand, cancel: bool, replacement: Option<ItemStack>) -> Self {
        Self {
            hand,
            cancel,
            replacement,
            events: RecorderMutex::new(Vec::new()),
        }
    }
}

impl EventHandler<PlayerInteractEvent> for InteractRecorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push(InteractSnapshot {
            action: event.action.clone(),
            clicked_pos: event.clicked_pos,
            block: event.block,
        });
        if let Some(replacement) = &self.replacement {
            event
                .player
                .inventory()
                .set_stack_in_hand(self.hand, replacement.clone());
        }
        event.cancelled = self.cancel;
        Box::pin(async {})
    }
}

pub(super) struct GameSnapshot {
    pub(super) key: String,
    pub(super) position: Vector3<f64>,
    pub(super) hand_item: &'static Item,
    pub(super) used_stat: i32,
    pub(super) sounds_already_sent: Vec<SoundPacket>,
}

pub(super) struct GameRecorder {
    pub(super) player: Arc<crate::entity::player::Player>,
    pub(super) observer: Arc<RecorderMutex<TestPlayer>>,
    pub(super) events: RecorderMutex<Vec<GameSnapshot>>,
}

impl EventHandler<GenericGameEvent> for GameRecorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        let hand = self.player.inventory().get_stack_in_hand(Hand::Right);
        let sounds_already_sent = decode_sounds(&self.observer.lock().unwrap().take_packets());
        self.events.lock().unwrap().push(GameSnapshot {
            key: event.event_key.clone(),
            position: event.position,
            hand_item: hand.item,
            used_stat: self
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
            sounds_already_sent,
        });
        Box::pin(async {})
    }
}

pub(super) struct StatSnapshot {
    pub(super) statistic_id: String,
    pub(super) amount: i32,
    pub(super) hand_item: &'static Item,
    pub(super) previous_value: i32,
}

pub(super) struct StatRecorder {
    pub(super) player: Arc<crate::entity::player::Player>,
    pub(super) events: RecorderMutex<Vec<StatSnapshot>>,
}

impl EventHandler<PlayerStatisticIncrementEvent> for StatRecorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        let hand = self.player.inventory().get_stack_in_hand(Hand::Right);
        self.events.lock().unwrap().push(StatSnapshot {
            statistic_id: event.statistic_id.clone(),
            amount: event.amount,
            hand_item: hand.item,
            previous_value: self
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
        });
        Box::pin(async {})
    }
}

pub(super) fn register_interact(
    fixture: &Fixture,
    hand: Hand,
    cancel: bool,
    replacement: Option<ItemStack>,
) -> Arc<InteractRecorder> {
    let recorder = Arc::new(InteractRecorder::new(hand, cancel, replacement));
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(recorder.clone(), EventPriority::Normal, true);
    recorder
}
