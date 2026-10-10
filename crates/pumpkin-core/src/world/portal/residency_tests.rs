use std::{sync::Arc, time::Duration};

use pumpkin_data::{
    Block, biome::Biome, block_properties::HorizontalAxis, chunk::ChunkStatus,
    dimension::Dimension, entity::EntityType,
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

use super::{NetherPortal, PortalProcessor, PortalType};
use crate::{
    entity::{Entity, EntityBase, death_test_world::DeathTestWorld},
    world::World,
};

async fn save_destination(world: &Arc<World>, safe_terrain: bool) {
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        world.dimension.clone(),
        Vec::new(),
        Biome::NETHER_WASTES.registry_id.to_owned(),
    )));
    let mut chunks = Vec::new();
    for x in -2..=2 {
        for z in -2..=2 {
            let mut proto = ProtoChunk::new(x, z, &generator);
            proto.stage = StagedChunkEnum::Full;
            if safe_terrain {
                // The only four-block-wide floor crosses into chunk X=2, outside old 3x3.
                // A cave roof makes the scan descend through the air above the floor.
                for block_x in 30..=33 {
                    for block_z in 0..=2 {
                        if block_x >> 4 == x && block_z >> 4 == z {
                            for y in [79, 100] {
                                proto.set_block_state(
                                    block_x,
                                    y,
                                    block_z,
                                    Block::STONE.default_state,
                                );
                            }
                        }
                    }
                }
            }
            let mut chunk = Chunk::Proto(Box::new(proto));
            chunk.upgrade_to_level_chunk(
                &world.dimension,
                &pumpkin_config::lighting::LightingEngineConfig::default(),
            );
            let Chunk::Level(mut chunk) = chunk else {
                panic!("fixture chunk must be upgraded")
            };
            if safe_terrain && x == 2 {
                // A dependency load may publish a saved FULL chunk incidentally. A saved
                // SPAWN chunk becomes readable only when the scan explicitly requests FULL.
                Arc::get_mut(&mut chunk).unwrap().status = ChunkStatus::Spawn;
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
}

fn enter_portal(fixture: &DeathTestWorld, target: BlockPos) -> Arc<Entity> {
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    let scale = destination.dimension.coordinate_scale / source.dimension.coordinate_scale;
    let entity = Arc::new(Entity::new(
        source.clone(),
        Vector3::new(
            f64::from(target.0.x).mul_add(scale, scale / 2.0),
            f64::from(target.0.y),
            f64::from(target.0.z).mul_add(scale, scale / 2.0),
        ),
        &EntityType::COW,
    ));
    *entity.portal_manager.lock().unwrap() = Some(PortalProcessor::new(
        PortalType::Nether,
        BlockPos::new(0, 80, 0),
        destination,
    ));
    source.add_entity_silent(entity.clone());
    // Drive the real entity tick, including portal processing and the async arrival.
    entity.tick(entity.as_ref(), &fixture.server);
    entity
}

async fn await_arrival(entity: &Entity) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while entity
            .portal_cooldown
            .load(std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_creation_loads_safe_frame_beyond_old_three_by_three() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, true).await;
    let entity = enter_portal(&fixture, BlockPos::new(15, 80, 1));
    await_arrival(&entity).await;
    assert_eq!(entity.pos.load(), Vector3::new(32.0, 80.0, 1.5));
    // A fallback also replaces the floor beside the frame with obsidian.
    assert_eq!(
        destination.get_block(&BlockPos::new(31, 79, 0)),
        &Block::STONE
    );
    assert_eq!(
        destination.get_block(&BlockPos::new(32, 80, 1)),
        &Block::NETHER_PORTAL
    );
    assert_eq!(
        destination.get_block(&BlockPos::new(33, 80, 1)),
        &Block::OBSIDIAN
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_search_keeps_existing_portal_resident_until_scan_finishes() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, false).await;
    destination
        .level
        .get_or_fetch_chunk(Vector2::new(0, 0), |_| ())
        .await
        .unwrap();
    NetherPortal::build_portal_frame(
        &destination,
        BlockPos::new(1, 80, 1),
        HorizontalAxis::X,
        true,
    );
    // Persist the real frame before unloading; the normal chunk load path must find it again.
    let saved_chunk = destination
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap()
        .clone();
    destination
        .level
        .chunk_saver
        .save_chunks(
            &destination.level.level_folder,
            vec![(Vector2::new(0, 0), saved_chunk)],
        )
        .await
        .unwrap();
    destination
        .level
        .chunk_saver
        .block_and_await_ongoing_tasks()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while destination.level.is_chunk_loaded(&Vector2::new(0, 0)) {
            destination
                .level
                .should_unload
                .store(true, std::sync::atomic::Ordering::Relaxed);
            destination.level.level_channel.notify();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    *destination.portal_scan_gate.lock().unwrap() = Some((ready_tx, resume_rx));
    let entity = enter_portal(&fixture, BlockPos::new(1, 80, 1));
    tokio::time::timeout(Duration::from_secs(20), ready_rx)
        .await
        .unwrap()
        .unwrap();
    // Allow the scheduler to consume ticket removals, then force its actual unload pass.
    tokio::time::sleep(Duration::from_millis(100)).await;
    destination
        .level
        .should_unload
        .store(true, std::sync::atomic::Ordering::Relaxed);
    destination.level.level_channel.notify();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let unload_pass_ran = !destination
        .level
        .should_unload
        .load(std::sync::atomic::Ordering::Relaxed);
    let remained_loaded = destination.level.is_chunk_loaded(&Vector2::new(0, 0));
    resume_tx.send(()).unwrap();
    await_arrival(&entity).await;
    let actual = entity.pos.load();
    fixture.server.shutdown().await;
    assert!(
        unload_pass_ran,
        "scheduler did not run the requested unload pass"
    );
    assert!(
        remained_loaded,
        "existing portal unloaded between fetch and scan"
    );
    assert_eq!(
        actual,
        Vector3::new(2.0, 80.0, 1.5),
        "created a duplicate portal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_creation_releases_residency_when_cancelled_before_scan() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, false).await;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (_resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    *destination.portal_scan_gate.lock().unwrap() = Some((ready_tx, resume_rx));
    let world = destination.clone();
    let task = tokio::spawn(async move {
        PortalType::create_nether_portal(&world, BlockPos::new(1, 80, 1), HorizontalAxis::X).await
    });
    tokio::time::timeout(Duration::from_secs(20), ready_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !destination
            .level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .is_empty()
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        destination
            .level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .is_empty()
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_creation_retains_residency_until_cancelled_blocking_job_finishes() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, false).await;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    *destination.portal_scan_gate.lock().unwrap() = Some((ready_tx, resume_rx));
    let world = destination.clone();
    let task = tokio::spawn(async move {
        PortalType::create_nether_portal(&world, BlockPos::new(1, 80, 1), HorizontalAxis::X).await
    });
    tokio::time::timeout(Duration::from_secs(20), ready_rx)
        .await
        .unwrap()
        .unwrap();
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let world = destination.clone();
    let blocker = std::thread::spawn(move || {
        let _border = world.worldborder.lock().unwrap();
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(2));
    });
    locked_rx.await.unwrap();
    resume_tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let held = !destination
        .level
        .chunk_loading
        .lock()
        .unwrap()
        .ticket
        .is_empty();
    release_tx.send(()).unwrap();
    blocker.join().unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while !destination
            .level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let frame = destination.get_block(&BlockPos::new(0, 80, 1));
    fixture.server.shutdown().await;
    assert!(
        held,
        "cancelling the caller released a still-running scan's terrain"
    );
    assert_eq!(frame, &Block::NETHER_PORTAL);
}
