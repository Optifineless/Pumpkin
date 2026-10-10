use pumpkin_data::{Block, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockAccessor;

use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, PlacedArgs,
    blocks::plant::{PlantBlockBase, seagrass::supports_seagrass},
};
#[pumpkin_block("minecraft:tall_seagrass")]
pub struct TallSeaGrassBlock;
impl BlockBehaviour for TallSeaGrassBlock {
    fn player_will_destroy(&self, args: crate::block::PlayerWillDestroyArgs<'_>) {
        super::double_plant::player_will_destroy(args);
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
            && super::double_plant::full_water(
                args.block_accessor.get_block_state(&args.position.up()),
            )
            && args
                .world
                .is_none_or(|world| args.position.0.y < world.get_top_y())
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if let Some(state) = super::double_plant::partner_update(&args) {
            return state;
        }
        let props = pumpkin_data::block_properties::TallSeagrassLikeProperties::from_state_id(
            args.state_id,
        );
        if props.half == pumpkin_data::block_properties::DoubleBlockHalf::Lower
            && args.direction == pumpkin_data::BlockDirection::Down
            && !supports_seagrass(
                args.world.get_block(&args.position.down()),
                args.world.get_block_state(&args.position.down()),
            )
        {
            return Block::AIR.default_state.id;
        }
        args.state_id
    }
    fn placed(&self, args: PlacedArgs<'_>) {
        // DoublePlantBlock.setPlacedBy, inherited by TallSeagrassBlock.
        let mut props = pumpkin_data::block_properties::TallSeagrassLikeProperties::from_state_id(
            args.state_id,
        );
        if props.half != pumpkin_data::block_properties::DoubleBlockHalf::Lower {
            return;
        }
        props.half = pumpkin_data::block_properties::DoubleBlockHalf::Upper;
        args.world.set_block_state(
            &args.position.up(),
            props.to_state_id(args.block),
            pumpkin_world::world::BlockFlags::NOTIFY_ALL
                | pumpkin_world::world::BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
    }
}

impl PlantBlockBase for TallSeaGrassBlock {
    fn can_plant_on_top(&self, block_accessor: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        supports_seagrass(
            block_accessor.get_block(pos),
            block_accessor.get_block_state(pos),
        ) && super::double_plant::full_water(block_accessor.get_block_state(&pos.up()))
    }
}
