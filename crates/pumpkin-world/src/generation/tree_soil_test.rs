use super::test::{TestBlockRegistry, TestGridCache, surface_biomes};
use crate::{
    chunk_system::StagedChunkEnum,
    generation::{generator::WorldGenerator, get_world_gen, proto_chunk::ProtoChunk},
};
use pumpkin_data::dimension::Dimension;
use pumpkin_util::world_seed::Seed;

#[test]
fn seed_zero_natural_tree_trunks_retain_soil() {
    use crate::generation::height_limit::HeightLimitView;
    use crate::generation::proto_chunk::GenerationCache;
    use pumpkin_data::tag::{self, Taggable};
    use pumpkin_util::math::vector3::Vector3;

    let world_gen = get_world_gen(
        Seed(0),
        Dimension::OVERWORLD,
        false,
        Vec::new(),
        String::new(),
    );
    let WorldGenerator::Noise(generator) = &*world_gen else {
        unreachable!()
    };
    let mut chunks = std::collections::HashMap::new();
    for cx in -1..=2 {
        for cz in -1..=2 {
            let mut chunk = ProtoChunk::new(cx, cz, &world_gen);
            chunk.step_to_biomes(generator);
            chunk.stage = StagedChunkEnum::StructureReferences;
            chunk.step_to_noise(generator);
            let biomes = surface_biomes(&world_gen, cx, cz);
            chunk.step_to_surface(generator, &biomes);
            chunk.step_to_carvers(generator);
            chunks.insert((cx, cz), chunk);
        }
    }
    let mut cache = TestGridCache {
        center_pos: (0, 0),
        chunks,
    };
    let surface_heights: Vec<_> = (0..32)
        .flat_map(|x| {
            let cache = &cache;
            (0..32).map(move |z| cache.get_top_y(&pumpkin_util::HeightMap::OceanFloorWg, x, z))
        })
        .collect();
    for cx in 0..=1 {
        for cz in 0..=1 {
            cache.center_pos = (cx, cz);
            ProtoChunk::generate_features_and_structure(
                &mut cache,
                &TestBlockRegistry,
                &generator.random_config,
            );
        }
    }
    let mut trunks = Vec::new();
    let mut unsupported = Vec::new();
    for x in 0..32 {
        for z in 0..32 {
            // Find the lowest log column; require a vertical trunk to exclude branches.
            let bottom = cache.bottom_y() as i32;
            let top = bottom + cache.height() as i32;
            let Some(y) = (bottom + 1..top - 2).find(|&y| {
                GenerationCache::get_block_state(&cache, &Vector3::new(x, y, z))
                    .to_block()
                    .has_tag(&tag::Block::MINECRAFT_LOGS)
            }) else {
                continue;
            };
            // Tall oak branches are not trunk bases; roots start at the terrain surface.
            if y > surface_heights[(x * 32 + z) as usize] + 1 {
                continue;
            }
            let is_trunk = (1..=2).all(|dy| {
                GenerationCache::get_block_state(&cache, &Vector3::new(x, y + dy, z))
                    .to_block()
                    .has_tag(&tag::Block::MINECRAFT_LOGS)
            });
            if !is_trunk {
                continue;
            }
            let below =
                GenerationCache::get_block_state(&cache, &Vector3::new(x, y - 1, z)).to_block();
            trunks.push((x, y, z));
            if !below.has_tag(&tag::Block::MINECRAFT_DIRT) {
                unsupported.push(((x, y, z), below.name));
            }
        }
    }
    assert!(
        !trunks.is_empty(),
        "fixed seed must generate natural trunks"
    );
    assert!(
        unsupported.is_empty(),
        "seed 0 chunks (0..=1, 0..=1): {unsupported:?}; trunks: {trunks:?}"
    );
}
