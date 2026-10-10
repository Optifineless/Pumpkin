use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockMetadata, GetComparatorOutputArgs, PathComputationType, UseWithItemArgs,
};
use pumpkin_data::block_properties::WaterCauldronLikeProperties;
use pumpkin_data::{BlockId, BlockState};

#[path = "cauldron_interactions.rs"]
mod interactions;

pub struct CauldronBlock;

impl BlockMetadata for CauldronBlock {
    fn ids() -> Box<[BlockId]> {
        [
            BlockId::CAULDRON,
            BlockId::WATER_CAULDRON,
            BlockId::LAVA_CAULDRON,
            BlockId::POWDER_SNOW_CAULDRON,
        ]
        .into()
    }
}

impl BlockBehaviour for CauldronBlock {
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        interactions::interact(args)
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        match args.block.id {
            BlockId::WATER_CAULDRON | BlockId::POWDER_SNOW_CAULDRON => {
                let state_id = args.world.get_block_state_id(args.position);
                let props = WaterCauldronLikeProperties::from_state_id(state_id);
                Some(props.level)
            }
            BlockId::LAVA_CAULDRON => Some(3),
            _ => Some(0),
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

#[cfg(test)]
#[path = "cauldron_tests.rs"]
mod tests;
