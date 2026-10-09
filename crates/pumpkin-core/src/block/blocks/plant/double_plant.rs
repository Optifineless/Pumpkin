use crate::block::GetStateForNeighborUpdateArgs;

#[cfg(test)]
mod tests;
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{
        DoubleBlockHalf, PitcherCropLikeProperties, SmallDripleafLikeProperties,
        TallSeagrassLikeProperties,
    },
};

pub(super) fn partner_update(args: &GetStateForNeighborUpdateArgs<'_>) -> Option<BlockStateId> {
    // DoublePlantBlock.updateShape: the other half must be the same block and opposite half.
    let half = part(args.block, args.state_id);
    let direction = if half == DoubleBlockHalf::Lower {
        BlockDirection::Up
    } else {
        BlockDirection::Down
    };
    if args.direction != direction {
        return None;
    }
    if Block::from_state_id(args.neighbor_state_id) == args.block {
        let other = part(args.block, args.neighbor_state_id);
        if half != other {
            return Some(args.state_id);
        }
    }
    Some(Block::AIR.default_state.id)
}

fn part(block: &Block, state: BlockStateId) -> DoubleBlockHalf {
    if block == &Block::SMALL_DRIPLEAF {
        SmallDripleafLikeProperties::from_state_id(state).half
    } else if block == &Block::PITCHER_CROP {
        PitcherCropLikeProperties::from_state_id(state).half
    } else {
        TallSeagrassLikeProperties::from_state_id(state).half
    }
}

pub(super) fn water_source(state: &pumpkin_data::BlockState) -> bool {
    // SmallDripleafBlock.mayPlaceOn: isSourceOfType(WATER).
    let (fluid, state) = crate::world::World::fluid_state_from_block_state(state.id);
    fluid.matches_type(&pumpkin_data::fluid::Fluid::WATER) && state.is_source
}

pub(super) fn full_water(state: &pumpkin_data::BlockState) -> bool {
    // TallSeagrassBlock.getStateForPlacement / canSurvive accepts full falling water too.
    let (fluid, state) = crate::world::World::fluid_state_from_block_state(state.id);
    let water = &pumpkin_data::fluid::Fluid::WATER;
    let full_amount = water.states[water.default_state_index as usize].level;
    fluid.matches_type(water) && state.level == full_amount
}

// DoublePlantBlock.preventDropFromBottomPart suppresses the loot-bearing lower half first.
pub(super) fn player_will_destroy(args: crate::block::PlayerWillDestroyArgs<'_>) {
    if args.player.gamemode.load() != pumpkin_util::GameMode::Creative
        || part(args.block, args.state.id) != DoubleBlockHalf::Upper
    {
        return;
    }
    let bottom = args.position.down();
    let state = args.world.get_block_state_id(&bottom);
    if Block::from_state_id(state) == args.block
        && part(args.block, state) == DoubleBlockHalf::Lower
    {
        let fluid = crate::world::World::fluid_state_from_block_state(state).0;
        let replacement = if fluid.matches_type(&pumpkin_data::fluid::Fluid::WATER) {
            Block::WATER.default_state.id
        } else {
            Block::AIR.default_state.id
        };
        super::super::player_destroy::remove_partner(args, bottom, replacement);
    }
}
