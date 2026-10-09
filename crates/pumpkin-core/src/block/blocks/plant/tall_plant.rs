use crate::block::PlacedArgs;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::BlockId;
use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::{DoubleBlockHalf, TallSeagrassLikeProperties};
use pumpkin_world::world::BlockFlags;

use crate::block::{
    BlockBehaviour, BlockMetadata, BonemealArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
    blocks::plant::PlantBlockBase,
};

pub struct TallPlantBlock;

impl BlockMetadata for TallPlantBlock {
    fn ids() -> Box<[BlockId]> {
        [
            BlockId::TALL_GRASS,
            BlockId::LARGE_FERN,
            BlockId::PITCHER_PLANT,
            // TallFlowerBlocks
            BlockId::SUNFLOWER,
            BlockId::LILAC,
            BlockId::PEONY,
            BlockId::ROSE_BUSH,
        ]
        .into()
    }
}

impl BlockBehaviour for TallPlantBlock {
    fn player_will_destroy(&self, args: crate::block::PlayerWillDestroyArgs<'_>) {
        super::double_plant::player_will_destroy(args);
    }

    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        // Only TallFlowerBlock implements BonemealableBlock; grass, fern and pitcher plant do not.
        [
            BlockId::SUNFLOWER,
            BlockId::LILAC,
            BlockId::PEONY,
            BlockId::ROSE_BUSH,
        ]
        .contains(&args.block.id)
    }
    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        // TallFlowerBlock.performBonemeal pops a flower item without replacing the plant.
        if let Some(item) = pumpkin_data::item::Item::from_id(args.block.item_id) {
            args.world.drop_stack(
                args.position,
                pumpkin_data::item_stack::ItemStack::new(1, item),
            );
        }
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        let up_pos = args.position.up();

        let upper_state = args.block_accessor.get_block_state(&up_pos);
        let Some(world) = args.world else {
            return <Self as PlantBlockBase>::can_place_at(
                self,
                args.block_accessor,
                args.position,
            ) && upper_state.is_air();
        };

        if up_pos.0.y > world.get_top_y() {
            return false;
        }
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
            && upper_state.is_air()
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if let Some(state) = super::double_plant::partner_update(&args) {
            return state;
        }
        if args.direction == BlockDirection::Down
            && TallSeagrassLikeProperties::from_state_id(args.state_id).half
                == DoubleBlockHalf::Lower
            && !<Self as PlantBlockBase>::can_place_at(self, args.world, args.position)
        {
            return Block::AIR.default_state.id;
        }
        args.state_id
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if TallSeagrassLikeProperties::from_state_id(args.state_id).half != DoubleBlockHalf::Lower {
            return;
        }
        {
            let mut tall_plant_props = TallSeagrassLikeProperties::from_state_id(args.state_id);
            tall_plant_props.half = DoubleBlockHalf::Upper;
            args.world.set_block_state(
                &args.position.offset(BlockDirection::Up.to_offset()),
                tall_plant_props.to_state_id(args.block),
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
            );
        }
    }
}

impl PlantBlockBase for TallPlantBlock {}
