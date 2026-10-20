#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real-entry regression fixtures use bounded setup and valid captured packets"
)]

use crate::{
    entity::{EntityBase, vehicle::boat::BoatEntity},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        vehicle::{
            vehicle_damage::VehicleDamageEvent, vehicle_destroy::VehicleDestroyEvent,
            vehicle_update::VehicleUpdateEvent,
        },
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    data_component_impl::CustomNameImpl,
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    packet::clientbound::play::{REMOVE_ENTITIES, SET_ENTITY_DATA},
};
use pumpkin_protocol::{VarInt, java::server::play::SAttack};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    text::TextComponent,
};
use pumpkin_world::world::BlockFlags;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

pub const DEADLINE: Duration = Duration::from_secs(10);
pub const CHUNK: Vector2<i32> = Vector2::new(0, 0);
pub const POS: Vector3<f64> = Vector3::new(8.5, 64.0, 8.5);
pub const NAME: &str = "G06 named vessel";

pub struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: TestPlayer,
    pub directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.random_tick_speed = 0;
            info.game_rules.spawn_mobs = false;
            info
        });
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 12.5));
        player
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(CHUNK);
        Self {
            server,
            world,
            player,
            directory,
        }
    }

    pub fn boat(&self, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let boat = crate::entity::r#type::from_type(kind, POS, &self.world, uuid::Uuid::new_v4());
        boat.get_entity().set_custom_name(TextComponent::text(NAME));
        self.world.spawn_entity_non_save(boat.clone());
        assert!(
            self.world
                .get_entity_by_uuid(boat.get_entity().entity_uuid)
                .is_some()
        );
        assert!(
            self.world
                .entity_tracker
                .get_tracked_entity(boat.get_entity().entity_id)
                .is_some()
        );
        boat
    }

    pub fn set_block(&self, pos: BlockPos, block: &'static Block) {
        self.world.set_block_state(
            &pos,
            block.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    pub fn fill_contact(&self, block: &'static Block) {
        for x in 7..=9 {
            for z in 7..=9 {
                self.set_block(BlockPos::new(x, 64, z), block);
            }
        }
    }

    pub fn tick(&self, count: usize) {
        assert!(count <= 40, "all regression tick loops are bounded");
        for _ in 0..count {
            self.world.tick(&self.server);
        }
    }

    pub fn attack(&self, boat: &dyn EntityBase) {
        self.player
            .player
            .last_attacked_ticks
            .store(100, Ordering::Relaxed);
        self.player.client().handle_attack(
            &self.player.player,
            &SAttack {
                entity_id: boat.get_entity().entity_id.into(),
            },
            &self.server,
        );
        assert!(!self.player.client().is_closed());
    }

    pub fn drops(&self) -> Vec<ItemStack> {
        drops(&self.world)
    }

    pub fn other_world(&self, dimension: pumpkin_data::dimension::Dimension) -> Arc<World> {
        let level = pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            self.directory.path().join("destination"),
            0,
            dimension.clone(),
        );
        let destination = Arc::new(World::load(
            level,
            self.server.level_info.clone(),
            dimension,
            self.server.block_registry.clone(),
            Arc::downgrade(&self.server),
        ));
        combat_test_support::publish_empty_chunk(&destination, CHUNK);
        self.server
            .worlds
            .store(Arc::new(vec![self.world.clone(), destination.clone()]));
        destination
    }

    pub fn assert_named_drop(&self, item: &'static Item) {
        let stacks = self.drops();
        assert_eq!(
            stacks.len(),
            1,
            "destruction must publish one live vehicle-item entity"
        );
        assert_eq!(
            stacks[0].item, item,
            "drop must use the concrete generated boat/raft/chest item"
        );
        assert_eq!(
            stacks[0].item_count, 1,
            "vehicle-item count must be exactly one"
        );
        assert_eq!(
            stacks[0]
                .get_data_component::<CustomNameImpl>()
                .map(|component| component.name.clone()),
            Some(TextComponent::text(NAME)),
            "vehicle item must preserve the entity custom name"
        );
    }

    pub async fn finish(self) {
        for world in self.server.worlds.load_full().iter() {
            tokio::time::timeout(DEADLINE, world.level.shutdown())
                .await
                .unwrap()
                .unwrap();
        }
    }
}

pub fn drops(world: &World) -> Vec<ItemStack> {
    world
        .entities
        .load()
        .iter()
        .filter(|entity| !entity.get_entity().is_removed())
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| item.get_item_stack().lock().unwrap().clone())
        .collect()
}

pub fn boat(boat: &dyn EntityBase) -> &BoatEntity {
    boat.cast_any().downcast_ref::<BoatEntity>().unwrap()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    Damage(f32, Option<i32>),
    Destroy(Option<i32>),
    Update,
}

#[derive(Default)]
pub struct Recorder {
    pub cancel_damage: AtomicBool,
    pub cancel_destroy: AtomicBool,
    pub observed: Mutex<Vec<Observed>>,
}

