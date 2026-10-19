use super::*;
use crate::{
    entity::EntityBase, net::java::combat_test_support::TestPlayer, server::combat_test_support,
    world::World,
};
use pumpkin_data::{Block, dimension::Dimension};
use pumpkin_util::{math::vector3::Vector3, world_seed::Seed};
use pumpkin_world::{
    ProtoChunk,
    generation::{
        generator::{VanillaGenerator, WorldGenerator},
        structure::structures::{StructurePosition, create_chunk_random},
    },
    world_info::BiomeSource,
};
use std::{sync::Arc, time::Duration};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const CITY_CHUNK: (i32, i32) = (25, 123);

fn noise(world: &WorldGenerator) -> &VanillaGenerator {
    let WorldGenerator::Noise(generator) = world else {
        panic!("noise generator fixture")
    };
    generator
}

fn cached(
    world: &WorldGenerator,
    key: StructureKeys,
    chunk: (i32, i32),
) -> Option<StructurePosition> {
    noise(world)
        .global_structure_cache
        .get_or_compute_structure_start(key, chunk.0, chunk.1, || {
            panic!("command or generation must initialize the production cache")
        })
}

fn generate(world: &WorldGenerator, chunk: (i32, i32)) -> ProtoChunk {
    let mut generated = ProtoChunk::new(chunk.0, chunk.1, world);
    generated.step_to_biomes(noise(world));
    generated.set_structure_starts(noise(world));
    generated.set_structure_references(noise(world));
    generated
}

fn piece_bounds(position: &StructurePosition) -> Vec<pumpkin_util::math::block_box::BlockBox> {
    let collector = position.collector.lock().unwrap();
    assert!(!collector.pieces.is_empty());
    collector
        .pieces
        .iter()
        .map(|piece| piece.bounding_box())
        .collect()
}

fn bounded_dispatcher() -> CommandDispatcher {
    // Same registered parser and production executor; radius zero visits exactly one region.
    let mut dispatcher = CommandDispatcher::new();
    dispatcher.register(
        command("locate", DESCRIPTION).requires(PERMISSION).then(
            literal("structure").then(
                argument(
                    ARG_STRUCTURE,
                    ResourceOrTagKeyArgument(STRUCTURE_REGISTRY.clone()),
                )
                .executes(LocateStructureExecutor { search_radius: 0 }),
            ),
        ),
    );
    dispatcher
}

fn command_world(
    server: &Arc<crate::server::Server>,
    path: &std::path::Path,
    dimension: Dimension,
    seed: u64,
    fixed_biome: Option<&str>,
    set: &str,
) -> Arc<World> {
    let level = pumpkin_world::level::Level::from_root_folder(
        &pumpkin_config::world::LevelConfig::default(),
        path.to_path_buf(),
        seed as i64,
        dimension.clone(),
    );
    let source = fixed_biome.map(|biome| BiomeSource::Fixed {
        biome: format!("minecraft:{biome}"),
        biome_type: "minecraft:fixed".to_owned(),
    });
    level.set_world_gen(Arc::new(WorldGenerator::Noise(Box::new(
        VanillaGenerator::new_with_all_settings(
            Seed(seed),
            dimension.clone(),
            None,
            source.as_ref(),
            Some(&[set.to_owned()]),
        ),
    ))));
    let world = Arc::new(World::load(
        level,
        server.level_info.clone(),
        dimension,
        server.block_registry.clone(),
        Arc::downgrade(server),
    ));
    server.worlds.store(Arc::new(vec![world.clone()]));
    world
}

fn place_city(
    generated: &mut ProtoChunk,
    generator: &WorldGenerator,
    world: &Arc<World>,
    position: &StructurePosition,
    seed: u64,
    chunk: (i32, i32),
) -> Vec<pumpkin_data::BlockStateId> {
    assert_eq!(position.start_pos, BlockPos::new(407, 60, 1975));
    assert_eq!(piece_bounds(position).len(), 68);
    generated.step_to_noise(noise(generator));
    position.collector.lock().unwrap().generate_in_chunk(
        generated,
        &crate::world::WorldPortal(world.clone()),
        &mut create_chunk_random(seed as i64, chunk.0, chunk.1),
        seed as i64,
    );
    let mut blocks = Vec::new();
    for x in 0..16 {
        for z in 0..16 {
            for y in 60..100 {
                blocks.push(generated.get_block_state_raw(x, y, z));
            }
        }
    }
    assert!(
        blocks
            .iter()
            .any(|state| Block::from_state_id(*state) == &Block::PURPUR_BLOCK),
        "located city must produce actual purpur blocks"
    );
    blocks
}

