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
use pumpkin_inventory::Inventory;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::{
    Hand, PermissionLvl,
    math::{vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, mpsc};

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

enum ObserverCommand {
    TakeSounds(mpsc::Sender<Vec<SoundPacket>>),
}

pub(super) struct Effects {
    player: Arc<Player>,
    observer: mpsc::Sender<ObserverCommand>,
    observations: mpsc::Receiver<Observation>,
    sounds: mpsc::Receiver<Vec<SoundPacket>>,
}

impl Effects {
    pub fn watch(fixture: &mut BottleFixture, hand: Hand) -> Self {
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
        let (observer_sender, observer_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(ObserverCommand::TakeSounds(reply)) = observer_receiver.recv() {
                let _ = reply.send(take_sounds(&mut observer));
            }
        });
        let (observation_sender, observations) = mpsc::channel();
        let (sound_sender, sounds) = mpsc::channel();
        let effects = Self {
            player: fixture.user.player.clone(),
            observer: observer_sender.clone(),
            observations,
            sounds,
        };
        let handler = Arc::new(EffectHandler {
            player: fixture.user.player.clone(),
            hand,
            observer: observer_sender,
            observations: observation_sender,
            sounds: sound_sender,
        });
        fixture
            .server
            .plugin_manager
            .register::<GenericGameEvent, _>(handler.clone(), EventPriority::Normal, true);
        fixture
            .server
            .plugin_manager
            .register::<PlayerStatisticIncrementEvent, _>(handler, EventPriority::Normal, true);
        effects
    }

    fn take_observer_sounds(&self) -> Vec<SoundPacket> {
        let (reply, sounds) = mpsc::channel();
        if self
            .observer
            .send(ObserverCommand::TakeSounds(reply))
            .is_ok()
        {
            return sounds.recv().unwrap_or_default();
        }
        Vec::new()
    }

    pub fn take_observations(&self) -> Vec<Observation> {
        self.observations.try_iter().collect()
    }

    pub fn observer_sounds(&self) -> Vec<SoundPacket> {
        self.sounds
            .try_iter()
            .flatten()
            .chain(self.take_observer_sounds())
            .collect()
    }

    pub fn replace_at(&self, stage: Stage, replacement: ItemStack) {
        let inventory = self.player.inventory();
        inventory.set_stack(REVIEW_REPLACEMENT_SLOT, replacement);
        inventory.set_stack(
            REVIEW_MARKER_SLOT,
            ItemStack::new(1, replacement_marker(stage)),
        );
    }

    pub fn select_at(&self, stage: Stage, slot: u8) {
        assert_eq!(slot, 1, "review fixtures use the second hotbar slot");
        self.player.inventory().set_stack(
            REVIEW_MARKER_SLOT,
            ItemStack::new(1, selection_marker(stage)),
        );
    }
}

const REVIEW_MARKER_SLOT: usize = 8;
const REVIEW_REPLACEMENT_SLOT: usize = 9;

fn replacement_marker(stage: Stage) -> &'static Item {
    match stage {
        Stage::FluidPickup => &Item::STICK,
        Stage::Statistic => &Item::DIAMOND,
    }
}

fn selection_marker(stage: Stage) -> &'static Item {
    match stage {
        Stage::FluidPickup => &Item::EMERALD,
        Stage::Statistic => &Item::IRON_INGOT,
    }
}

struct EffectHandler {
    player: Arc<Player>,
    hand: Hand,
    observer: mpsc::Sender<ObserverCommand>,
    observations: mpsc::Sender<Observation>,
    sounds: mpsc::Sender<Vec<SoundPacket>>,
}

impl EffectHandler {
    fn take_observer_sounds(&self) -> Vec<SoundPacket> {
        let (reply, sounds) = mpsc::channel();
        if self
            .observer
            .send(ObserverCommand::TakeSounds(reply))
            .is_ok()
        {
            return sounds.recv().unwrap_or_default();
        }
        Vec::new()
    }

    fn record(&self, stage: Stage, position: Option<Vector3<f64>>) {
        let inventory = self.player.inventory();
        let held = inventory.get_stack_in_hand(self.hand);
        let used = self
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id));
        let sounds = self.take_observer_sounds();
        let _ = self.sounds.send(sounds.clone());
        let _ = self.observations.send(Observation {
            stage,
            position,
            held,
            used,
            sounds,
        });
        self.apply_control(stage);
    }

    fn apply_control(&self, stage: Stage) {
        let inventory = self.player.inventory();
        let marker = inventory.get_stack(REVIEW_MARKER_SLOT);
        if marker.item == replacement_marker(stage) {
            let replacement = inventory.get_stack(REVIEW_REPLACEMENT_SLOT);
            inventory.set_stack_in_hand(self.hand, replacement);
            inventory.set_stack(REVIEW_REPLACEMENT_SLOT, ItemStack::EMPTY.clone());
            inventory.set_stack(REVIEW_MARKER_SLOT, ItemStack::EMPTY.clone());
        } else if marker.item == selection_marker(stage) {
            inventory.set_selected_slot(1);
            inventory.set_stack(REVIEW_MARKER_SLOT, ItemStack::EMPTY.clone());
        }
    }
}

impl EventHandler<GenericGameEvent> for EffectHandler {
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

impl EventHandler<PlayerStatisticIncrementEvent> for EffectHandler {
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
