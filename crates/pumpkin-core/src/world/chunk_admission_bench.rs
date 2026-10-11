//! Fixed-seed hot-path scenes, with no listeners, generation workers or random AI.
use std::{sync::Arc, sync::atomic::Ordering, time::Instant};

use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{FacingHopper, HopperLikeProperties},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use rayon::prelude::*;

use super::{
    World,
    spawn_test_support::{proto, publish},
};
use crate::{
    block::entities::{BlockEntity, hopper::HopperBlockEntity},
    entity::EntityBase,
};

struct Scene {
    world: Arc<World>,
    mobs: Vec<Arc<dyn EntityBase>>,
    hoppers: Vec<Arc<HopperBlockEntity>>,
}

#[tokio::test]
#[ignore = "release unload latency benchmark; run alone with --nocapture"]
async fn preset_unload_latency_at_forty_five_mspt() {
    use pumpkin_world::{
        chunk::{ChunkData, io::Dirtiable},
        world::WorldPortalExt,
    };
    use std::time::Duration;
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();
    for round in 0..3 {
        for offset in 0..2 {
            let legacy = (round + offset) % 2 == 0;
            let (mut scene, _dir) = scene(0).await;
            scene.mobs.clear();
            let world = &scene.world;
            world.level.loaded_chunks.clear();
            let portal: Arc<dyn WorldPortalExt> = Arc::new(super::WorldPortal(world.clone()));
            world.level.world_portal.store(Arc::new(Some(portal)));
            for x in 0..512 {
                let pos = Vector2::new(x, 0);
                let chunk = ChunkData::empty_sync(x, 0);
                chunk.mark_dirty(true);
                world.level.loaded_chunks.insert(pos, chunk.clone());
                let entity_chunk = world.level.get_entity_chunk(pos).await.unwrap();
                world.make_chunk_entities_live(&entity_chunk, None);
                let mob = crate::entity::r#type::from_type(
                    &EntityType::PIG,
                    Vector3::new(f64::from(x * 16) + 1.5, 64.0, 1.5),
                    world,
                    uuid::Uuid::from_u128(x as u128 + 1),
                );
                assert!(world.spawn_entity(mob));
                world.queue_chunk_unload(&chunk);
            }
            world
                .level
                .chunk_lifecycles
                .benchmark_use_legacy_admission(legacy);
            let start = Instant::now();
            let mut samples = Vec::new();
            let mut drains = Vec::new();
            while !world.level.loaded_chunks.is_empty() {
                assert!(
                    start.elapsed() < Duration::from_secs(60),
                    "unload latency exceeded sixty seconds"
                );
                let tick_start = Instant::now();
                world.drain_chunk_unloads();
                drains.push(tick_start.elapsed().as_micros());
                {
                    let _chunks = world.hold_tick_chunks();
                    let work_start = Instant::now();
                    while work_start.elapsed() < Duration::from_millis(45) {
                        std::hint::spin_loop();
                    }
                }
                samples.push(tick_start.elapsed().as_micros());
                if let Some(remaining) = Duration::from_millis(50).checked_sub(tick_start.elapsed())
                {
                    tokio::time::sleep(remaining).await;
                }
            }
            samples.sort_unstable();
            drains.sort_unstable();
            tracing::info!(
                round,
                legacy,
                chunks = 512,
                latency_ms = start.elapsed().as_millis(),
                ticks = samples.len(),
                median_mspt_us = samples[samples.len() / 2],
                max_mspt_us = samples.last().unwrap(),
                median_drain_us = drains[drains.len() / 2],
                max_drain_us = drains.last().unwrap(),
                "unload benchmark"
            );
            assert!(world.entities.load().is_empty());
            world.level.world_portal.store(Arc::new(None));
            world.level.shutdown().await.unwrap();
        }
    }
}

async fn scene(kind: usize) -> (Scene, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let server = crate::server::combat_test_support::server(dir.path());
    let world = crate::server::combat_test_support::world(&server, dir.path());
    // Stop and join the empty scheduler before publishing the preset terrain.
    world
        .level
        .shut_down_chunk_system
        .store(true, Ordering::Release);
    let threads = std::mem::take(&mut *world.level.thread_tracker.lock().unwrap());
    for thread in threads {
        thread.join().unwrap();
    }
    world.level.chunk_system_tasks.close();
    world.level.chunk_system_tasks.wait().await;
    for x in 0..5 {
        let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
        terrain.x = x;
        terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
        publish(&world, terrain);
    }
    let mut result = Scene {
        world,
        mobs: Vec::new(),
        hoppers: Vec::new(),
    };
    if kind == 0 || kind == 3 {
        for index in 0..200 {
            let mob = crate::entity::r#type::from_type(
                &EntityType::PIG,
                Vector3::new(
                    if kind == 3 {
                        15.98
                    } else {
                        2.0 + f64::from(index % 10)
                    },
                    64.0,
                    2.0,
                ),
                &result.world,
                uuid::Uuid::from_u128(index as u128 + 1),
            );
            result.mobs.push(mob);
        }
    } else if kind == 1 {
        let mut props = HopperLikeProperties::default(&Block::HOPPER);
        props.facing = FacingHopper::East;
        for x in 1..=64 {
            let pos = BlockPos::new(x, 64, 2);
            result.world.set_block_state(
                &pos,
                props.to_state_id(&Block::HOPPER),
                BlockFlags::UPDATE_KNOWN_SHAPE,
            );
            let hopper = Arc::new(HopperBlockEntity::new(pos, FacingHopper::East));
            hopper.set_stack(0, ItemStack::new(32, &Item::COBBLESTONE));
            result.world.add_block_entity(hopper.clone());
            result.hoppers.push(hopper);
        }
    } else {
        for x in 1..=32 {
            result.world.set_block_state(
                &BlockPos::new(x, 64, 2),
                Block::REDSTONE_LAMP.default_state.id,
                BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }
    }
    (result, dir)
}