async fn command_order(
    dimension: Dimension,
    seed: u64,
    fixed_biome: Option<&str>,
    set: &str,
    key: StructureKeys,
    chunk: (i32, i32),
) -> TestResult {
    let mut expected = None;
    for locate_first in [false, true] {
        // Separate real worlds and generators give both orders an empty production cache.
        let directory = tempfile::tempdir()?;
        let server = combat_test_support::server(directory.path());
        let world = command_world(
            &server,
            directory.path(),
            dimension.clone(),
            seed,
            fixed_biome,
            set,
        );
        let player = TestPlayer::new(&world);
        player.player.permission_lvl.store(PermissionLvl::Four);
        player.player.get_entity().set_pos(Vector3::new(
            f64::from(chunk.0 * 16),
            80.0,
            f64::from(chunk.1 * 16),
        ));
        let source = player.player.get_command_source(&server);
        let command = format!("locate structure minecraft:{}", key.to_name());
        let generator = world.level.world_gen();
        let generated = (!locate_first).then(|| generate(&generator, chunk));
        let result = bounded_dispatcher().execute_input(&command, &source);
        if result.is_err() {
            tokio::time::timeout(Duration::from_secs(20), world.level.shutdown()).await??;
        }
        assert_eq!(
            result.map_err(|error| format!("locate_first={locate_first}, {key:?}: {error:?}"))?,
            0
        );
        let located = cached(&generator, key, chunk).expect("command must cache a valid start");
        let bounds = piece_bounds(&located);
        let mut generated = generated.unwrap_or_else(|| generate(&generator, chunk));
        let after_generation = cached(&generator, key, chunk).unwrap();
        assert!(Arc::ptr_eq(&located.collector, &after_generation.collector));
        let blocks = if key == StructureKeys::EndCity {
            place_city(
                &mut generated,
                &generator,
                &world,
                &after_generation,
                seed,
                chunk,
            )
        } else {
            Vec::new()
        };
        let actual = (located.start_pos, bounds, blocks);
        if let Some(expected) = &expected {
            assert_eq!(
                &actual, expected,
                "command/generation order must preserve start, pieces and placed blocks"
            );
        } else {
            expected = Some(actual);
        }
        tokio::time::timeout(Duration::from_secs(20), world.level.shutdown()).await??;
    }
    Ok(())
}

#[tokio::test]
async fn b1019_g23_end_command_order_and_template_placement() -> TestResult {
    command_order(
        Dimension::THE_END,
        12_345,
        None,
        "end_cities",
        StructureKeys::EndCity,
        CITY_CHUNK,
    )
    .await
}

#[tokio::test]
async fn b1019_g23_fixed_biome_command_order() -> TestResult {
    command_order(
        Dimension::OVERWORLD,
        0,
        Some("swamp"),
        "swamp_huts",
        StructureKeys::SwampHut,
        (0, 18),
    )
    .await
}

#[tokio::test]
async fn b1019_g23_overworld_command_refuses_end_city() -> TestResult {
    let directory = tempfile::tempdir()?;
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Four);
    player.player.get_entity().set_pos(Vector3::new(
        f64::from(CITY_CHUNK.0 * 16),
        80.0,
        f64::from(CITY_CHUNK.1 * 16),
    ));
    let source = player.player.get_command_source(&server);
    let result = bounded_dispatcher().execute_input("locate structure minecraft:end_city", &source);
    assert!(
        result
            .expect_err("Overworld must not locate an End city")
            .is(&STRUCTURE_NOT_FOUND_ERROR_TYPE)
    );
    let generator = world.level.world_gen.load_full();
    let set = &StructureSet::END_CITIES;
    let StructurePlacementType::RandomSpread(spread) = &set.placement.placement_type else {
        panic!("End cities use random-spread placement")
    };
    let candidate = pumpkin_world::generation::structure::placement::get_structure_chunk_in_region(
        spread,
        world.level.seed.0 as i64,
        CITY_CHUNK.0.div_euclid(spread.spacing),
        CITY_CHUNK.1.div_euclid(spread.spacing),
        set.placement.salt,
    );
    assert!(cached(&generator, StructureKeys::EndCity, candidate).is_none());
    tokio::time::timeout(Duration::from_secs(20), world.level.shutdown()).await??;
    Ok(())
}
