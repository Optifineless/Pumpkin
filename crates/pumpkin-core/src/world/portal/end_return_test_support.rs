#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Portal regression fixture construction"
)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::future::BoxFuture;
use pumpkin_data::{Block, biome::Biome, dimension::Dimension};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2},
    world_seed::Seed,
};
use pumpkin_world::{
    chunk_system::chunk_state::Chunk,
    generation::{
        generator::{WorldGenerator, flat::FlatGenerator},
        proto_chunk::ProtoChunk,
    },
};
use tokio::sync::oneshot;

use super::residency::PortalChunkResidency;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        EventHandler, EventPriority,
        api::events::player::player_changed_world::PlayerChangedWorldEvent,
    },
    server::Server,
    world::World,
};

pub(super) const PORTAL: BlockPos = BlockPos::new(8, 80, 8);
pub(super) const PERSONAL: BlockPos = BlockPos::new(8, 64, 8);
pub(super) const DEADLINE: Duration = Duration::from_secs(45);

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub overworld: Arc<World>,
    pub end: Arc<World>,
    pub nether: Arc<World>,
    retained: Vec<PortalChunkResidency>,
    _owner: DeathTestWorld,
}

impl Fixture {
    pub async fn new() -> Self {
        let owner = tokio::time::timeout(DEADLINE, DeathTestWorld::with_end())
            .await
            .unwrap();
        let server = owner.server.clone();
        let overworld = owner.world();
        let end = server.get_world_from_dimension(&Dimension::THE_END);
        let nether = server.get_world_from_dimension(&Dimension::THE_NETHER);
        assert_eq!(end.dimension, Dimension::THE_END);
        let mut retained = Vec::new();
        for world in [&overworld, &end, &nether] {
            let mut residency = PortalChunkResidency::new(world.level.clone());
            residency.add_area(PERSONAL, 2);
            publish_floor(world, Vector2::new(0, 0));
            retained.push(residency);
        }
        let mut platform = PortalChunkResidency::new(end.level.clone());
        platform.add_area(BlockPos::new(100, 49, 0), 3);
        for z in [-1, 0] {
            publish_floor(&end, Vector2::new(6, z));
        }
        retained.push(platform);
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.spawn_x = 2;
            info.spawn_z = 2;
            info.spawn_y = 200;
            info.spawn_yaw = 33.0;
            info.spawn_pitch = 7.0;
            info.game_rules.respawn_radius = 0;
            info
        });
        Self {
            server,
            overworld,
            end,
            nether,
            retained,
            _owner: owner,
        }
    }

    pub fn returning_player(&self) -> TestPlayer {
        let client = TestPlayer::new(&self.end);
        client.player.get_entity().set_pos(PORTAL.to_centered_f64());
        client
            .player
            .seen_credits
            .store(true, std::sync::atomic::Ordering::Relaxed);
        client
    }

    pub async fn finish(self) {
        drop(self.retained);
        tokio::time::timeout(DEADLINE, self.server.shutdown())
            .await
            .unwrap();
    }
}

fn publish_floor(world: &World, pos: Vector2<i32>) {
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        world.dimension.clone(),
        Vec::new(),
        Biome::PLAINS.registry_id.to_owned(),
    )));
    let mut proto = ProtoChunk::new(pos.x, pos.y, &generator);
    for x in 0..16 {
        for z in 0..16 {
            proto.set_block_state(x, 63, z, Block::STONE.default_state);
        }
    }
    let mut chunk = Chunk::Proto(Box::new(proto));
    chunk.upgrade_to_level_chunk(
        &world.dimension,
        &pumpkin_config::lighting::LightingEngineConfig::default(),
    );
    let Chunk::Level(chunk) = chunk else {
        panic!("fixture chunk must be full")
    };
    world.level.loaded_chunks.insert(pos, chunk);
}

// Load both formats through the production player reader; these retain their saved meanings.
pub(super) fn load_personal(player: &Player, dimension: &Dimension, legacy: bool) {
    let mut nbt = NbtCompound::new();
    nbt.put_bool("seenCredits", true);
    if legacy {
        nbt.put_int("SpawnX", PERSONAL.0.x);
        nbt.put_int("SpawnY", PERSONAL.0.y);
        nbt.put_int("SpawnZ", PERSONAL.0.z);
        nbt.put_string("SpawnDimension", dimension.minecraft_name.to_owned());
        nbt.put_bool("SpawnForced", false);
    } else {
        let mut respawn = NbtCompound::new();
        respawn.put_string("dimension", dimension.minecraft_name.to_owned());
        respawn.put(
            "pos",
            NbtTag::IntArray(vec![PERSONAL.0.x, PERSONAL.0.y, PERSONAL.0.z]),
        );
        respawn.put_float("angle", 27.0);
        respawn.put_bool("forced", false);
        nbt.put_compound("respawn", respawn);
    }
    player.read_custom_nbt(&nbt);
}

pub(super) fn contact(server: &Server, entity: &dyn EntityBase) {
    let world = entity.get_entity().world.load_full();
    world.set_block_state(
        &PORTAL,
        Block::END_PORTAL.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    world.block_registry.on_entity_collision(
        &Block::END_PORTAL,
        &world,
        entity,
        &PORTAL,
        world.get_block_state(&PORTAL),
        server,
    );
    assert!(
        entity.get_entity().portal_manager.lock().unwrap().is_some(),
        "registered End portal must admit the contact"
    );
    entity.get_entity().tick(entity, server);
}

struct Arrived(Mutex<Option<oneshot::Sender<()>>>);
impl EventHandler<PlayerChangedWorldEvent> for Arrived {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerChangedWorldEvent,
    ) -> BoxFuture<'a, ()> {
        if let Some(done) = self.0.lock().unwrap().take() {
            let _ = done.send(());
        }
        Box::pin(async {})
    }
}

pub(super) async fn transfer(fixture: &Fixture, client: &mut TestPlayer) {
    let (done, completed) = oneshot::channel();
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangedWorldEvent, _>(
            Arc::new(Arrived(Mutex::new(Some(done)))),
            EventPriority::Lowest,
            true,
        );
    contact(&fixture.server, client.player.as_ref());
    tokio::time::timeout(DEADLINE, client.with_outgoing_writer(completed))
        .await
        .unwrap()
        .unwrap();
}
