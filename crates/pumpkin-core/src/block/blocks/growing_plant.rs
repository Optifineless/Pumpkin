use crate::{
    block::{GetStateForNeighborUpdateArgs, OnPlaceArgs, OnScheduledTickArgs},
    world::World,
};
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{CaveVinesLikeProperties, CaveVinesPlantLikeProperties, KelpLikeProperties},
    fluid::Fluid,
    tag::{self, Taggable},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::{
    tick::TickPriority,
    world::{BlockAccessor, BlockFlags},
};
use rand::RngExt;

const MAX_AGE: u8 = 25;

pub(crate) struct GrowingPlant {
    pub head: &'static Block,
    pub body: &'static Block,
    pub growth_direction: BlockDirection,
}

impl GrowingPlant {
    // GrowingPlantBlock.canSurvive checks only the attachment, not the growth-direction cell.
    pub(crate) fn can_survive(&self, world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        let support_pos = pos.offset(self.growth_direction.opposite().to_offset());
        let (block, state) = world.get_block_and_state(&support_pos);
        if self.head == &Block::KELP && block.has_tag(&tag::Block::MINECRAFT_CANNOT_SUPPORT_KELP) {
            return false;
        }
        self.is_segment(block) || state.is_side_solid(self.growth_direction)
    }

    fn is_segment(&self, block: &Block) -> bool {
        block == self.head || block == self.body
    }

    fn head_state(&self, berries: bool) -> BlockStateId {
        // GrowingPlantHeadBlock.getStateForPlacement(RandomSource).
        let age = rand::rng().random_range(0..MAX_AGE);
        if self.head == &Block::CAVE_VINES {
            CaveVinesLikeProperties { age, berries }.to_state_id(self.head)
        } else {
            KelpLikeProperties { age }.to_state_id(self.head)
        }
    }

    fn body_state(&self, state: BlockStateId) -> BlockStateId {
        // CaveVinesBlock.updateBodyAfterConvertedFromHead preserves berries.
        if self.head == &Block::CAVE_VINES {
            CaveVinesPlantLikeProperties {
                berries: CaveVinesLikeProperties::from_state_id(state).berries,
            }
            .to_state_id(self.body)
        } else {
            self.body.default_state.id
        }
    }

    pub(crate) fn get_state_for_placement(&self, args: &OnPlaceArgs<'_>) -> BlockStateId {
        // KelpBlock.getStateForPlacement requires full water; GrowingPlantBlock selects head/body.
        if self.head == &Block::KELP {
            let (fluid, state) =
                World::fluid_state_from_block_state(args.world.get_block_state_id(args.position));
            let full_level = Fluid::WATER.states[Fluid::WATER.default_state_index as usize].level;
            if !fluid.matches_type(&Fluid::WATER) || state.level != full_level {
                return BlockStateId::AIR;
            }
        }
        let growth_pos = args.position.offset(self.growth_direction.to_offset());
        if self.is_segment(args.world.get_block(&growth_pos)) {
            self.body.default_state.id
        } else {
            self.head_state(false)
        }
    }

    pub(crate) fn update_shape(&self, args: &GetStateForNeighborUpdateArgs<'_>) -> BlockStateId {
        // GrowingPlantHeadBlock.updateShape / GrowingPlantBodyBlock.updateShape.
        let unsupported = args.direction == self.growth_direction.opposite()
            && !self.can_survive(args.world, args.position);
        if unsupported {
            args.world
                .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        }
        if args.block == self.head {
            let growth_pos = args.position.offset(self.growth_direction.to_offset());
            if (args.direction == self.growth_direction
                && self.is_segment(Block::from_state_id(args.neighbor_state_id)))
                || (args.direction == self.growth_direction.opposite()
                    && !unsupported
                    && self.is_segment(args.world.get_block(&growth_pos)))
            {
                return self.body_state(args.state_id);
            }
        } else if args.direction == self.growth_direction
            && !self.is_segment(Block::from_state_id(args.neighbor_state_id))
        {
            let berries = self.head == &Block::CAVE_VINES
                && CaveVinesPlantLikeProperties::from_state_id(args.state_id).berries;
            return self.head_state(berries);
        }
        if self.head == &Block::KELP {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as u8,
                TickPriority::Normal,
            );
        }
        args.state_id
    }

    pub(crate) fn tick(&self, args: &OnScheduledTickArgs<'_>) {
        // GrowingPlantBlock.tick calls Level.destroyBlock with drops and retained fluid.
        if !self.can_survive(args.world.as_ref(), args.position) {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }
}

/// Retains source water when a waterlogged block or kelp segment is destroyed.
pub(crate) fn fluid_state_after_break(state: BlockStateId) -> BlockStateId {
    // Level.destroyBlock uses the old block's fluid state rather than replacing it with air.
    if state.is_waterlogged()
        || matches!(
            state.to_block_id(),
            pumpkin_data::BlockId::KELP | pumpkin_data::BlockId::KELP_PLANT
        )
    {
        Block::WATER.default_state.id
    } else {
        BlockStateId::AIR
    }
}
