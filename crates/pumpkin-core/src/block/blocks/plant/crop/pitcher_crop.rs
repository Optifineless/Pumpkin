use pumpkin_data::block_properties::{DoubleBlockHalf, PitcherCropLikeProperties};
use pumpkin_data::{Block, BlockStateId, tag, tag::Taggable};
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use crate::block::{
    BlockBehaviour, BonemealArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    RandomTickArgs,
};

pub const MAX_AGE: u8 = 4;
// PitcherCropBlock.DOUBLE_PLANT_AGE_INTERSECTION.
const DOUBLE_PLANT_AGE_INTERSECTION: u8 = 3;

#[pumpkin_block("minecraft:pitcher_crop")]
pub struct PitcherCropBlock;

impl PitcherCropBlock {
    // PitcherCropBlock.getLowerHalf resolves bone meal clicked on either half.
    fn lower_half(
        world: &crate::world::World,
        pos: pumpkin_util::math::position::BlockPos,
    ) -> Option<(
        pumpkin_util::math::position::BlockPos,
        PitcherCropLikeProperties,
    )> {
        let (block, state_id) = world.get_block_and_state_id(&pos);
        if block != &Block::PITCHER_CROP {
            return None;
        }
        let props = PitcherCropLikeProperties::from_state_id(state_id);
        if props.half == DoubleBlockHalf::Lower {
            return Some((pos, props));
        }

        let lower_pos = pos.down();
        let (lower_block, lower_state_id) = world.get_block_and_state_id(&lower_pos);
        if lower_block != &Block::PITCHER_CROP {
            return None;
        }
        let lower_props = PitcherCropLikeProperties::from_state_id(lower_state_id);
        (lower_props.half == DoubleBlockHalf::Lower).then_some((lower_pos, lower_props))
    }

    // PitcherCropBlock.canGrow is shared by bonemeal admission and its mutation.
    fn can_grow(
        world: &crate::world::World,
        lower_pos: &pumpkin_util::math::position::BlockPos,
        lower_state: PitcherCropLikeProperties,
        new_age: u8,
    ) -> bool {
        lower_state.age < MAX_AGE
            && world.get_raw_brightness(lower_pos, 0) >= 8
            && world.is_in_build_limit(lower_pos.up())
            && (!Self::is_double(new_age) || Self::can_grow_into(world, &lower_pos.up()))
    }

    fn can_grow_into(
        world: &crate::world::World,
        pos: &pumpkin_util::math::position::BlockPos,
    ) -> bool {
        let (block, state) = world.get_block_and_state_id(pos);
        state.to_state().is_air() || block == &Block::PITCHER_CROP
    }

    const fn is_double(age: u8) -> bool {
        age >= DOUBLE_PLANT_AGE_INTERSECTION
    }

    fn grow_bonemeal(
        world: &std::sync::Arc<crate::world::World>,
        lower_pos: pumpkin_util::math::position::BlockPos,
        mut lower_state: PitcherCropLikeProperties,
    ) {
        let new_age = lower_state.age.saturating_add(1).min(MAX_AGE);
        if !Self::can_grow(world, &lower_pos, lower_state, new_age) {
            return;
        }

        lower_state.age = new_age;
        // PitcherCropBlock.grow uses setBlock(..., 2) before setBlockAndUpdate on the upper half.
        world.set_block_state(
            &lower_pos,
            lower_state.to_state_id(&Block::PITCHER_CROP),
            BlockFlags::NOTIFY_LISTENERS,
        );
        if Self::is_double(new_age) {
            let mut upper_state = lower_state;
            upper_state.half = DoubleBlockHalf::Upper;
            world.set_block_state(
                &lower_pos.up(),
                upper_state.to_state_id(&Block::PITCHER_CROP),
                BlockFlags::NOTIFY_ALL,
            );
        }
    }

    #[must_use]
    pub fn can_survive(
        world: &dyn BlockAccessor,
        pos: &pumpkin_util::math::position::BlockPos,
        props: &PitcherCropLikeProperties,
    ) -> bool {
        let below = world.get_block(&pos.down());
        if props.half == DoubleBlockHalf::Lower {
            below.has_tag(&tag::Block::MINECRAFT_SUPPORTS_CROPS) || below == &Block::FARMLAND
        } else {
            below == &Block::PITCHER_CROP
        }
    }
}

impl BlockBehaviour for PitcherCropBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        let props = PitcherCropLikeProperties::from_state_id(args.state.id);
        Self::can_survive(args.block_accessor, args.position, &props)
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = PitcherCropLikeProperties::default(args.block);
        props.age = 0;
        props.half = DoubleBlockHalf::Lower;
        props.to_state_id(args.block)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let props = PitcherCropLikeProperties::from_state_id(args.state_id);
        if !Self::can_survive(args.world, args.position, &props) {
            return Block::AIR.default_state.id;
        }

        if props.half == DoubleBlockHalf::Lower && Self::is_double(props.age) {
            let above = args.world.get_block(&args.position.up());
            if above != &Block::PITCHER_CROP {
                return Block::AIR.default_state.id;
            }
        } else if props.half == DoubleBlockHalf::Upper {
            let below = args.world.get_block(&args.position.down());
            if below != &Block::PITCHER_CROP {
                return Block::AIR.default_state.id;
            }
        }

        args.state_id
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = PitcherCropLikeProperties::from_state_id(state_id);
        if props.half == DoubleBlockHalf::Lower && props.age < MAX_AGE {
            let next_age = props.age + 1;
            if next_age >= 3 {
                let above_pos = args.position.up();
                let above_state = args.world.get_block_state(&above_pos);
                if !above_state.is_air()
                    && Block::from_state_id(above_state.id) != &Block::PITCHER_CROP
                {
                    return;
                }
            }
            props.age = next_age;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_ALL,
            );
            if next_age >= 3 {
                let mut upper_props = props;
                upper_props.half = DoubleBlockHalf::Upper;
                args.world.set_block_state(
                    &args.position.up(),
                    upper_props.to_state_id(args.block),
                    BlockFlags::NOTIFY_ALL,
                );
            }
        }
    }

    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        Self::lower_half(args.world, *args.position).is_some_and(|(lower_pos, lower_state)| {
            Self::can_grow(
                args.world,
                &lower_pos,
                lower_state,
                lower_state.age.saturating_add(1).min(MAX_AGE),
            )
        })
    }

    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        if let Some((lower_pos, lower_state)) = Self::lower_half(args.world, *args.position) {
            Self::grow_bonemeal(args.world, lower_pos, lower_state);
        }
    }
}

#[cfg(test)]
#[path = "pitcher_crop_tests.rs"]
mod tests;
