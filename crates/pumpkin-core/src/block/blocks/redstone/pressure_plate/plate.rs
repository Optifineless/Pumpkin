use pumpkin_data::{
    Block, BlockDirection, BlockId, BlockState, BlockStateId,
    tag::{self},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{
        BlockBehaviour, BlockMetadata, CanPlaceAtArgs, EmitsRedstonePowerArgs,
        GetRedstonePowerArgs, OnEntityCollisionArgs, OnNeighborUpdateArgs, OnScheduledTickArgs,
        OnStateReplacedArgs,
    },
    world::World,
};

use super::{EntitySensitivity, PressurePlate, detection_box_at, eligible_entity_count};

/// This is for Normal Pressure plates, so not Gold or Iron
pub struct PressurePlateBlock;

type PressurePlateProps = pumpkin_data::block_properties::StonePressurePlateLikeProperties;

impl BlockMetadata for PressurePlateBlock {
    fn ids() -> Box<[BlockId]> {
        let mut combined = Vec::new();
        combined.extend_from_slice(tag::Block::MINECRAFT_WOODEN_PRESSURE_PLATES.1);
        combined.extend_from_slice(tag::Block::MINECRAFT_STONE_PRESSURE_PLATES.1);
        combined.iter().map(|v| BlockId::new_or_air(*v)).collect()
    }
}

impl BlockBehaviour for PressurePlateBlock {
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        self.on_entity_collision_pp(args);
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state = args.world.get_block_state(args.position);
        if state.id.to_block() != args.block {
            return;
        }
        let output = self.get_redstone_output(args.block, state.id);
        if output > 0 {
            Self.update_plate_state(args.world, args.position, args.block, state, output);
        }
    }

    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        self.on_state_replaced_pp(args);
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        self.get_redstone_output(args.block, args.state.id)
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        if args.direction == BlockDirection::Up {
            return self.get_redstone_output(args.block, args.state.id);
        }
        0
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let observed = args.world.get_block_state_id(args.position);
        if observed.to_block() != args.block {
            return;
        }
        if !Self::can_pressure_plate_place_at(args.world, args.position) {
            args.world.break_block_from_observed(
                args.position,
                observed,
                None,
                BlockFlags::NOTIFY_ALL,
            );
        }
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        args.world
            .is_some_and(|world| Self::can_pressure_plate_place_at(world, args.position))
    }
}

impl PressurePlate for PressurePlateBlock {
    fn get_redstone_output(&self, _block: &Block, state: BlockStateId) -> u8 {
        let props = PressurePlateProps::from_state_id(state);
        if props.powered { 15 } else { 0 }
    }

    fn calculate_redstone_output(&self, world: &World, block: &Block, pos: &BlockPos) -> u8 {
        let aabb = detection_box_at(pos);
        let sensitivity = if tag::Block::MINECRAFT_STONE_PRESSURE_PLATES
            .1
            .contains(&block.id.as_u16())
        {
            EntitySensitivity::Living
        } else {
            EntitySensitivity::Everything
        };
        if eligible_entity_count(world, &aabb, sensitivity) > 0 {
            return 15;
        }
        0
    }

    fn set_redstone_output(&self, block: &Block, state: &BlockState, output: u8) -> BlockStateId {
        let mut props = PressurePlateProps::from_state_id(state.id);
        props.powered = output > 0;
        props.to_state_id(block)
    }
}
