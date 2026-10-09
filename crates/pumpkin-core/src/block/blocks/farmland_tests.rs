use super::*;
use crate::world::spawn_test_support::{proto, publish};
use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::api::events::block::moisture_change::MoistureChangeEvent,
    plugin::{BoxFuture, EventHandler, EventPriority},
    server::Server,
};
use pumpkin_data::biome::Biome;
use pumpkin_util::math::vector2::Vector2;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

fn put(world: &Arc<World>, pos: &BlockPos, block: &Block) {
    world.set_block_state(pos, block.default_state.id, BlockFlags::FORCE_STATE);
}

fn tick(world: &Arc<World>, pos: &BlockPos) {
    FarmlandBlock.random_tick(RandomTickArgs {
        world,
        block: &Block::FARMLAND,
        position: pos,
    });
}

fn moisture(world: &Arc<World>, pos: &BlockPos) -> u8 {
    FarmlandProperties::from_state_id(world.get_block_state_id(pos)).moisture
}

fn add_chunk(world: &World) {
    publish(world, proto(&Biome::PLAINS, &Block::AIR));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rain_hydrates_exposed_farmland_but_not_roofed_or_snowy() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    world.set_raining(true);
    // Level.isRaining / ServerLevel.advanceWeatherCycle wait for visible rain (> 0.2).
    assert!(!world.is_raining());
    for _ in 0..21 {
        world.tick_environment();
    }
    assert!(world.is_raining());
    let cases = [
        (BlockPos::new(2, 64, 2), &Biome::PLAINS, false, 7),
        (BlockPos::new(10, 64, 2), &Biome::PLAINS, true, 0),
        (BlockPos::new(2, 64, 10), &Biome::SNOWY_PLAINS, false, 0),
    ];
    for (pos, biome, roof, expected) in cases {
        // Crops keep dry farmland intact, allowing the moisture to be inspected.
        put(&world, &pos, &Block::FARMLAND);
        put(&world, &pos.up(), &Block::WHEAT);
        if roof {
            put(&world, &pos.up().up(), &Block::STONE);
        }
        let above = pos.up();
        if let Some(chunk) = world.level.loaded_chunks.get(&Vector2::new(0, 0)) {
            chunk.section.set_relative_biome(
                (above.0.x / 4) as usize,
                ((above.0.y - world.dimension.min_y) / 4) as usize,
                (above.0.z / 4) as usize,
                biome.id,
            );
        }
        world.set_sky_light_level(&above, 15);
        assert_eq!(
            world.is_raining_at(&above),
            expected == 7,
            "{pos:?} roof={roof} biome={} height={}",
            world.get_biome(&above).registry_id,
            world.get_heightmap_height(
                pumpkin_world::chunk::ChunkHeightmapType::MotionBlocking,
                above.0.x,
                above.0.z
            )
        );
        tick(&world, &pos);
        assert_eq!(moisture(&world, &pos), expected);
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintains_farmland_tag_controls_survival() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let pos = BlockPos::new(2, 64, 2);
    for (block, expected) in [
        (&Block::WHEAT, true),
        (&Block::OAK_FENCE_GATE, true),
        (&Block::MOVING_PISTON, true),
        (&Block::STONE, false),
    ] {
        put(&world, &pos.up(), block);
        assert_eq!(
            can_place_at(world.as_ref(), &pos),
            expected,
            "{}",
            block.name
        );
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn farmland_survives_replacement_before_scheduled_tick() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let pos = BlockPos::new(2, 64, 2);
    put(&world, &pos, &Block::FARMLAND);
    world.set_block_state(
        &pos.up(),
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(world.is_block_tick_scheduled(&pos, &Block::FARMLAND));
    // The solid support scheduled a conversion, but was replaced before it ran.
    put(&world, &pos.up(), &Block::WHEAT);
    FarmlandBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::FARMLAND,
        position: &pos,
    });
    assert_eq!(world.get_block(&pos), &Block::FARMLAND);
    put(&world, &pos.up(), &Block::STONE);
    FarmlandBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::FARMLAND,
        position: &pos,
    });
    assert_eq!(world.get_block(&pos), &Block::DIRT);
    fixture.server.shutdown().await;
}

struct MoistureHandler {
    calls: Arc<AtomicU32>,
    cancelled: Arc<AtomicBool>,
}
impl EventHandler<MoistureChangeEvent> for MoistureHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut MoistureChangeEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Relaxed);
        event.cancelled = self.cancelled.load(Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fully_hydrated_farmland_skips_events_and_cancelled_updates_keep_moisture() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let pos = BlockPos::new(2, 64, 2);
    put(&world, &pos, &Block::FARMLAND);
    put(&world, &pos.up(), &Block::WHEAT);
    put(&world, &pos.offset(Vector3::new(1, 0, 0)), &Block::WATER);
    let calls = Arc::new(AtomicU32::new(0));
    let cancelled = Arc::new(AtomicBool::new(true));
    fixture
        .server
        .plugin_manager
        .register::<MoistureChangeEvent, _>(
            Arc::new(MoistureHandler {
                calls: calls.clone(),
                cancelled: cancelled.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    tick(&world, &pos);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(moisture(&world, &pos), 0);
    cancelled.store(false, Relaxed);
    tick(&world, &pos);
    assert_eq!(moisture(&world, &pos), 7);
    tick(&world, &pos);
    assert_eq!(calls.load(Relaxed), 2);
    fixture.server.shutdown().await;
}
