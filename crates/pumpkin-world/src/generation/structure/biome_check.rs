use pumpkin_data::{
    structures::{Structure, StructureKeys},
    tag::{RegistryKey, get_tag_ids},
};

use crate::{
    biome::BiomeSupplier,
    generation::{
        biome_coords, noise::router::multi_noise_sampler::MultiNoiseSampler, positions::chunk_pos,
    },
};

const START_OFFSET_IN_CHUNK: i32 = 7;

pub(super) fn allowed_biomes(structure: &Structure) -> &'static [u16] {
    get_tag_ids(
        RegistryKey::WorldgenBiome,
        structure
            .biomes
            .strip_prefix('#')
            .unwrap_or(structure.biomes),
    )
    .unwrap_or(&[])
}

pub(super) struct StructureBiomeCheck<'a, 'b> {
    pub supplier: &'a dyn BiomeSupplier,
    pub sampler: &'a mut MultiNoiseSampler<'b>,
    pub allowed: &'static [u16],
    pub height_bounds: (i32, i32),
}

impl StructureBiomeCheck<'_, '_> {
    // Structure.GenerationContext.couldStructureExistInColumn checks every quart Y.
    pub(super) fn column(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let x = biome_coords::from_block(x);
        let z = biome_coords::from_block(z);
        (biome_coords::from_block(min_y)..=biome_coords::from_block(max_y)).any(|y| {
            self.allowed
                .contains(&(self.supplier.biome(x, y, z, self.sampler).id as u16))
        })
    }

    pub(super) fn terrain_column(
        &mut self,
        key: StructureKeys,
        chunk_x: i32,
        chunk_z: i32,
    ) -> bool {
        let (x, z) = match key {
            // EndCityStructure and WoodlandMansionStructure.findGenerationPoint.
            StructureKeys::EndCity | StructureKeys::Mansion => (
                chunk_pos::start_block_x(chunk_x) + START_OFFSET_IN_CHUNK,
                chunk_pos::start_block_z(chunk_z) + START_OFFSET_IN_CHUNK,
            ),
            // Structure.onTopOfChunkCenter. Jigsaws use their rotated center later.
            StructureKeys::Shipwreck
            | StructureKeys::ShipwreckBeached
            | StructureKeys::Monument => (
                chunk_pos::get_center_x(chunk_x),
                chunk_pos::get_center_z(chunk_z),
            ),
            _ => return true,
        };
        self.column(x, z, self.height_bounds.0 - 1, self.height_bounds.1)
    }
}
