use crate::world::World;
use pumpkin_data::block_properties::{DoubleBlockHalf, PitcherCropLikeProperties};
use pumpkin_data::{Block, BlockStateId, tag, tag::Taggable};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::{BlockAccessor, BlockFlags};
use rand::RngExt;

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
    // PitcherCropBlock.getLowerHalf / canGrow: every growth entry point uses this gate.
    fn lower_half(world: &World, pos: BlockPos) -> Option<(BlockPos, PitcherCropLikeProperties)> {
        let (block, state) = world.get_block_and_state_id(&pos);
        if block != &Block::PITCHER_CROP {
            return None;
        }
        let props = PitcherCropLikeProperties::from_state_id(state);
        if props.half == DoubleBlockHalf::Lower {
            return Some((pos, props));
        }
        let (block, state) = world.get_block_and_state_id(&pos.down());
        if block != &Block::PITCHER_CROP {
            return None;
        }
        let props = PitcherCropLikeProperties::from_state_id(state);
        (props.half == DoubleBlockHalf::Lower).then_some((pos.down(), props))
    }

    fn can_grow(world: &World, pos: BlockPos, props: PitcherCropLikeProperties) -> bool {
        props.age < MAX_AGE
            && world.get_raw_brightness(&pos, 0) >= 8
            && pos.up().0.y >= world.dimension.min_y
            && pos.up().0.y <= world.get_top_y()
            && (props.age + 1 < DOUBLE_PLANT_AGE_INTERSECTION
                || world.get_block_state(&pos.up()).is_air()
                || world.get_block(&pos.up()) == &Block::PITCHER_CROP)
    }

    fn grow(world: &std::sync::Arc<World>, pos: BlockPos, mut props: PitcherCropLikeProperties) {
        // PitcherCropBlock.grow writes the lower half before updating the upper half.
        if !Self::can_grow(world, pos, props) {
            return;
        }
        props.age += 1;
        world.set_block_state(
            &pos,
            props.to_state_id(&Block::PITCHER_CROP),
            BlockFlags::NOTIFY_LISTENERS,
        );
        if props.age >= DOUBLE_PLANT_AGE_INTERSECTION {
            props.half = DoubleBlockHalf::Upper;
            world.set_block_state(
                &pos.up(),
                props.to_state_id(&Block::PITCHER_CROP),
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
                && PitcherCropLikeProperties::from_state_id(world.get_block_state_id(&pos.down()))
                    .half
                    == DoubleBlockHalf::Lower
        }
    }
}

impl BlockBehaviour for PitcherCropBlock {
    fn player_will_destroy(&self, args: crate::block::PlayerWillDestroyArgs<'_>) {
        super::super::double_plant::player_will_destroy(args);
    }

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
        // PitcherCropBlock.updateShape delegates mature crops to DoublePlantBlock.updateShape.
        if props.age >= DOUBLE_PLANT_AGE_INTERSECTION
            && let Some(state) = super::super::double_plant::partner_update(&args)
        {
            return state;
        }
        if !Self::can_survive(args.world, args.position, &props) {
            return Block::AIR.default_state.id;
        }

        if props.half == DoubleBlockHalf::Lower && props.age >= DOUBLE_PLANT_AGE_INTERSECTION {
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
        // PitcherCropBlock.randomTick uses CropBlock.getGrowthSpeed.
        let props =
            PitcherCropLikeProperties::from_state_id(args.world.get_block_state_id(args.position));
        if props.half == DoubleBlockHalf::Lower {
            let speed = super::get_available_moisture(args.world, args.position, args.block);
            if rand::rng().random_range(0..=(25.0 / speed) as i32) == 0 {
                Self::grow(args.world, *args.position, props);
            }
        }
    }

    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        Self::lower_half(args.world, *args.position)
            .is_some_and(|(pos, props)| Self::can_grow(args.world, pos, props))
    }

    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        if let Some((pos, props)) = Self::lower_half(args.world, *args.position) {
            Self::grow(args.world, pos, props);
        }
    }
}
