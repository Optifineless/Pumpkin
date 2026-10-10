use crate::block::blocks::growing_plant::GrowingPlant;
use crate::block::{
    BlockBehaviour, BlockMetadata, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs,
};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockStateId};

pub struct WeepingVinesBlock;

const PLANT: GrowingPlant = GrowingPlant {
    head: &Block::WEEPING_VINES,
    body: &Block::WEEPING_VINES_PLANT,
    growth_direction: BlockDirection::Down,
};

impl BlockMetadata for WeepingVinesBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::WEEPING_VINES, BlockId::WEEPING_VINES_PLANT].into()
    }
}

impl BlockBehaviour for WeepingVinesBlock {
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
