use super::{
    generator::{VanillaGenerator, WorldGenerator},
    get_world_gen,
    noise::router::surface_height_sampler::{
        SurfaceHeightEstimateSampler, SurfaceHeightSamplerBuilderOptions,
    },
    surface::{MaterialRuleContext, estimate_surface_height},
};
use crate::{
    ProtoChunk,
    chunk::palette::{BiomePalette, NetworkPalette},
    chunk_system::{Chunk, generation_cache::SurfaceBiomeNeighborhood},
};
use pumpkin_config::lighting::LightingEngineConfig;
use pumpkin_data::{chunk::Biome, dimension::Dimension};
use pumpkin_util::{math::vector3::Vector3, world_seed::Seed};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    seed: u64,
    village_bounds: [i32; 4],
    biome_names: Vec<String>,
    chunks: Vec<FixtureChunk>,
}

#[derive(Deserialize)]
struct FixtureChunk {
    x: i32,
    z: i32,
    // x-major, then z: preliminary surface, top solid Y, top and second state IDs.
    columns: Vec<[i32; 4]>,
    // Anvil section order, then y/z/x within each 4x4x4 biome section.
    biomes: Vec<u8>,
}

fn fixture() -> Result<Fixture, serde_json::Error> {
    serde_json::from_str(include_str!(
        "../../../../assets/tests/vanilla_village_surface_1790825110648364942.json"
    ))
}

fn neighborhood(world: &WorldGenerator, x: i32, z: i32) -> SurfaceBiomeNeighborhood {
    let WorldGenerator::Noise(generator) = world else {
        unreachable!()
    };
    let mut neighbors = SurfaceBiomeNeighborhood::new(x, z);
    for cx in x - 1..=x + 1 {
        for cz in z - 1..=z + 1 {
            let mut chunk = ProtoChunk::new(cx, cz, world);
            chunk.step_to_biomes(generator);
            assert!(neighbors.push_chunk(&Chunk::Proto(Box::new(chunk))));
        }
    }
    neighbors
}

#[test]
fn vanilla_village_preliminary_surface_columns() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let world = get_world_gen(
        Seed(fixture.seed),
        Dimension::OVERWORLD,
        false,
        vec![],
        String::new(),
    );
    let WorldGenerator::Noise(generator) = &*world else {
        unreachable!()
    };
    let shape = generator.settings.shape;
    let options = SurfaceHeightSamplerBuilderOptions::new(
        i32::from(shape.min_y),
        i32::from(shape.max_y()),
        shape.vertical_cell_block_count() as usize,
    );
    let mut sampler =
        SurfaceHeightEstimateSampler::generate(&generator.base_router.surface_estimator, &options);
    let terrain = &generator.terrain_cache;
    let mut context = MaterialRuleContext::new(
        shape.min_y,
        shape.height,
        &generator.random_config.base_random_deriver,
        generator.random_config.legacy_random_source,
        generator.random_config.seed,
        &terrain.terrain_builder,
        &terrain.surface_noise,
        &terrain.secondary_noise,
        generator.settings.sea_level,
    );
    for chunk in fixture.chunks {
        assert_eq!(chunk.columns.len(), 256);
        for (index, column) in chunk.columns.iter().enumerate() {
            let x = chunk.x * 16 + (index / 16) as i32;
            let z = chunk.z * 16 + (index % 16) as i32;
            context.init_horizontal(x, z);
            assert_eq!(
                estimate_surface_height(&mut context, &mut sampler),
                column[0] + context.run_depth - 8,
                "vanilla MaterialRuleContext.getMinSurfaceLevel at ({x}, {z})"
            );
        }
    }
    Ok(())
}

fn surface_chunk(world: &WorldGenerator, expected: &FixtureChunk) -> ProtoChunk {
    let WorldGenerator::Noise(generator) = world else {
        unreachable!()
    };
    let mut chunk = ProtoChunk::new(expected.x, expected.z, world);
    chunk.step_to_biomes(generator);
    chunk.set_structure_starts(generator);
    chunk.set_structure_references(generator);
    chunk.step_to_noise(generator);
    chunk.step_to_surface(generator, &neighborhood(world, expected.x, expected.z));
    chunk
}

#[test]
fn village_start_outside_source_chunk_uses_biome_source() -> Result<(), Box<dyn std::error::Error>>
{
    use super::noise::router::multi_noise_sampler::MultiNoiseSampler;
    use super::structure::{
        generate_structure_position,
        height_sampler::NoiseHeightSampler,
        structures::{StructureGeneratorContext, create_chunk_random},
    };
    use crate::biome::BiomeSupplier;
    use pumpkin_data::structures::{Structure, StructureKeys};

    let fixture = fixture()?;
    let world = get_world_gen(
        Seed(fixture.seed),
        Dimension::OVERWORLD,
        false,
        vec![],
        String::new(),
    );
    let WorldGenerator::Noise(generator) = &*world else {
        unreachable!()
    };
    let mut chunk = ProtoChunk::new(-20, 15, &world);
    chunk.step_to_biomes(generator);
    let mut height_sampler = NoiseHeightSampler::new(generator);
    let key = StructureKeys::VillagePlains;
    let pos = generate_structure_position(
        &key,
        Structure::get(&key),
        StructureGeneratorContext {
            seed: fixture.seed as i64,
            chunk_x: chunk.x,
            chunk_z: chunk.z,
            random: create_chunk_random(fixture.seed as i64, chunk.x, chunk.z),
            sea_level: generator.settings.sea_level,
            min_y: i32::from(chunk.bottom_y()),
            height: chunk.height(),
            height_sampler: Some(&mut height_sampler),
            structure_key: Some(key),
        },
    )
    .ok_or("missing generation position")?;
    let pos = pos.start_pos.0;
    let mut noise = MultiNoiseSampler::generate(&generator.base_router.multi_noise);
    let source_biome =
        generator
            .biome_supplier
            .biome(pos.x >> 2, pos.y >> 2, pos.z >> 2, &mut noise);
    let stored_biome = chunk.get_biome_id(pos.x >> 2, pos.y >> 2, pos.z >> 2);
    chunk.set_structure_starts(generator);
    assert!(
        chunk.has_structure(key),
        "stub {pos:?}, source biome {}, wrapped biome {:?}",
        source_biome.registry_id,
        Biome::from_id(stored_biome).map(|b| b.registry_id)
    );
    Ok(())
}

