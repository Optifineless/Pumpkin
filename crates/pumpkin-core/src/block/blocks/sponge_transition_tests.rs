#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use crate::{
    block::{
        BlockBehaviour, GetStateForNeighborUpdateArgs, OnNeighborUpdateArgs,
        blocks::sponge::{SpongeBlock, WetSpongeBlock},
        registry::BlockRegistry,
    },
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, Payload},
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockDirection, BlockId, BlockStateId,
    biome::Biome,
    item::Item,
    item_stack::ItemStack,
    packet::clientbound::play::{SECTION_BLOCKS_UPDATE, SOUND},
    sound::Sound,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_protocol::{
    ServerPacket, VarInt,
    java::{client::play::CMultiBlockUpdate, server::play::SUseItemOn},
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};

const SPONGE: BlockPos = BlockPos::new(8, 70, 8);
const PROBE: BlockPos = BlockPos::new(8, 70, 9);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhysicsDispatch {
    target: BlockPos,
    source: BlockPos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NeighborCallback {
    target: BlockPos,
    source_block: BlockId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShapeCallback {
    target: BlockPos,
    neighbor: BlockPos,
}

struct ProbeRecords {
    neighbors: Sender<NeighborCallback>,
    shapes: Sender<ShapeCallback>,
}

#[pumpkin_block("minecraft:stone")]
struct TransitionProbe(Arc<ProbeRecords>);

impl BlockBehaviour for TransitionProbe {
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let _ = self.0.neighbors.send(NeighborCallback {
            target: *args.position,
            source_block: args.source_block.id,
        });
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let _ = self.0.shapes.send(ShapeCallback {
            target: *args.position,
            neighbor: *args.neighbor_position,
        });
        args.state_id
    }
}

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    neighbor_callbacks: Receiver<NeighborCallback>,
    shape_callbacks: Receiver<ShapeCallback>,
    _chunk: Arc<pumpkin_world::chunk::ChunkData>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new(states: &[(BlockPos, BlockStateId)]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let ordinary_world = combat_test_support::world(&server, directory.path());
        let (neighbor_sender, neighbor_callbacks) = std::sync::mpsc::channel();
        let (shape_sender, shape_callbacks) = std::sync::mpsc::channel();
        let records = Arc::new(ProbeRecords {
            neighbors: neighbor_sender,
            shapes: shape_sender,
        });
        let mut registry = BlockRegistry::default();
        registry.register(SpongeBlock);
        registry.register(WetSpongeBlock);
        registry.register(TransitionProbe(records));
        let world = Arc::new(World::load(
            ordinary_world.level.clone(),
            ordinary_world.level_info.clone(),
            ordinary_world.dimension.clone(),
            Arc::new(registry),
            Arc::downgrade(&server),
        ));
        let mut proto = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        for (position, state_id) in states {
            proto.set_block_state(
                position.0.x,
                position.0.y,
                position.0.z,
                state_id.to_state(),
            );
        }
        let chunk = spawn_test_support::publish(&world, proto);
        world
            .active_chunks
            .write()
            .unwrap()
            .insert(Vector2::new(0, 0));
        Self {
            server,
            world,
            neighbor_callbacks,
            shape_callbacks,
            _chunk: chunk,
            _directory: directory,
        }
    }

    fn register<E>(&self, callback: impl Fn(&mut E) + Send + Sync + 'static)
    where
        E: Payload + Send + Sync + 'static,
    {
        self.server.plugin_manager.register::<E, _>(
            Arc::new(Callback(callback)),
            EventPriority::Normal,
            true,
        );
    }

    fn place_sponge_with_item(&self, player: &TestPlayer) {
        player
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::SPONGE));
        player
            .client()
            .handle_use_item_on(
                &player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: SPONGE.down(),
                    face: VarInt(BlockDirection::Up as i32),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

struct Callback<F>(F);

impl<E, F> EventHandler<E> for Callback<F>
where
    E: Payload,
    F: Fn(&mut E) + Send + Sync,
{
    fn handle_blocking<'a>(&'a self, _: &'a Arc<Server>, event: &'a mut E) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn water() -> BlockStateId {
    Block::WATER.default_state.id
}

fn block_updates(player: &TestPlayer, packets: &[bytes::Bytes]) -> Vec<(BlockPos, BlockStateId)> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            let packet_id = VarInt::decode(&mut data).ok()?.0;
            let version = player.client().version.load();
            (packet_id == SECTION_BLOCKS_UPDATE.0)
                .then(|| CMultiBlockUpdate::read(&mut data, &version).unwrap())
        })
        .flat_map(|packet| packet.updates)
        .collect()
}

fn count_sound(packets: &[bytes::Bytes], sound: Sound) -> usize {
    packets
        .iter()
        .filter(|packet| {
            let mut data = packet.as_ref();
            let Ok(packet_id) = VarInt::decode(&mut data) else {
                return false;
            };
            // Sound packets encode the registry id as the second VarInt for current Java clients.
            if packet_id.0 != SOUND.0 {
                return false;
            }
            VarInt::decode(&mut data).is_ok_and(|id| id.0 == i32::from(sound as u16) + 1)
        })
        .count()
}

