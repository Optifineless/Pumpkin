#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Regression fixtures and subprocess failure diagnostics"
)]

use std::{
    process::{Command, Stdio},
    sync::{Arc, Barrier, Mutex},
    time::Duration,
};

use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{HorizontalAxis, NetherPortalLikeProperties},
    dimension::Dimension,
    entity::EntityType,
};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    world_seed::Seed,
};
use pumpkin_world::{
    chunk::io::FileIO,
    chunk_system::chunk_state::{Chunk, StagedChunkEnum},
    generation::{
        generator::{WorldGenerator, flat::FlatGenerator},
        proto_chunk::ProtoChunk,
    },
};
use tokio::sync::oneshot;

use crate::{
    entity::{Entity, EntityBase, death_test_world::DeathTestWorld, living::LivingEntity},
    world::World,
};

type Arrival = (Vector3<f64>, Arc<World>);

// Observe the existing EntityBase teleport boundary; lookup, tick and chunk I/O are real.
struct Traveler {
    entity: Entity,
    arrival: Mutex<Option<oneshot::Sender<Arrival>>>,
}

impl EntityBase for Traveler {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn teleport(
        &self,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        world: Arc<World>,
    ) {
        self.entity.teleport(position, yaw, pitch, &world);
        let arrival = self.arrival.lock().unwrap().take();
        if let Some(arrival) = arrival {
            let _ = arrival.send((position, world));
        }
    }
}

#[test]
fn unloaded_portal_destination_leaves_two_rayon_workers_available() {
    if let Ok(mode) = std::env::var("PUMPKIN_PORTAL_DEADLOCK_CHILD") {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(portal_travel(mode == "existing"));
        return;
    }
    for mode in ["existing", "create"] {
        run_subprocess(mode);
    }
}

fn run_subprocess(mode: &str) {
    let name = concat!(
        module_path!(),
        "::unloaded_portal_destination_leaves_two_rayon_workers_available"
    )
    .strip_prefix("pumpkin_core::")
    .unwrap();
    let log = tempfile::NamedTempFile::new().unwrap();
    let output = log.reopen().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("PUMPKIN_PORTAL_DEADLOCK_CHILD", mode)
        .env("RAYON_NUM_THREADS", "2")
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .spawn()
        .unwrap();
    let pid = child.id();
    let (finished, completion) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let _ = finished.send(child.wait());
    });
    // Only a crash guard: success is an explicit arrival from each real teleport call.
    let status = completion.recv_timeout(Duration::from_secs(60));
    if status.is_err() {
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap();
    }
    waiter.join().unwrap();
    let output = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        matches!(status, Ok(Ok(status)) if status.success()),
        "Portal {mode} regression failed (60 s crash guard if no exit status): {status:?}\n{output}"
    );
}

async fn portal_travel(existing: bool) {
    assert_eq!(rayon::current_num_threads(), 2);
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, existing).await;
    assert!(destination.level.loaded_chunks.is_empty());
    let mut arrivals = Vec::new();
    let mut travelers = Vec::new();
    for _ in 0..2 {
        let (arrival, receiver) = oneshot::channel();
        let traveler = Arc::new(Traveler {
            entity: Entity::new(
                source.clone(),
                Vector3::new(8.0, 80.0, 8.0),
                &EntityType::COW,
            ),
            arrival: Mutex::new(Some(arrival)),
        });
        source.add_entity_silent(traveler.clone());
        arrivals.push(receiver);
        travelers.push(traveler);
    }
    let entered = Arc::new(Barrier::new(3));
    let mut ticks = Vec::new();
    // Keep POI I/O from queuing a decoder before both simulation workers have queued
    // their local destination job. yield_local runs that exact local job on the old path.
    let poi = destination.portal_poi.lock().unwrap();
    for traveler in travelers {
        let server = fixture.server.clone();
        let destination = destination.clone();
        let entered = entered.clone();
        let (done, completion) = oneshot::channel();
        ticks.push(completion);
        rayon::spawn(move || {
            let _runtime = server.runtime.enter();
            traveler
                .entity
                .try_use_portal(destination, BlockPos::new(8, 80, 8));
            traveler.entity.tick(traveler.as_ref(), &server);
            entered.wait();
            rayon::yield_local();
            let _ = done.send(());
        });
    }
    // Both worker entry points meet this barrier before destination preparation proceeds.
    entered.wait();
    drop(poi);
    for tick in ticks {
        tick.await.unwrap();
    }
    for arrival in arrivals {
        let (position, arrived_world) = arrival.await.unwrap();
        assert!(Arc::ptr_eq(&arrived_world, &destination));
        let frame = BlockPos::new(
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        );
        assert_eq!(destination.get_block(&frame), &Block::NETHER_PORTAL);
    }
    fixture.server.shutdown().await;
}

pub(super) async fn save_destination(world: &World, existing: bool) {
    save_destination_with_block(world, existing, None).await;
}

pub(super) async fn save_destination_with_block(
    world: &World,
    existing: bool,
    block_entity: Option<(BlockPos, &Block, pumpkin_nbt::NbtCompound)>,
) {
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        world.dimension.clone(),
        Vec::new(),
        Biome::NETHER_WASTES.registry_id.to_owned(),
    )));
    let mut chunks = Vec::new();
    for x in -2..=1 {
        for z in -2..=1 {
            let mut proto = ProtoChunk::new(x, z, &generator);
            proto.stage = StagedChunkEnum::Full;
            if let Some((pos, block, _)) = &block_entity
                && pos.chunk_position() == Vector2::new(x, z)
            {
                let relative = pos.chunk_relative_position();
                proto.set_block_state(relative.x, pos.0.y, relative.z, block.default_state);
            }
            if existing && x == 0 && z == 0 {
                let portal = NetherPortalLikeProperties {
                    axis: HorizontalAxis::X,
                }
                .to_state_id(&Block::NETHER_PORTAL);
                for width in -1..=2 {
                    for height in -1..=3 {
                        let state = if width == -1 || width == 2 || height == -1 || height == 3 {
                            Block::OBSIDIAN.default_state
                        } else {
                            pumpkin_data::BlockState::from_id(portal)
                        };
                        proto.set_block_state(1 + width, 80 + height, 1, state);
                    }
                }
            }
            let mut chunk = Chunk::Proto(Box::new(proto));
            chunk.upgrade_to_level_chunk(
                &world.dimension,
                &pumpkin_config::lighting::LightingEngineConfig::default(),
            );
            let Chunk::Level(chunk) = chunk else {
                panic!("fixture must be a full chunk")
            };
            if let Some((pos, _, nbt)) = &block_entity
                && !nbt.is_empty()
                && pos.chunk_position() == Vector2::new(x, z)
            {
                chunk
                    .pending_block_entities
                    .lock()
                    .unwrap()
                    .insert(*pos, nbt.clone());
            }
            chunks.push((Vector2::new(x, z), chunk));
        }
    }
    world
        .level
        .chunk_saver
        .save_chunks(&world.level.level_folder, chunks)
        .await
        .unwrap();
    world
        .level
        .chunk_saver
        .block_and_await_ongoing_tasks()
        .await
        .unwrap();
    if existing {
        world
            .portal_poi
            .lock()
            .unwrap()
            .add_portal(BlockPos::new(1, 80, 1));
    }
}