#[test]
fn vanilla_bearded_surface_top_two_layers() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let world = get_world_gen(
        Seed(fixture.seed),
        Dimension::OVERWORLD,
        false,
        vec![],
        String::new(),
    );
    let mut counts = BTreeMap::new();
    let mut footprint_counts = BTreeMap::new();
    let mut mismatches = Vec::new();
    for expected in fixture.chunks {
        let chunk = surface_chunk(&world, &expected);
        let mut chunk_counts = BTreeMap::new();
        for (index, &[_, y, top, below]) in expected.columns.iter().enumerate() {
            let x = expected.x * 16 + (index / 16) as i32;
            let z = expected.z * 16 + (index % 16) as i32;
            let mut actual_y = chunk.top_block_height_exclusive(x, z) - 1;
            while actual_y > i32::from(chunk.bottom_y()) {
                let state = chunk
                    .get_block_state(&Vector3::new(x, actual_y, z))
                    .to_state();
                if !state.is_air() && !state.is_liquid() {
                    break;
                }
                actual_y -= 1;
            }
            let actual = [
                actual_y,
                i32::from(
                    chunk
                        .get_block_state(&Vector3::new(x, actual_y, z))
                        .as_u16(),
                ),
                i32::from(
                    chunk
                        .get_block_state(&Vector3::new(x, actual_y - 1, z))
                        .as_u16(),
                ),
            ];
            *chunk_counts.entry(actual[1]).or_insert(0) += 1;
            let [min_x, min_z, max_x, max_z] = fixture.village_bounds;
            if (min_x..=max_x).contains(&x) && (min_z..=max_z).contains(&z) {
                *footprint_counts.entry(actual[1]).or_insert(0) += 1;
            }
            if actual != [y, top, below] {
                mismatches.push((x, z, actual, [y, top, below]));
            }
        }
        counts.insert((expected.x, expected.z), chunk_counts);
    }
    assert!(
        mismatches.is_empty(),
        "{} differing columns; first differences: {:?}; footprint counts: {footprint_counts:?}; top state counts: {counts:?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(20)]
    );
    Ok(())
}

fn network_biome_at(palette: &BiomePalette, index: usize) -> u8 {
    let network = palette.convert_network();
    let bits = usize::from(network.bits_per_entry);
    let value = if let Some(per_word) = 64usize.checked_div(bits) {
        ((network.packed_data[index / per_word] as u64 >> ((index % per_word) * bits))
            & ((1 << bits) - 1)) as usize
    } else {
        0
    };
    match network.palette {
        NetworkPalette::Single(id) => id,
        NetworkPalette::Indirect(ids) => ids[value],
        NetworkPalette::Direct => value as u8,
    }
}

fn check_biomes(generator: &VanillaGenerator, world: &WorldGenerator, fixture: &Fixture) {
    let mut mismatches = BTreeMap::new();
    for expected in &fixture.chunks {
        let mut chunk = ProtoChunk::new(expected.x, expected.z, world);
        chunk.step_to_biomes(generator);
        assert_eq!(expected.biomes.len(), usize::from(chunk.height()) * 4);
        let mut chunk = Chunk::Proto(Box::new(chunk));
        chunk.upgrade_to_level_chunk(&Dimension::OVERWORLD, &LightingEngineConfig::Default);
        let Chunk::Level(chunk) = chunk else {
            unreachable!()
        };
        let palettes = chunk
            .section
            .biome_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (section, biomes) in expected.biomes.as_chunks::<64>().0.iter().enumerate() {
            for (index, &expected_id) in biomes.iter().enumerate() {
                let actual_id = network_biome_at(&palettes[section], index);
                let actual = Biome::from_id(actual_id).map(|biome| biome.registry_id);
                let expected_name = fixture.biome_names[usize::from(expected_id)].as_str();
                if actual != Some(expected_name) {
                    *mismatches
                        .entry((expected.x, expected.z, actual, expected_name))
                        .or_insert(0) += 1;
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "Anvil / network biome pairs: {mismatches:?}"
    );
}

#[test]
fn vanilla_anvil_biomes_match_chunk_network_palettes() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let world = get_world_gen(
        Seed(fixture.seed),
        Dimension::OVERWORLD,
        false,
        vec![],
        String::new(),
    );
    let WorldGenerator::Noise(generator) = &*world else {
        unreachable!()
    };
    check_biomes(generator, &world, &fixture);
    Ok(())
}
