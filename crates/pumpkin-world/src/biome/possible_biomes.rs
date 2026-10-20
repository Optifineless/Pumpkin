use pumpkin_data::chunk::{Biome, BiomeTree};
use rustc_hash::FxHashSet;

use super::ActiveBiomeSupplier;

impl ActiveBiomeSupplier {
    /// BiomeSource.possibleBiomes follows the configured source, including overrides.
    pub(crate) fn possible_biomes(&self) -> FxHashSet<u8> {
        let mut out = FxHashSet::default();
        match self {
            Self::MultiNoise(supplier) => collect_tree_biomes(supplier.source, &mut out),
            Self::Fixed(supplier) => {
                out.insert(supplier.biome.id);
            }
            Self::End(_) => {
                for biome in [
                    &Biome::THE_END,
                    &Biome::END_HIGHLANDS,
                    &Biome::END_MIDLANDS,
                    &Biome::SMALL_END_ISLANDS,
                    &Biome::END_BARRENS,
                ] {
                    out.insert(biome.id);
                }
            }
        }
        out
    }
}

fn collect_tree_biomes(tree: &'static BiomeTree, out: &mut FxHashSet<u8>) {
    match tree {
        BiomeTree::Leaf { biome, .. } => {
            out.insert(biome.id);
        }
        BiomeTree::Branch { nodes, .. } => {
            for node in *nodes {
                collect_tree_biomes(node, out);
            }
        }
    }
}
