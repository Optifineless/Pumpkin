use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId,
    tag::{self, Taggable},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockAccessor;

use crate::block::{
    BlockBehaviour, BonemealArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
    blocks::plant::PlantBlockBase,
};
#[pumpkin_block("minecraft:seagrass")]
pub struct SeaGrassBlock;
impl BlockBehaviour for SeaGrassBlock {
    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        args.world.get_block(&args.position.up()) == &Block::WATER
    }
    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        // SeagrassBlock.performBonemeal writes both halves without neighbor callbacks in between.
        use pumpkin_data::block_properties::{DoubleBlockHalf, TallSeagrassLikeProperties};
        use pumpkin_world::world::BlockFlags;
        let mut props = TallSeagrassLikeProperties::default(&Block::TALL_SEAGRASS);
        props.half = DoubleBlockHalf::Lower;
        args.world.set_block_state(
            args.position,
            props.to_state_id(&Block::TALL_SEAGRASS),
            BlockFlags::NOTIFY_LISTENERS | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
        props.half = DoubleBlockHalf::Upper;
        args.world.set_block_state(
            &args.position.up(),
            props.to_state_id(&Block::TALL_SEAGRASS),
            BlockFlags::NOTIFY_LISTENERS,
        );
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

impl PlantBlockBase for SeaGrassBlock {
    fn can_plant_on_top(
        &self,
        block_accessor: &dyn pumpkin_world::world::BlockAccessor,
        pos: &pumpkin_util::math::position::BlockPos,
    ) -> bool {
        let (support_block, support_block_state) = block_accessor.get_block_and_state(pos);
        let replacing_block = block_accessor.get_block(&pos.up());
        if replacing_block != &Block::WATER && replacing_block != &Block::SEAGRASS {
            return false;
        }
        if supports_seagrass(support_block, support_block_state) {
            return true;
        }
        false
    }
    fn get_state_for_neighbor_update(
        &self,
        block_accessor: &dyn BlockAccessor,
        block_pos: &BlockPos,
        block_state: BlockStateId,
    ) -> BlockStateId {
        if !<Self as PlantBlockBase>::can_place_at(self, block_accessor, block_pos) {
            return Block::WATER.default_state.id;
        }
        block_state
    }
}
#[must_use]
pub fn supports_seagrass(support_block: &Block, support_block_state: &BlockState) -> bool {
    support_block_state.is_side_solid(BlockDirection::Up)
        && !support_block.has_tag(&tag::Block::MINECRAFT_CANNOT_SUPPORT_SEAGRASS)
}
