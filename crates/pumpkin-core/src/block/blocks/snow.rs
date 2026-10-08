use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockState, BlockStateId, block_properties::SnowLikeProperties, tag};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::{
    tick::TickPriority,
    world::{BlockAccessor, BlockFlags},
};

use crate::block::{
    BlockBehaviour, CanUpdateAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs, PathComputationType, RandomTickArgs,
};

#[pumpkin_block("minecraft:snow")]
pub struct LayeredSnowBlock;

impl BlockBehaviour for LayeredSnowBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if !can_place_at(args.world, args.position) {
            return Block::AIR.default_state.id;
        }
        let mut props = SnowLikeProperties::default(args.block);
        // SnowLayerBlock.getStateForPlacement keeps eight layers as snow.
        props.layers = match args.replacing {
            crate::block::BlockIsReplacing::Itself(state) => {
                (SnowLikeProperties::from_state_id(state).layers + 1).min(8)
            }
            _ => 1,
        };
        props.to_state_id(&Block::SNOW)
    }

    // SnowLayerBlock.canBeReplaced routes stacking through BlockItem.place.
    fn can_update_at(&self, args: CanUpdateAtArgs<'_>) -> bool {
        let layers = SnowLikeProperties::from_state_id(args.state_id).layers;
        layers < 8
            && (args.position != &args.use_item_on.position
                || args.direction == pumpkin_data::BlockDirection::Up)
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if !can_place_at(args.world.as_ref(), args.position) {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        // Snow layers melt when lit by block light above level 11,
        // e.g. from a nearby torch.
        if args.world.get_block_light_level(args.position).unwrap_or(0) > 11 {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if !can_place_at(args.world, args.position) {
            args.world
                .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        }
        args.state_id
    }

    fn is_pathfindable(&self, state: &BlockState, computation_type: PathComputationType) -> bool {
        computation_type == PathComputationType::Land
            && SnowLikeProperties::from_state_id(state.id).layers < 5
    }
}

fn can_place_at(block_accessor: &dyn BlockAccessor, position: &BlockPos) -> bool {
    let below_pos = position.down();
    let (below_block, state) = block_accessor.get_block_and_state(&below_pos);

    if below_block.has_tag(&tag::Block::MINECRAFT_CANNOT_SUPPORT_SNOW_LAYER) {
        return false;
    }
    if below_block.has_tag(&tag::Block::MINECRAFT_SUPPORT_OVERRIDE_SNOW_LAYER) {
        return true;
    }

    // Block.isFaceFullSquare(collisionShape, Direction.UP): the collision shape must fully cover
    // the top face, e.g. leaves are not "side solid" but do support snow layers.
    state.get_block_collision_shapes().any(|shape| {
        shape.max.y >= 1.0
            && shape.min.x <= 0.0
            && shape.max.x >= 1.0
            && shape.min.z <= 0.0
            && shape.max.z >= 1.0
    }) || (below_block == &Block::SNOW && SnowLikeProperties::from_state_id(state.id).layers == 8)
}
