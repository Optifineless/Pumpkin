use pumpkin_data::{
    BlockState,
    structures::{Structure, StructureKeys, StructureSet},
};
use rustc_hash::FxHashSet;

use super::{
    biome_check::allowed_biomes,
    height_sampler::NoiseHeightSampler,
    lazily_generate_structure_with_biome_check,
    placement::weighted_structure_order,
    structures::{HeightSampler, StructureGeneratorContext, create_chunk_random},
};
use crate::generation::{
    generator::VanillaGenerator, noise::router::multi_noise_sampler::MultiNoiseSampler,
};

pub fn has_possible_biome(key: StructureKeys, possible: &FxHashSet<u8>) -> bool {
    allowed_biomes(Structure::get(&key))
        .iter()
        .any(|id| possible.contains(&(*id as u8)))
}

/// Resolve the same first successful weighted entry as ChunkGenerator.createStructures.
pub fn winner_at(
    generator: &VanillaGenerator,
    set: &StructureSet,
    possible: &FxHashSet<u8>,
    chunk_x: i32,
    chunk_z: i32,
) -> Option<StructureKeys> {
    let seed = generator.random_config.seed;
    for entry in weighted_structure_order(set.structures, seed, chunk_x, chunk_z) {
        let key = entry.structure;
        let start = generator
            .global_structure_cache
            .get_or_compute_structure_start(key, chunk_x, chunk_z, || {
                if !has_possible_biome(key, possible) {
                    return None;
                }
                let settings = generator.settings;
                // Construct real column sampling only after the cheap biome checks succeed.
                let mut heights = DeferredHeightSampler {
                    generator,
                    sampler: None,
                };
                let mut biomes = MultiNoiseSampler::generate(&generator.base_router.multi_noise);
                let seed = seed as i64;
                let context = StructureGeneratorContext {
                    seed,
                    chunk_x,
                    chunk_z,
                    random: create_chunk_random(seed, chunk_x, chunk_z),
                    sea_level: settings.sea_level,
                    min_y: i32::from(settings.shape.min_y).max(generator.dimension.min_y),
                    height: settings.shape.height.min(generator.dimension.height as u16),
                    height_sampler: Some(&mut heights),
                    structure_key: Some(key),
                };
                lazily_generate_structure_with_biome_check(
                    key,
                    Structure::get(&key),
                    context,
                    &generator.biome_supplier,
                    &mut biomes,
                    Some((
                        generator.dimension.min_y,
                        generator.dimension.min_y + generator.dimension.height - 1,
                    )),
                )
            });
        if start.is_some() {
            return Some(key);
        }
    }
    None
}

struct DeferredHeightSampler<'a> {
    generator: &'a VanillaGenerator,
    sampler: Option<NoiseHeightSampler<'a>>,
}

impl<'a> DeferredHeightSampler<'a> {
    fn sampler(&mut self) -> &mut NoiseHeightSampler<'a> {
        self.sampler
            .get_or_insert_with(|| NoiseHeightSampler::new(self.generator))
    }
}

impl HeightSampler for DeferredHeightSampler<'_> {
    fn estimate_height(&mut self, x: i32, z: i32) -> i32 {
        self.sampler().estimate_height(x, z)
    }

    fn estimate_ocean_floor_height(&mut self, x: i32, z: i32) -> i32 {
        self.sampler().estimate_ocean_floor_height(x, z)
    }

    fn sample_column_block(&mut self, x: i32, z: i32, y: i32) -> Option<&'static BlockState> {
        self.sampler().sample_column_block(x, z, y)
    }
}
