use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs, PathComputationType,
};
use crate::world::World;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{
    BlockDirection, BlockState, BlockStateId, block_properties::LanternLikeProperties, tag,
};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

#[pumpkin_block_from_tag("minecraft:lanterns")]
pub struct LanternBlock;

impl BlockBehaviour for LanternBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = pumpkin_data::block_properties::LanternLikeProperties::default(args.block);
        props.r#waterlogged = args.replacing.water_source();

        let block_up_state = args.world.get_block_state(&args.position.up());
        if block_up_state.is_center_solid(BlockDirection::Down) {
            props.r#hanging = true;
        }

        props.to_state_id(args.block)
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        args.world
            .is_some_and(|world| can_place_at(world, args.position))
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let props = LanternLikeProperties::from_state_id(args.state_id);
        if !can_survive(args.world, args.position, props.r#hanging) {
            args.world
                .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        }
        args.state_id
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let props = LanternLikeProperties::from_state_id(state_id);
        if !can_survive(args.world, args.position, props.r#hanging) {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

fn can_survive(world: &World, position: &BlockPos, hanging: bool) -> bool {
    // Mirrors LanternBlock.canSurvive/getConnectedDirection: HANGING selects one support side.
    if hanging {
        return world
            .get_block_state(&position.up())
            .is_center_solid(BlockDirection::Down);
    }

    //idk why this don't update with .is_center_solid so this is a 'temporary patch'
    if world
        .get_block(&position.down())
        .has_tag(&tag::Block::C_FENCE_GATES)
    {
        let fence_gate_props =
            pumpkin_data::block_properties::OakFenceGateLikeProperties::from_state_id(
                world.get_block_state_id(&position.down()),
            );

        if fence_gate_props.open {
            return false;
        }
    }
    let (block_down, block_down_state) = world.get_block_and_state(&position.down());
    block_down_state.is_center_solid(BlockDirection::Up)
        || block_down.has_tag(&tag::Block::MINECRAFT_UNSTABLE_BOTTOM_CENTER)
}

fn can_place_at(world: &World, position: &BlockPos) -> bool {
    // Placement checks run before the selected HANGING state is available.
    if world
        .get_block(&position.down())
        .has_tag(&tag::Block::C_FENCE_GATES)
    {
        let fence_gate_props =
            pumpkin_data::block_properties::OakFenceGateLikeProperties::from_state_id(
                world.get_block_state_id(&position.down()),
            );

        if fence_gate_props.open {
            return false;
        }
    }
    let (block_down, block_down_state) = world.get_block_and_state(&position.down());
    let block_up_state = world.get_block_state(&position.up());
    block_down_state.is_center_solid(BlockDirection::Up)
        || block_up_state.is_center_solid(BlockDirection::Down)
        || block_down.has_tag(&tag::Block::MINECRAFT_UNSTABLE_BOTTOM_CENTER)
}
