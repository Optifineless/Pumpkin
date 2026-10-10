use crate::block::blocks::growing_plant::GrowingPlant;
use crate::block::{
    BlockBehaviour, BlockMetadata, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs,
};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockStateId};

pub struct TwistingVinesBlock;

const PLANT: GrowingPlant = GrowingPlant {
    head: &Block::TWISTING_VINES,
    body: &Block::TWISTING_VINES_PLANT,
    growth_direction: BlockDirection::Up,
};

impl BlockMetadata for TwistingVinesBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::TWISTING_VINES, BlockId::TWISTING_VINES_PLANT].into()
    }
}

impl BlockBehaviour for TwistingVinesBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        PLANT.can_survive(args.block_accessor, args.position)
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        PLANT.get_state_for_placement(&args)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        PLANT.update_shape(&args)
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        PLANT.tick(&args);
    }
}