fn step(scene: &Scene, kind: usize, tick: u64) {
    let world = &scene.world;
    let _chunks = world.hold_tick_chunks();
    let _tick_admission = world.level.enter_tick_mutations();
    world.level_time.lock().unwrap().world_age = tick as i64;
    if kind == 0 || kind == 3 {
        scene.mobs.par_chunks(16).for_each(|batch| {
            let _tick_admission = world.level.enter_tick_mutations();
            for mob in batch {
                let entity = mob.get_entity();
                let motion =
                    Vector3::new(if tick.is_multiple_of(2) { 0.05 } else { -0.05 }, 0.0, 0.0);
                entity.set_velocity(motion);
                entity.set_rotation((tick % 360) as f32, 0.0);
                entity.move_entity(mob.as_ref(), motion);
            }
        });
    } else if kind == 1 {
        // A circulating chain keeps all 64 hoppers transferring throughout the sample.
        if tick.is_multiple_of(8) {
            let last = scene.hoppers.last().unwrap();
            let amount = last.get_stack(0).item_count;
            last.set_stack(0, ItemStack::new(32, &Item::COBBLESTONE));
            scene.hoppers[0].set_stack(0, ItemStack::new(amount, &Item::COBBLESTONE));
        }
        scene.hoppers.par_chunks(16).for_each(|batch| {
            let _tick_admission = world.level.enter_tick_mutations();
            for hopper in batch {
                hopper.tick(world);
            }
        });
    } else {
        // Fixed 16-tick square-wave clock, through real neighbor and scheduled lamp callbacks.
        if tick.is_multiple_of(8) {
            let state = if tick.is_multiple_of(16) {
                &Block::REDSTONE_BLOCK
            } else {
                &Block::AIR
            };
            for x in 1..=32 {
                world.set_block_state(
                    &BlockPos::new(x, 65, 2),
                    state.default_state.id,
                    BlockFlags::NOTIFY_NEIGHBORS | BlockFlags::UPDATE_KNOWN_SHAPE,
                );
            }
        }
        let active = (0..5).map(|x| Vector2::new(x, 0)).collect();
        let data = world.level.get_tick_data(&active, 0);
        for tick in data.block_ticks {
            let block = world.get_block(&tick.position);
            if let Some(behavior) = world.block_registry.get_pumpkin_block(block.id) {
                behavior.on_scheduled_tick(crate::block::OnScheduledTickArgs {
                    world,
                    block,
                    position: &tick.position,
                });
            }
        }
    }
    world.flush_block_updates();
    std::hint::black_box(world);
}

#[tokio::test]
#[ignore = "release timing benchmark; run alone with --nocapture --test-threads=1"]
async fn preset_chunk_admission_cost() {
    const WARMUP_TICKS: u64 = 200;
    const SAMPLE_TICKS: u64 = 320;
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();
    for threads in [2, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        for kind in 0..4 {
            let mut samples = [Vec::new(), Vec::new(), Vec::new()];
            for round in 0..15 {
                // Interleave all three modes and rotate their order to reduce time/order bias.
                for offset in 0..3 {
                    let mode = (round + offset) % 3;
                    let (scene, _dir) = scene(kind).await;
                    scene
                        .world
                        .level
                        .chunk_lifecycles
                        .benchmark_disable_admission(mode == 0);
                    scene
                        .world
                        .level
                        .chunk_lifecycles
                        .benchmark_use_legacy_admission(mode == 1);
                    pool.install(|| {
                        for tick in 0..WARMUP_TICKS {
                            step(&scene, kind, tick);
                        }
                    });
                    let start = Instant::now();
                    pool.install(|| {
                        for tick in WARMUP_TICKS..WARMUP_TICKS + SAMPLE_TICKS {
                            step(&scene, kind, tick);
                        }
                    });
                    samples[mode].push(start.elapsed().as_nanos() / u128::from(SAMPLE_TICKS));
                    scene.world.level.shutdown().await.unwrap();
                }
            }
            for (mode, mut samples) in samples.into_iter().enumerate() {
                samples.sort_unstable();
                tracing::info!(
                    kind,
                    threads,
                    mode,
                    median_ns_per_tick = samples[7],
                    min_ns_per_tick = samples[0],
                    ?samples,
                    "admission benchmark"
                );
            }
        }
    }
}