impl Recorder {
    pub fn register(server: &Server) -> Arc<Self> {
        let recorder = Arc::new(Self::default());
        server.plugin_manager.register::<VehicleDamageEvent, _>(
            recorder.clone(),
            EventPriority::Lowest,
            true,
        );
        server.plugin_manager.register::<VehicleDestroyEvent, _>(
            recorder.clone(),
            EventPriority::Lowest,
            true,
        );
        server.plugin_manager.register::<VehicleUpdateEvent, _>(
            recorder.clone(),
            EventPriority::Lowest,
            true,
        );
        recorder
    }

    pub fn damage_count(&self) -> usize {
        self.observed
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, Observed::Damage(..)))
            .count()
    }

    pub fn destroy_count(&self) -> usize {
        self.observed
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, Observed::Destroy(..)))
            .count()
    }
}

impl EventHandler<VehicleDamageEvent> for Recorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut VehicleDamageEvent,
    ) -> BoxFuture<'a, ()> {
        self.observed
            .lock()
            .unwrap()
            .push(Observed::Damage(event.damage, event.attacker_id));
        event.cancelled = self.cancel_damage.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

impl EventHandler<VehicleDestroyEvent> for Recorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut VehicleDestroyEvent,
    ) -> BoxFuture<'a, ()> {
        self.observed
            .lock()
            .unwrap()
            .push(Observed::Destroy(event.attacker_id));
        event.cancelled = self.cancel_destroy.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

impl EventHandler<VehicleUpdateEvent> for Recorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleUpdateEvent,
    ) -> BoxFuture<'a, ()> {
        self.observed.lock().unwrap().push(Observed::Update);
        Box::pin(async {})
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Stage {
    Damage,
    Destroy,
}

pub struct Action {
    run: Box<dyn Fn() + Send + Sync>,
    called: AtomicBool,
}

impl Action {
    pub fn register(server: &Server, stage: Stage, run: impl Fn() + Send + Sync + 'static) {
        let action = Arc::new(Self {
            run: Box::new(run),
            called: AtomicBool::new(false),
        });
        match stage {
            Stage::Damage => server.plugin_manager.register::<VehicleDamageEvent, _>(
                action,
                EventPriority::Normal,
                true,
            ),
            Stage::Destroy => server.plugin_manager.register::<VehicleDestroyEvent, _>(
                action,
                EventPriority::Normal,
                true,
            ),
        }
    }

    fn run_once(&self) {
        if !self.called.swap(true, Ordering::AcqRel) {
            (self.run)();
        }
    }
}

impl EventHandler<VehicleDamageEvent> for Action {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleDamageEvent,
    ) -> BoxFuture<'a, ()> {
        self.run_once();
        Box::pin(async {})
    }
}

impl EventHandler<VehicleDestroyEvent> for Action {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleDestroyEvent,
    ) -> BoxFuture<'a, ()> {
        self.run_once();
        Box::pin(async {})
    }
}

pub struct Gate {
    reached: mpsc::Receiver<()>,
    resume: mpsc::SyncSender<()>,
}

impl Gate {
    pub fn register(server: &Server, stage: Stage) -> Self {
        let (reached, observation) = mpsc::sync_channel(1);
        let (resume, continuation) = mpsc::sync_channel(1);
        let continuation = Mutex::new(Some(continuation));
        Action::register(server, stage, move || {
            let continuation = continuation.lock().unwrap().take().unwrap();
            reached.send(()).unwrap();
            continuation
                .recv_timeout(DEADLINE)
                .expect("damage callback was not resumed within its deadline");
        });
        Self {
            reached: observation,
            resume,
        }
    }

    pub fn reached(&self) {
        self.reached
            .recv_timeout(DEADLINE)
            .expect("actual vehicle callback did not reach bounded barrier");
    }

    pub fn resume(&self) {
        self.resume.send(()).unwrap();
    }
}

pub struct Worker {
    completed: mpsc::Receiver<bool>,
    thread: JoinHandle<()>,
}

impl Worker {
    pub fn damage(boat: Arc<dyn EntityBase>) -> Self {
        let runtime = tokio::runtime::Handle::current();
        let (completed, result) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let _runtime = runtime.enter();
            completed
                .send(boat.damage(
                    boat.as_ref(),
                    5.0,
                    pumpkin_data::damage::DamageType::GENERIC,
                ))
                .unwrap();
        });
        Self {
            completed: result,
            thread,
        }
    }

    pub fn finish(self) -> bool {
        let result = self
            .completed
            .recv_timeout(DEADLINE)
            .expect("damage worker exceeded bounded completion deadline");
        self.thread.join().unwrap();
        result
    }
}

pub fn packets(player: &mut TestPlayer, target: i32) -> (usize, usize) {
    let mut removed = 0;
    let mut metadata = 0;
    for packet in player.take_packets() {
        let mut reader = packet.as_ref();
        let id = VarInt::decode(&mut reader).unwrap().0;
        if id == REMOVE_ENTITIES.0 {
            let count = VarInt::decode(&mut reader).unwrap().0;
            assert!((0..=64).contains(&count));
            for _ in 0..count {
                removed += usize::from(VarInt::decode(&mut reader).unwrap().0 == target);
            }
        } else if id == SET_ENTITY_DATA.0 {
            metadata += usize::from(VarInt::decode(&mut reader).unwrap().0 == target);
        }
    }
    (removed, metadata)
}
