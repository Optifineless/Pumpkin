use std::{sync::Arc, time::Duration};

use pumpkin_data::{
    Block, biome::Biome, block_properties::HorizontalAxis, dimension::Dimension, entity::EntityType,
};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2},
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
    entity::{EntityBase, death_test_world::DeathTestWorld},
    world::World,
};

async fn save_empty_chunks(world: &World, positions: impl IntoIterator<Item = Vector2<i32>>) {
    save_chunks(world, positions, None).await;
}

async fn save_chunks(
    world: &World,
    positions: impl IntoIterator<Item = Vector2<i32>>,
    floor_roof: Option<(i32, i32)>,
) {
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        world.dimension.clone(),
        Vec::new(),
        Biome::THE_END.registry_id.to_owned(),
    )));
    let chunks = positions
        .into_iter()
        .map(|pos| {
            let mut proto = ProtoChunk::new(pos.x, pos.y, &generator);
            proto.stage = StagedChunkEnum::Full;
            if let Some(layers) = floor_roof {
                for x in 0..16 {
                    for z in 0..16 {
                        for y in <[i32; 2]>::from(layers) {
                            proto.set_block_state(
                                pos.x * 16 + x,
                                y,
                                pos.y * 16 + z,
                                Block::STONE.default_state,
                            );
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
                panic!("fixture must be full")
            };
            if world.dimension == Dimension::THE_END && pos.y < 0 {
                // A dependency SPAWN load cannot substitute for an explicit FULL platform ticket.
                Arc::get_mut(&mut chunk).unwrap().status = StagedChunkEnum::Spawn.into();
            }
            (pos, chunk)
        })
        .collect();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_portal_builds_entire_platform_from_unloaded_negative_z_chunk() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .create_world("portal-platform".to_owned(), Dimension::THE_END);
    save_empty_chunks(&destination, [Vector2::new(6, -1), Vector2::new(6, 0)]).await;
    assert!(destination.level.loaded_chunks.is_empty());
    let traveler = fixture.mob(&EntityType::COW);
    *traveler.get_entity().portal_manager.lock().unwrap() = Some(PortalProcessor::new(
        PortalType::End,
        BlockPos::new(0, 64, 0),
        destination.clone(),
    ));
    traveler
        .get_entity()
        .tick(traveler.as_ref(), &fixture.server);
    crate::entity::portal_travel::review_tests::finish_trip(traveler.get_entity()).await;
    assert!(Arc::ptr_eq(
        &traveler.get_entity().world.load_full(),
        &destination
    ));
    for x in 98..=102 {
        for z in -2..=2 {
            assert_eq!(
                destination.get_block(&BlockPos::new(x, 48, z)),
                &Block::OBSIDIAN
            );
        }
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dense_overworld_pois_load_only_nearest_frame_footprint() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    save_empty_chunks(
        &world,
        (-3..=5).flat_map(|x| (-3..=5).map(move |z| Vector2::new(x, z))),
    )
    .await;
    let mut setup = pumpkin_world::chunk_system::residency::ChunkResidency::new(
        world.level.chunk_loading.clone(),
    );
    setup.add(Vector2::new(0, 0));
    world
        .level
        .get_or_fetch_chunk(Vector2::new(0, 0), |_| ())
        .await
        .unwrap();
    NetherPortal::build_portal_frame(&world, BlockPos::new(1, 80, 1), HorizontalAxis::X, true);
    // A dense farm, including stale far POIs, must not load every candidate's neighborhood.
    {
        let mut poi = world.portal_poi.lock().unwrap();
        for x in (16..=64).step_by(2) {
            for z in (16..=64).step_by(2) {
                poi.add_portal(BlockPos::new(x, 80, z));
            }
        }
    }
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    *world.portal_scan_gate.lock().unwrap() = Some((ready, gate));
    let searching = world.clone();
    let search = tokio::spawn(async move {
        NetherPortal::search_for_portal(&searching, BlockPos::new(1, 80, 1)).await
    });
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    let loaded = world.level.chunk_loading.lock().unwrap().ticket.len();
    drop(setup);
    resume.send(()).unwrap();
    assert!(search.await.unwrap().is_some());
    fixture.server.shutdown().await;
    assert!(
        loaded <= 4,
        "nearest exit retained {loaded} full chunks for a dense POI farm"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_portal_trips_create_one_frame_on_safe_terrain() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture
        .server
        .create_world("portal-simultaneous".to_owned(), Dimension::THE_NETHER);
    save_chunks(
        &world,
        (-2..=1).flat_map(|x| (-2..=1).map(move |z| Vector2::new(x, z))),
        Some((79, 100)),
    )
    .await;
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    *world.portal_scan_gate.lock().unwrap() = Some((ready, gate));
    let first = fixture.mob(&EntityType::COW);
    let second = fixture.mob(&EntityType::COW);
    crate::entity::portal_travel::review_tests::tick_trip(&fixture, first.as_ref(), world.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    crate::entity::portal_travel::review_tests::tick_trip(&fixture, second.as_ref(), world.clone());
    assert!(second.get_entity().portal_travel_pending_for_test());
    resume.send(()).unwrap();
    crate::entity::portal_travel::review_tests::finish_trip(first.get_entity()).await;
    crate::entity::portal_travel::review_tests::finish_trip(second.get_entity()).await;
    let portals = world.portal_poi.lock().unwrap().get_in_square(
        BlockPos::new(1, 80, 1),
        32,
        Some(super::poi::POI_TYPE_NETHER_PORTAL),
    );
    assert_eq!(
        portals.len(),
        6,
        "simultaneous trips built separate portal frames"
    );
    assert_eq!(
        first.get_entity().pos.load(),
        second.get_entity().pos.load()
    );
    for traveler in [first, second] {
        assert!(Arc::ptr_eq(
            &traveler.get_entity().world.load_full(),
            &world
        ));
        assert!(
            world
                .get_entity_by_id(traveler.get_entity().entity_id)
                .is_some()
        );
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destination_unload_is_rejected_before_portal_chunks_are_requested() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture
        .server
        .create_world("portal-before-lookup".to_owned(), Dimension::THE_NETHER);
    save_empty_chunks(
        &world,
        (-2..=1).flat_map(|x| (-2..=1).map(move |z| Vector2::new(x, z))),
    )
    .await;
    let (ready, started) = tokio::sync::oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel();
    *world.portal_blocking_gate.lock().unwrap() = Some((ready, gate));
    let traveler = fixture.mob(&EntityType::COW);
    crate::entity::portal_travel::review_tests::tick_trip(
        &fixture,
        traveler.as_ref(),
        world.clone(),
    );
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    assert!(world.level.loaded_chunks.is_empty());
    let rejected = fixture
        .server
        .unload_world(world.get_world_name())
        .await
        .is_err();
    release.send(()).unwrap();
    assert!(
        rejected,
        "destination unloaded before portal lookup could finish"
    );
    crate::entity::portal_travel::review_tests::finish_trip(traveler.get_entity()).await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_portal_build_keeps_world_live_until_started_writes_finish() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture
        .server
        .create_world("portal-cancelled-build".to_owned(), Dimension::THE_NETHER);
    save_empty_chunks(
        &world,
        (-2..=1).flat_map(|x| (-2..=1).map(move |z| Vector2::new(x, z))),
    )
    .await;
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    *world.portal_scan_gate.lock().unwrap() = Some((ready, gate));
    let traveler = fixture.mob(&EntityType::COW);
    crate::entity::portal_travel::review_tests::tick_trip(
        &fixture,
        traveler.as_ref(),
        world.clone(),
    );
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    let (ready, started) = tokio::sync::oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel();
    *world.portal_blocking_gate.lock().unwrap() = Some((ready, gate));
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    traveler.get_entity().invalidate_portal_travel();
    assert!(
        fixture
            .server
            .unload_world(world.get_world_name())
            .await
            .is_err(),
        "unloaded a world during a started portal build"
    );
    assert!(!world.level.cancel_token.is_cancelled());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        while !world.level.chunk_loading.lock().unwrap().ticket.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture
        .server
        .unload_world(world.get_world_name())
        .await
        .unwrap();
    fixture.server.shutdown().await;
}
