use std::{
    process::{Command, Stdio},
    sync::{Arc, atomic::Ordering::Relaxed},
    time::{Duration, Instant},
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

use crate::{
    entity::{Entity, death_test_world::DeathTestWorld},
    world::{
        World,
        portal::{PortalProcessor, PortalType, SourcePortalInfo},
        spawn_test_support::{proto, publish},
    },
};

// A separate process isolates the two-worker global pool and lets a deadlock fail by timeout.
fn bounded_regression(name: &str, existing_portal: bool) {
    if std::env::var_os("PUMPKIN_PORTAL_DEADLOCK_CHILD").is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(portal_travel(existing_portal));
        return;
    }

    let log = tempfile::NamedTempFile::new().unwrap();
    let output = log.reopen().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("PUMPKIN_PORTAL_DEADLOCK_CHILD", "1")
        .env("RAYON_NUM_THREADS", "2")
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "Portal regression failed (outer timeout if no exit status): {status:?}\n{output}"
    );
    assert!(output.contains("test result: ok. 1 passed;"), "{output}");
}

#[test]
fn unloaded_existing_portal_does_not_block_world_tick() {
    bounded_regression(
        concat!(
            module_path!(),
            "::unloaded_existing_portal_does_not_block_world_tick"
        )
        .strip_prefix("pumpkin_core::")
        .unwrap(),
        true,
    );
}

#[test]
fn unloaded_new_portal_does_not_block_world_tick() {
    bounded_regression(
        concat!(
            module_path!(),
            "::unloaded_new_portal_does_not_block_world_tick"
        )
        .strip_prefix("pumpkin_core::")
        .unwrap(),
        false,
    );
}

async fn save_destination(world: &World, existing_portal: bool) {
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        world.dimension.clone(),
        Vec::new(),
        Biome::NETHER_WASTES.registry_id.to_owned(),
    )));
    let mut chunks = Vec::new();
    for x in -1..=1 {
        for z in -1..=1 {
            let mut proto = ProtoChunk::new(x, z, &generator);
            proto.stage = StagedChunkEnum::Full;
            if existing_portal && x == 0 && z == 0 {
                let portal = NetherPortalLikeProperties {
                    axis: HorizontalAxis::X,
                }
                .to_state_id(&Block::NETHER_PORTAL);
                // A real 2-by-3 portal in a saved chunk, with a POI available before chunk loading.
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
                panic!("fixture chunk must be upgraded")
            };
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
    if existing_portal {
        world
            .portal_poi
            .lock()
            .unwrap()
            .add_portal(BlockPos::new(1, 80, 1));
    }
    assert!(world.level.loaded_chunks.is_empty());
}

async fn portal_travel(existing_portal: bool) {
    assert_eq!(rayon::current_num_threads(), 2);
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, existing_portal).await;
    publish(&source, proto(&Biome::PLAINS, &Block::STONE));
    source
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));

    let mut travelers = Vec::new();
    // Two batches make World.tick use both global workers, as in the production backtrace.
    for index in 0..32 {
        let entity = Arc::new(Entity::new(
            source.clone(),
            Vector3::new(8.5, 80.0, 9.0),
            &EntityType::COW,
        ));
        entity.yaw.store(30.0);
        if index == 0 || index == 16 {
            let mut processor = PortalProcessor::new(
                PortalType::Nether,
                BlockPos::new(8, 80, 8),
                destination.clone(),
            );
            processor.set_source_portal(SourcePortalInfo {
                lower_corner: BlockPos::new(8, 80, 8),
                axis: HorizontalAxis::Z,
                width: 2,
                height: 3,
            });
            *entity.portal_manager.lock().unwrap() = Some(processor);
            travelers.push(entity.clone());
        }
        source.add_entity_silent(entity);
    }
    let passenger = Arc::new(Entity::new(
        source.clone(),
        Vector3::new(8.5, 80.0, 9.0),
        &EntityType::COW,
    ));
    passenger.yaw.store(15.0);
    travelers[0].add_passenger(travelers[0].clone(), passenger.clone());

    let (tick_tx, tick_rx) = tokio::sync::oneshot::channel();
    let server = fixture.server.clone();
    rayon::spawn(move || {
        source.tick(&server);
        let _ = tick_tx.send(());
    });
    let expected = if existing_portal {
        Vector3::new(2.0, 80.0, 1.5)
    } else {
        Vector3::new(1.5, 80.0, 1.0)
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        tick_rx.await.unwrap();
        while travelers.iter().any(|entity| entity.pos.load() != expected)
            || passenger.pos.load() != expected
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("world tick and portal teleport timed out");

    for entity in travelers {
        assert_eq!(
            entity.yaw.load(),
            if existing_portal { -60.0 } else { 30.0 }
        );
        assert_eq!(
            entity.portal_cooldown.load(Relaxed),
            entity.default_portal_cooldown()
        );
    }
    assert_eq!(
        passenger.yaw.load(),
        if existing_portal { -75.0 } else { 15.0 }
    );
    assert_eq!(
        passenger.portal_cooldown.load(Relaxed),
        passenger.default_portal_cooldown()
    );
    let frame = if existing_portal {
        BlockPos::new(1, 80, 1)
    } else {
        BlockPos::new(1, 80, 0)
    };
    assert_eq!(destination.get_block(&frame), &Block::NETHER_PORTAL);
    assert_eq!(destination.get_block(&frame.down()), &Block::OBSIDIAN);
    fixture.server.shutdown().await;
}
