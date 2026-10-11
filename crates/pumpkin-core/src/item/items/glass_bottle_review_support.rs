use super::super::{BottleFixture, WATER};
use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::player_statistic_increment::PlayerStatisticIncrementEvent,
            world::generic_game::GenericGameEvent,
        },
    },
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_config::op::Op;
use pumpkin_data::{
    Block, dimension::Dimension, game_event::GameEvent, item::Item, item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::{
    Hand, PermissionLvl,
    math::{vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SoundPacket {
    pub sound: i32,
    pub category: i32,
    pub position: Vector3<i32>,
    pub volume: f32,
    pub pitch: f32,
}

pub(super) fn take_sounds(player: &mut TestPlayer) -> Vec<SoundPacket> {
    player
        .take_packets()
        .into_iter()
        .filter_map(|bytes| {
            let mut data = bytes.as_ref();
            if data.get_var_int().unwrap().0 != pumpkin_data::packet::clientbound::play::SOUND.0 {
                return None;
            }
            // Current Java sound holders encode a registered ID plus one.
            let sound = data.get_var_int().unwrap().0 - 1;
            assert!(sound >= 0, "this fixture emits registered sounds");
            Some(SoundPacket {
                sound,
                category: data.get_var_int().unwrap().0,
                position: Vector3::new(
                    data.get_i32().unwrap(),
                    data.get_i32().unwrap(),
                    data.get_i32().unwrap(),
                ),
                volume: data.get_f32().unwrap(),
                pitch: data.get_f32().unwrap(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    FluidPickup,
    Statistic,
}

pub(super) struct Observation {
    pub stage: Stage,
    pub position: Option<Vector3<f64>>,
    pub held: ItemStack,
    pub used: i32,
    pub sounds: Vec<SoundPacket>,
}

pub(super) struct Effects {
    player: Arc<Player>,
    hand: Hand,
    observer: Mutex<TestPlayer>,
    pub observations: Mutex<Vec<Observation>>,
    pub replacement: Mutex<Option<(Stage, ItemStack)>>,
    pub selection: Mutex<Option<(Stage, u8)>>,
}

impl Effects {
    pub fn watch(fixture: &mut BottleFixture, hand: Hand) -> Arc<Self> {
        let mut observer = TestPlayer::new(&fixture.world);
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(9.5, 64.0, 8.5));
        fixture.world.players.store(Arc::new(vec![
            fixture.user.player.clone(),
            observer.player.clone(),
        ]));
        fixture.user.take_packets();
        observer.take_packets();
        let effects = Arc::new(Self {
            player: fixture.user.player.clone(),
            hand,
            observer: Mutex::new(observer),
            observations: Mutex::new(Vec::new()),
            replacement: Mutex::new(None),
            selection: Mutex::new(None),
        });
        fixture
            .server
            .plugin_manager
            .register::<GenericGameEvent, _>(effects.clone(), EventPriority::Normal, true);
        fixture
            .server
            .plugin_manager
            .register::<PlayerStatisticIncrementEvent, _>(
                effects.clone(),
                EventPriority::Normal,
                true,
            );
        effects
    }

    fn record(&self, stage: Stage, position: Option<Vector3<f64>>) {
        let held = self.player.inventory().get_stack_in_hand(self.hand);
        let used = self
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id));
        let sounds = take_sounds(&mut self.observer.lock().unwrap());
        self.observations.lock().unwrap().push(Observation {
            stage,
            position,
            held,
            used,
            sounds,
        });
        let replacement = self
            .replacement
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(at, _)| *at == stage)
            .map(|(_, stack)| stack.clone());
        if let Some(replacement) = replacement {
            self.player
                .inventory()
                .set_stack_in_hand(self.hand, replacement);
        }
        let selection = self
            .selection
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(at, _)| *at == stage)
            .map(|(_, slot)| *slot);
        if let Some(slot) = selection {
            self.player.inventory().set_selected_slot(slot);
        }
    }

    pub fn observer_sounds(&self) -> Vec<SoundPacket> {
        let mut sounds: Vec<_> = self
            .observations
            .lock()
            .unwrap()
            .iter()
            .flat_map(|event| event.sounds.clone())
            .collect();
        sounds.extend(take_sounds(&mut self.observer.lock().unwrap()));
        sounds
    }
}

impl EventHandler<GenericGameEvent> for Effects {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::FluidPickup.name() {
            self.record(Stage::FluidPickup, Some(event.position));
        }
        Box::pin(async {})
    }
}

impl EventHandler<PlayerStatisticIncrementEvent> for Effects {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.gameprofile.id == self.player.gameprofile.id
            && event.statistic_id
                == format!("{:?}:{}", StatisticCategory::Used, Item::GLASS_BOTTLE.id)
        {
            self.record(Stage::Statistic, None);
        }
        Box::pin(async {})
    }
}

pub(super) fn add_operator(fixture: &BottleFixture, id: uuid::Uuid) {
    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap()
        .ops
        .push(Op::new(
            id,
            "bottle-operator".to_owned(),
            PermissionLvl::One,
            false,
        ));
}

pub(super) fn nether_fixture() -> BottleFixture {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let dimension = Dimension::THE_NETHER;
    let world = Arc::new(World::load(
        pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            dir.path().to_path_buf(),
            0,
            dimension.clone(),
        ),
        server.level_info.clone(),
        dimension,
        server.block_registry.clone(),
        Arc::downgrade(&server),
    ));
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    world.set_block_state(
        &WATER,
        Block::WATER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let user = TestPlayer::new(&world);
    user.player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    BottleFixture {
        _dir: dir,
        server,
        world,
        user,
    }
}
