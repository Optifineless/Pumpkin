use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, BlockId, BlockStateId, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::{BlockAccessor, BlockFlags};
use rand::seq::SliceRandom;

use crate::block::{
    BlockBehaviour, BlockMetadata, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
    blocks::plant::PlantBlockBase,
};

pub struct DryVegetationBlock;

impl BlockMetadata for DryVegetationBlock {
    fn ids() -> Box<[BlockId]> {
        [
            BlockId::DEAD_BUSH,
            BlockId::TALL_DRY_GRASS,
            BlockId::SHORT_DRY_GRASS,
        ]
        .into()
    }
}

impl BlockBehaviour for DryVegetationBlock {
    fn is_valid_bonemeal_target(&self, args: crate::block::BonemealArgs<'_>) -> bool {
        if args.block == &Block::SHORT_DRY_GRASS {
            return true;
        }
        args.block == &Block::TALL_DRY_GRASS
            && horizontal_directions().into_iter().any(|direction| {
                can_spread_to(args.world, args.position.offset(direction.to_offset()))
            })
    }

    fn perform_bonemeal(&self, args: crate::block::BonemealArgs<'_>) -> bool {
        if args.block == &Block::SHORT_DRY_GRASS {
            return args.world.set_block_state_from_observed(
                args.position,
                args.state_id,
                Block::TALL_DRY_GRASS.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
        }
        let mut directions = horizontal_directions();
        directions.shuffle(&mut rand::rng());
        if let Some((position, observed)) = directions.into_iter().find_map(|direction| {
            let position = args.position.offset(direction.to_offset());
            let observed = args.world.get_block_state_id(&position);
            (observed.to_state().is_air() && can_spread_to(args.world, position))
                .then_some((position, observed))
        }) {
            return args.world.set_block_states_from_observed(&[
                (
                    *args.position,
                    args.state_id,
                    args.state_id,
                    BlockFlags::empty(),
                ),
                (
                    position,
                    observed,
                    Block::SHORT_DRY_GRASS.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                ),
            ]);
        }
        true
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        <Self as PlantBlockBase>::get_state_for_neighbor_update(
            self,
            args.world,
            args.position,
            args.state_id,
        )
    }
}

const fn horizontal_directions() -> [BlockDirection; 4] {
    [
        BlockDirection::North,
        BlockDirection::South,
        BlockDirection::West,
        BlockDirection::East,
    ]
}

fn can_spread_to(world: &crate::world::World, position: BlockPos) -> bool {
    world.is_loaded(&position)
        && world.get_block_state(&position).is_air()
        && world.block_registry.can_place_at(
            None,
            Some(world),
            world,
            None,
            &Block::SHORT_DRY_GRASS,
            Block::SHORT_DRY_GRASS.default_state,
            &position,
            None,
            None,
        )
}

impl PlantBlockBase for DryVegetationBlock {
    fn can_plant_on_top(&self, block_accessor: &dyn BlockAccessor, block_pos: &BlockPos) -> bool {
        let block_below = block_accessor.get_block(block_pos);
        block_below.has_tag(&tag::Block::MINECRAFT_SUPPORTS_DRY_VEGETATION)
    }
}
