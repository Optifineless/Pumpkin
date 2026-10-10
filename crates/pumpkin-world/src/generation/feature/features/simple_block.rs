use pumpkin_data::{
    Block, BlockState,
    block_properties::{BlockProperties, DoubleBlockHalf, TallSeagrassLikeProperties},
};
use pumpkin_util::{math::position::BlockPos, random::RandomGenerator};

use crate::generation::proto_chunk::GenerationCache;
use crate::{
    generation::block_state_provider::BlockStateProvider,
    world::{BlockAccessor, WorldPortalExt},
};

pub struct SimpleBlockFeature {
    pub to_place: BlockStateProvider,
    pub schedule_tick: Option<bool>,
}

impl SimpleBlockFeature {
    pub fn generate<T: GenerationCache>(
        &self,
        block_registry: &dyn WorldPortalExt,
        chunk: &mut T,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        let state = self.to_place.get(random, pos, chunk, block_registry);
        let block = Block::from_state_id(state.id);
        let block_accessor: &dyn BlockAccessor = chunk;
        if !block_registry.can_place_at(block, state, block_accessor, &pos) {
            return false;
        }

        if <TallSeagrassLikeProperties as BlockProperties>::handles_block_id(block.id) {
            let upper_pos = pos.up();
            if !block_accessor.get_block_state(&upper_pos).is_air() {
                return false;
            }

            // SimpleBlockFeature.place checks headroom, then DoublePlantBlock.placeAt writes both halves.
            let mut properties = TallSeagrassLikeProperties::from_state_id(state.id);
            properties.half = DoubleBlockHalf::Lower;
            chunk.set_block_state(&pos.0, BlockState::from_id(properties.to_state_id(block)));
            properties.half = DoubleBlockHalf::Upper;
            chunk.set_block_state(
                &upper_pos.0,
                BlockState::from_id(properties.to_state_id(block)),
            );
        } else {
            chunk.set_block_state(&pos.0, state);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        generation::{
            block_state_provider::SimpleStateProvider,
            feature::configured_features::{CONFIGURED_FEATURES, ConfiguredFeature},
            get_world_gen,
            proto_chunk::{GenerationCache, ProtoChunk},
        },
        world::{BlockAccessor, WorldPortalExt},
    };
    use pumpkin_data::{Mirror, Rotation, chunk::Biome, dimension::Dimension};
    use pumpkin_util::{
        math::position::BlockPos,
        random::{RandomGenerator, legacy_rand::LegacyRand},
        world_seed::Seed,
    };

    struct TestBlockRegistry;

    impl WorldPortalExt for TestBlockRegistry {
        fn can_place_at(
            &self,
            _block: &Block,
            _state: &BlockState,
            _block_accessor: &dyn BlockAccessor,
            _block_pos: &BlockPos,
        ) -> bool {
            true
        }

        fn mirror(
            &self,
            block: &Block,
            state_id: pumpkin_data::BlockStateId,
            mirror: Mirror,
        ) -> &'static BlockState {
            block.mirror(state_id, mirror)
        }

        fn rotate(
            &self,
            block: &Block,
            state_id: pumpkin_data::BlockStateId,
            rotation: Rotation,
        ) -> &'static BlockState {
            block.rotate(state_id, rotation)
        }

        fn spawn_mobs_for_chunk_generation(
            &self,
            _cache: &mut dyn GenerationCache,
            _biome: &'static Biome,
            _chunk_x: i32,
            _chunk_z: i32,
        ) {
        }
    }

    fn empty_chunk(pos: BlockPos) -> ProtoChunk {
        let generator = get_world_gen(
            Seed(0),
            Dimension::OVERWORLD,
            true,
            Vec::new(),
            String::new(),
        );
        ProtoChunk::new(pos.0.x >> 4, pos.0.z >> 4, &generator)
    }

    fn tall_grass_feature() -> SimpleBlockFeature {
        SimpleBlockFeature {
            to_place: BlockStateProvider::Simple(SimpleStateProvider {
                state: Block::TALL_GRASS.default_state,
            }),
            schedule_tick: None,
        }
    }

    #[test]
    fn flower_meadow_tall_grass_state_places_both_halves() {
        let Some(ConfiguredFeature::SimpleBlock(feature)) = CONFIGURED_FEATURES
            .get(&pumpkin_data::configured_feature::ConfiguredFeature::FlowerMeadow)
        else {
            panic!("FlowerMeadow must use SimpleBlockFeature");
        };
        let BlockStateProvider::DualNoise(provider) = &feature.to_place else {
            panic!("FlowerMeadow must use its vanilla dual-noise state provider");
        };
        let Some(state) = provider
            .base
            .states
            .iter()
            .find(|state| Block::from_state_id(state.id).id == Block::TALL_GRASS.id)
            .copied()
        else {
            panic!("FlowerMeadow provider must include tall grass");
        };
        let feature = SimpleBlockFeature {
            to_place: BlockStateProvider::Simple(SimpleStateProvider { state }),
            schedule_tick: feature.schedule_tick,
        };

        let pos = BlockPos::new(4, 64, 4);
        let mut chunk = empty_chunk(pos);
        GenerationCache::set_block_state(&mut chunk, &pos.0, Block::AIR.default_state);
        GenerationCache::set_block_state(&mut chunk, &pos.up().0, Block::AIR.default_state);
        let mut random = RandomGenerator::Legacy(LegacyRand::from_seed(0));

        assert!(feature.generate(&TestBlockRegistry, &mut chunk, &mut random, pos,));
        let lower = GenerationCache::get_block_state(&chunk, &pos.0);
        let upper = GenerationCache::get_block_state(&chunk, &pos.up().0);
        assert_eq!(Block::from_state_id(lower).id, Block::TALL_GRASS.id);
        assert_eq!(Block::from_state_id(upper).id, Block::TALL_GRASS.id);
        assert_eq!(
            TallSeagrassLikeProperties::from_state_id(lower).half,
            DoubleBlockHalf::Lower
        );
        assert_eq!(
            TallSeagrassLikeProperties::from_state_id(upper).half,
            DoubleBlockHalf::Upper
        );
    }

    #[test]
    fn simple_block_tall_grass_declines_when_upper_space_is_occupied() {
        let pos = BlockPos::new(4, 64, 4);
        let mut chunk = empty_chunk(pos);
        GenerationCache::set_block_state(&mut chunk, &pos.0, Block::AIR.default_state);
        GenerationCache::set_block_state(&mut chunk, &pos.up().0, Block::STONE.default_state);
        let mut random = RandomGenerator::Legacy(LegacyRand::from_seed(0));

        assert!(!tall_grass_feature().generate(&TestBlockRegistry, &mut chunk, &mut random, pos,));
        assert_eq!(
            GenerationCache::get_block_state(&chunk, &pos.0),
            Block::AIR.default_state.id
        );
    }
}
