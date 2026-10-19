#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use crate::{
    plugin::{BoxFuture, EventHandler, EventPriority, Payload},
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockStateId,
    biome::Biome,
    block_properties::{DoubleBlockHalf, TallSeagrassLikeProperties, WhiteWoolSlabLikeProperties},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

const SPONGE: BlockPos = BlockPos::new(8, 70, 8);

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    _chunk: Arc<ChunkData>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new(states: &[(BlockPos, BlockStateId)]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
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
            _chunk: chunk,
            _directory: directory,
        }
    }

    fn place_sponge(&self, position: BlockPos) {
        self.world.set_block_state(
            &position,
            Block::SPONGE.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
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

fn waterlogged_slab() -> BlockStateId {
    let mut properties = WhiteWoolSlabLikeProperties::default(&Block::OAK_SLAB);
    properties.waterlogged = true;
    properties.to_state_id(&Block::OAK_SLAB)
}

fn tall_seagrass(half: DoubleBlockHalf) -> BlockStateId {
    let mut properties =
        TallSeagrassLikeProperties::from_state_id(Block::TALL_SEAGRASS.default_state.id);
    properties.half = half;
    properties.to_state_id(&Block::TALL_SEAGRASS)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue80_absorbs_waterlogged_blocks_and_aquatic_plants() {
    let slab = BlockPos::new(8, 70, 9);
    let ordinary_water = BlockPos::new(9, 70, 8);
    let water_behind_slab = BlockPos::new(8, 70, 10);
    let seagrass = BlockPos::new(7, 70, 8);
    let kelp = BlockPos::new(8, 71, 8);
    let kelp_plant = BlockPos::new(8, 70, 7);
    let tall_lower = BlockPos::new(7, 71, 8);
    let tall_upper = tall_lower.up();
    let fixture = Fixture::new(&[
        (slab, waterlogged_slab()),
        (ordinary_water, water()),
        (water_behind_slab, water()),
        (seagrass, Block::SEAGRASS.default_state.id),
        (kelp, Block::KELP.default_state.id),
        (kelp_plant, Block::KELP_PLANT.default_state.id),
        (tall_lower, tall_seagrass(DoubleBlockHalf::Lower)),
        (tall_upper, tall_seagrass(DoubleBlockHalf::Upper)),
    ]);
    let kelp_drops = Arc::new(AtomicUsize::new(0));
    let seagrass_drops = Arc::new(AtomicUsize::new(0));
    let recorded_kelp_drops = kelp_drops.clone();
    let recorded_seagrass_drops = seagrass_drops.clone();
    fixture.register::<crate::plugin::api::events::entity::item_spawn::ItemSpawnEvent>(
        move |event| match event.item_name.as_str() {
            "kelp" => {
                recorded_kelp_drops.fetch_add(1, Relaxed);
            }
            "seagrass" => {
                recorded_seagrass_drops.fetch_add(1, Relaxed);
            }
            _ => {}
        },
    );

    fixture.place_sponge(SPONGE);

    assert_eq!(fixture.world.get_block(&ordinary_water), &Block::AIR);
    assert_eq!(fixture.world.get_block(&water_behind_slab), &Block::AIR);
    assert_eq!(fixture.world.get_block(&slab), &Block::OAK_SLAB);
    assert!(!fixture.world.get_block_state_id(&slab).is_waterlogged());
    for position in [seagrass, kelp, kelp_plant, tall_lower, tall_upper] {
        assert_eq!(
            fixture.world.get_block(&position),
            &Block::AIR,
            "{position:?}"
        );
    }
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    assert_eq!(
        kelp_drops.load(Relaxed),
        2,
        "kelp and its plant section both use their block loot tables"
    );
    assert_eq!(
        seagrass_drops.load(Relaxed),
        0,
        "seagrass without shears follows its ordinary no-drop loot table"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue80_neighbor_dispatch_absorbs_new_water() {
    let water_position = SPONGE.offset((1, 0, 0).into());
    let fixture = Fixture::new(&[]);
    fixture.place_sponge(SPONGE);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::SPONGE);

    // The real World::set_block_state path dispatches the neighbor update after placement.
    fixture
        .world
        .set_block_state(&water_position, water(), BlockFlags::NOTIFY_ALL);

    assert_eq!(fixture.world.get_block(&water_position), &Block::AIR);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue80_cancelled_absorb_event_preserves_water() {
    let water_position = SPONGE.offset((1, 0, 0).into());
    let fixture = Fixture::new(&[(water_position, water())]);
    let calls = Arc::new(AtomicUsize::new(0));
    let event_calls = calls.clone();
    fixture.register::<crate::plugin::api::events::block::sponge_absorb::SpongeAbsorbEvent>(
        move |event| {
            event_calls.fetch_add(1, Relaxed);
            event.cancelled = true;
        },
    );

    fixture.place_sponge(SPONGE);

    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(fixture.world.get_block(&water_position), &Block::WATER);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::SPONGE);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue80_event_replacement_survives_and_disconnects_stale_water() {
    let first = SPONGE.offset((1, 0, 0).into());
    let second = SPONGE.offset((2, 0, 0).into());
    let fixture = Fixture::new(&[(first, water()), (second, water())]);
    let world = fixture.world.clone();
    fixture.register::<crate::plugin::api::events::block::sponge_absorb::SpongeAbsorbEvent>(
        move |_| {
            world.set_block_state(
                &first,
                Block::STONE.default_state.id,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        },
    );

    fixture.place_sponge(SPONGE);

    assert_eq!(fixture.world.get_block(&first), &Block::STONE);
    assert_eq!(fixture.world.get_block(&second), &Block::WATER);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::SPONGE);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue81_absorbs_six_connected_blocks_only() {
    let water_positions = (1..=7)
        .map(|distance| BlockPos::new(SPONGE.0.x + distance, SPONGE.0.y, SPONGE.0.z))
        .collect::<Vec<_>>();
    let seeds = water_positions
        .iter()
        .map(|position| (*position, water()))
        .collect::<Vec<_>>();
    let fixture = Fixture::new(&seeds);

    fixture.place_sponge(SPONGE);

    for position in &water_positions[..6] {
        assert_eq!(
            fixture.world.get_block(position),
            &Block::AIR,
            "{position:?}"
        );
    }
    assert_eq!(fixture.world.get_block(&water_positions[6]), &Block::WATER);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue81_depth_uses_graph_distance_in_a_winding_corridor() {
    let start = BlockPos::new(8, 70, 8);
    let offsets = [
        (0, 0, 1),
        (1, 0, 1),
        (1, 0, 2),
        (1, 0, 3),
        (2, 0, 3),
        (2, -1, 3),
        (2, -1, 2),
    ];
    let positions = offsets
        .iter()
        .map(|(x, y, z)| BlockPos::new(start.0.x + x, start.0.y + y, start.0.z + z))
        .collect::<Vec<_>>();
    let seeds = positions
        .iter()
        .map(|position| (*position, water()))
        .collect::<Vec<_>>();
    let fixture = Fixture::new(&seeds);

    fixture.place_sponge(start);

    for position in &positions[..6] {
        assert_eq!(
            fixture.world.get_block(position),
            &Block::AIR,
            "{position:?}"
        );
    }
    assert_eq!(fixture.world.get_block(&positions[6]), &Block::WATER);
    let taxicab_distance = (positions[6].0.x - start.0.x).abs()
        + (positions[6].0.y - start.0.y).abs()
        + (positions[6].0.z - start.0.z).abs();
    assert_eq!(taxicab_distance, 5);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_issue81_caps_absorption_at_sixty_four_water_blocks() {
    let mut seeds = Vec::new();
    for x in -6i32..=6 {
        for y in -6i32..=6 {
            for z in -6i32..=6 {
                if (x.abs() + y.abs() + z.abs()) <= 6 && (x, y, z) != (0, 0, 0) {
                    seeds.push((
                        BlockPos::new(SPONGE.0.x + x, SPONGE.0.y + y, SPONGE.0.z + z),
                        water(),
                    ));
                }
            }
        }
    }
    let fixture = Fixture::new(&seeds);

    fixture.place_sponge(SPONGE);

    let removed = seeds
        .iter()
        .filter(|(position, _)| fixture.world.get_block(position) == &Block::AIR)
        .count();
    assert_eq!(removed, 64);
    assert_eq!(fixture.world.get_block(&SPONGE), &Block::WET_SPONGE);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g11_undrainable_or_dry_blocks_stop_traversal() {
    let dry_sponge = BlockPos::new(4, 70, 4);
    let stone_separator = dry_sponge.offset((1, 0, 0).into());
    let water_after_stone = dry_sponge.offset((2, 0, 0).into());
    let other_sponge = BlockPos::new(12, 70, 4);
    let bubble_separator = other_sponge.offset((1, 0, 0).into());
    let water_after_bubble = other_sponge.offset((2, 0, 0).into());
    let fixture = Fixture::new(&[
        (stone_separator, Block::STONE.default_state.id),
        (water_after_stone, water()),
        (bubble_separator, Block::BUBBLE_COLUMN.default_state.id),
        (water_after_bubble, water()),
    ]);

    fixture.place_sponge(dry_sponge);
    fixture.place_sponge(other_sponge);

    assert_eq!(fixture.world.get_block(&dry_sponge), &Block::SPONGE);
    assert_eq!(fixture.world.get_block(&water_after_stone), &Block::WATER);
    assert_eq!(fixture.world.get_block(&other_sponge), &Block::SPONGE);
    assert_eq!(
        fixture.world.get_block(&bubble_separator),
        &Block::BUBBLE_COLUMN
    );
    assert_eq!(fixture.world.get_block(&water_after_bubble), &Block::WATER);
    fixture.finish().await;
}