fn record_physics(fixture: &Fixture, records: Sender<PhysicsDispatch>) {
    fixture.register::<crate::plugin::api::events::block::block_physics::BlockPhysicsEvent>(
        move |event| {
            let _ = records.send(PhysicsDispatch {
                target: event.block_pos,
                source: event.changed_pos,
            });
        },
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g11_sponge_wet_transition_omits_extra_ordinary_neighbor() {
    let water_east = SPONGE.offset((1, 0, 0).into());
    let water_southeast = SPONGE.offset((1, 0, 1).into());
    let fixture = Fixture::new(&[
        (SPONGE.down(), Block::STONE.default_state.id),
        (PROBE, Block::STONE.default_state.id),
        (water_east, water()),
        (water_southeast, water()),
    ]);
    let mut player = TestPlayer::new(&fixture.world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 70.0, 12.5));
    player.take_packets();

    let (physics_sender, physics) = std::sync::mpsc::channel();
    record_physics(&fixture, physics_sender);
    fixture.place_sponge_with_item(&player);

    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    assert_eq!(fixture.world.get_block(&water_east), &Block::AIR);
    assert_eq!(fixture.world.get_block(&water_southeast), &Block::AIR);
    fixture.world.flush_block_updates();
    let packets = player.take_packets();
    let updates = block_updates(&player, &packets);
    assert!(updates.contains(&(SPONGE, Block::WET_SPONGE.default_state.id)));
    assert!(updates.contains(&(water_east, Block::AIR.default_state.id)));
    assert!(updates.contains(&(water_southeast, Block::AIR.default_state.id)));
    assert_eq!(count_sound(&packets, Sound::BlockSpongeAbsorb), 1);

    let neighbor_callbacks = fixture.neighbor_callbacks.try_iter().collect::<Vec<_>>();
    assert_eq!(
        neighbor_callbacks
            .iter()
            .filter(|callback| callback.target == PROBE)
            .map(|callback| callback.source_block)
            .collect::<Vec<_>>(),
        [Block::WATER.id],
        "the drained water callback remains while the sponge transition callback is omitted"
    );
    let physics = physics.try_iter().collect::<Vec<_>>();
    assert_eq!(
        physics
            .iter()
            .filter(|dispatch| dispatch.target == PROBE)
            .copied()
            .collect::<Vec<_>>(),
        [PhysicsDispatch {
            target: PROBE,
            source: water_southeast,
        }],
        "the real dispatch retains its drained-water source position"
    );
    assert!(
        physics.iter().all(|dispatch| dispatch.source != SPONGE),
        "wet conversion must not dispatch ordinary neighbor callbacks from the sponge"
    );
    assert!(
        fixture
            .shape_callbacks
            .try_iter()
            .any(|callback| { callback.target == PROBE && callback.neighbor == SPONGE })
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g11_sponge_neighbor_activation_and_state_controls() {
    let water_east = SPONGE.offset((1, 0, 0).into());
    let water_west = SPONGE.offset((-1, 0, 0).into());
    let fixture = Fixture::new(&[
        (SPONGE.down(), Block::STONE.default_state.id),
        (PROBE, Block::STONE.default_state.id),
    ]);
    let mut player = TestPlayer::new(&fixture.world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 70.0, 12.5));
    player.take_packets();

    fixture.place_sponge_with_item(&player);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::SPONGE);
    fixture.world.flush_block_updates();
    let no_water_packets = player.take_packets();
    assert_eq!(count_sound(&no_water_packets, Sound::BlockSpongeAbsorb), 0);
    let _ = fixture.shape_callbacks.try_iter().count();

    fixture
        .world
        .set_block_state(&water_east, water(), BlockFlags::NOTIFY_ALL);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    assert_eq!(fixture.world.get_block(&water_east), &Block::AIR);
    fixture.world.flush_block_updates();
    let activation_packets = player.take_packets();
    let updates = block_updates(&player, &activation_packets);
    assert!(updates.contains(&(SPONGE, Block::WET_SPONGE.default_state.id)));
    assert!(updates.contains(&(water_east, Block::AIR.default_state.id)));
    assert_eq!(
        count_sound(&activation_packets, Sound::BlockSpongeAbsorb),
        1
    );
    assert!(
        fixture
            .shape_callbacks
            .try_iter()
            .any(|callback| { callback.target == PROBE && callback.neighbor == SPONGE })
    );

    fixture
        .world
        .set_block_state(&water_west, water(), BlockFlags::NOTIFY_ALL);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    assert_eq!(fixture.world.get_block(&water_west), &Block::WATER);
    fixture.world.flush_block_updates();
    let wet_control_packets = player.take_packets();
    assert_eq!(
        count_sound(&wet_control_packets, Sound::BlockSpongeAbsorb),
        0
    );
    fixture.finish().await;
}
