use crate::block::BlockIsReplacing;
use crate::block::blocks::plant::PlantBlockBase;
use crate::block::{
    BlockBehaviour, BonemealArgs, CanPlaceAtArgs, CanUpdateAtArgs, GetStateForNeighborUpdateArgs,
    OnPlaceArgs, PathComputationType,
};
use crate::entity::EntityBase;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, fluid::Fluid, tag};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::{tick::TickPriority, world::BlockFlags};
use rand::RngExt;

type SeaPickleProperties = pumpkin_data::block_properties::SeaPickleLikeProperties;
// VoxelShape.java:222 (calculateFace) samples just inside the upper boundary.
const UP_FACE_SAMPLE: f64 = 0.999_999_9;

#[pumpkin_block("minecraft:sea_pickle")]
pub struct SeaPickleBlock;

impl BlockBehaviour for SeaPickleBlock {
    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        // SeaPickleBlock.isValidBonemealTarget requires water and living coral.
        let properties = SeaPickleProperties::from_state_id(args.state_id);
        properties.waterlogged
            && args
                .world
                .get_block(&args.position.down())
                .has_tag(&tag::Block::MINECRAFT_CORAL_BLOCKS)
    }

    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        // SeaPickleBlock.performBonemeal spreads only into water over living coral.
        let mut z_span = 1;
        let mut z_offset = 0;
        for x in 0..5 {
            for z in 0..z_span {
                for y in (args.position.0.y - 1)..=(args.position.0.y) {
                    let target = BlockPos::new(
                        args.position.0.x - 2 + x,
                        y,
                        args.position.0.z - z_offset + z,
                    );
                    if target != *args.position
                        && rand::rng().random_range(0..6) == 0
                        && args.world.get_block(&target) == &Block::WATER
                        && args
                            .world
                            .get_block(&target.down())
                            .has_tag(&tag::Block::MINECRAFT_CORAL_BLOCKS)
                    {
                        let mut properties = SeaPickleProperties::default(args.block);
                        properties.pickles = rand::rng().random_range(1..=4);
                        args.world.set_block_state(
                            &target,
                            properties.to_state_id(args.block),
                            BlockFlags::NOTIFY_ALL,
                        );
                    }
                }
            }
            if x < 2 {
                z_span += 2;
                z_offset += 1;
            } else {
                z_span -= 2;
                z_offset -= 1;
            }
        }

        let mut properties = SeaPickleProperties::from_state_id(args.state_id);
        properties.pickles = 4;
        args.world.set_block_state(
            args.position,
            properties.to_state_id(args.block),
            BlockFlags::NOTIFY_LISTENERS,
        );
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if args.player.get_entity().pose.load() != EntityPose::Crouching
            && let BlockIsReplacing::Itself(state_id) = args.replacing
        {
            let mut sea_pickle_prop = SeaPickleProperties::from_state_id(state_id);
            if sea_pickle_prop.pickles < 4 {
                sea_pickle_prop.pickles += 1;
            }
            return sea_pickle_prop.to_state_id(args.block);
        }

        let mut sea_pickle_prop = SeaPickleProperties::default(args.block);
        sea_pickle_prop.waterlogged = args.replacing.water_source();
        sea_pickle_prop.to_state_id(args.block)
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
    }

    fn can_update_at(&self, args: CanUpdateAtArgs<'_>) -> bool {
        args.player.get_entity().pose.load() != EntityPose::Crouching
            && SeaPickleProperties::from_state_id(args.state_id).pickles < 4
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        // SeaPickleBlock.updateShape checks survival before scheduling water.
        if !<Self as PlantBlockBase>::can_place_at(self, args.world, args.position) {
            return Block::AIR.default_state.id;
        }
        let properties = SeaPickleProperties::from_state_id(args.state_id);
        if properties.waterlogged {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as u8,
                TickPriority::Normal,
            );
        }
        args.state_id
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl PlantBlockBase for SeaPickleBlock {
    fn can_plant_on_top(
        &self,
        block_accessor: &dyn pumpkin_world::world::BlockAccessor,
        pos: &BlockPos,
    ) -> bool {
        // SeaPickleBlock.mayPlaceOn and VoxelShape.calculateFace sample the upper face.
        let state = block_accessor.get_block_state(pos);
        state.is_side_solid(BlockDirection::Up)
            || state
                .get_block_collision_shapes_at(pos)
                .any(|shape| shape.min.y <= UP_FACE_SAMPLE && shape.max.y > UP_FACE_SAMPLE)
    }
}
