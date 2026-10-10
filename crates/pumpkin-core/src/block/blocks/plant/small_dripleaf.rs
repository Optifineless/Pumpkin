use crate::block::blocks::plant::PlantBlockBase;
use crate::block::{
    BlockBehaviour, BonemealArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    PlacedArgs,
};
use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::{DoubleBlockHalf, SmallDripleafLikeProperties};
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, tag};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

#[pumpkin_block("minecraft:small_dripleaf")]
pub struct SmallDripleafBlock;

impl BlockBehaviour for SmallDripleafBlock {
    fn player_will_destroy(&self, args: crate::block::PlayerWillDestroyArgs<'_>) {
        super::double_plant::player_will_destroy(args);
    }

    fn is_valid_bonemeal_target(&self, _args: BonemealArgs<'_>) -> bool {
        true
    }
    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        // SmallDripleafBlock.performBonemeal replaces the upper half with its water before growing.
        let props = SmallDripleafLikeProperties::from_state_id(args.state_id);
        let lower = if props.half == DoubleBlockHalf::Lower {
            *args.position
        } else {
            args.position.down()
        };
        let upper = args.world.get_block_state(&lower.up());
        let replacement = if super::double_plant::water_source(upper) {
            &Block::WATER
        } else {
            &Block::AIR
        };
        args.world.set_block_state(
            &lower.up(),
            replacement.default_state.id,
            BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        super::big_dripleaf::place_with_random_height(args.world, lower, props.facing);
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
            && args
                .block_accessor
                .get_block_state(&args.position.up())
                .replaceable()
            && args
                .world
                .is_none_or(|world| args.position.0.y < world.get_top_y())
    }
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let facing = args
            .player
            .living_entity
            .entity
            .get_horizontal_facing()
            .opposite();
        let mut small_dripleaf_props = SmallDripleafLikeProperties::default(args.block);

        small_dripleaf_props.facing = facing;
        small_dripleaf_props.waterlogged = args.replacing.water_source();
        small_dripleaf_props.half = DoubleBlockHalf::Lower;

        small_dripleaf_props.to_state_id(args.block)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if let Some(state) = super::double_plant::partner_update(&args) {
            return state;
        }
        let props = SmallDripleafLikeProperties::from_state_id(args.state_id);
        if props.half == DoubleBlockHalf::Lower
            && args.direction == pumpkin_data::BlockDirection::Down
            && !supports_small_dripleaf(
                args.world.get_block(&args.position.down()),
                props.waterlogged,
            )
        {
            return Block::AIR.default_state.id;
        }
        args.state_id
    }
    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let lower_small_dripleaf_props =
                SmallDripleafLikeProperties::from_state_id(args.state_id);
            // DoublePlantBlock.setPlacedBy is the lower-half item placement callback.
            if lower_small_dripleaf_props.half != DoubleBlockHalf::Lower {
                return;
            }

            let mut upper_small_dripleaf_props =
                SmallDripleafLikeProperties::default(&Block::SMALL_DRIPLEAF);

            let upper_block = args.world.get_block(&args.position.up());
            upper_small_dripleaf_props.facing = lower_small_dripleaf_props.facing;
            upper_small_dripleaf_props.waterlogged = upper_block == &Block::WATER;
            upper_small_dripleaf_props.half = DoubleBlockHalf::Upper;

            args.world.set_block_state(
                &args.position.up(),
                upper_small_dripleaf_props.to_state_id(&Block::SMALL_DRIPLEAF),
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
            );
        }
    }
}
impl PlantBlockBase for SmallDripleafBlock {
    fn can_plant_on_top(&self, block_accessor: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        // SmallDripleafBlock.mayPlaceOn uses the water at the lower half's position.
        supports_small_dripleaf(
            block_accessor.get_block(pos),
            super::double_plant::water_source(block_accessor.get_block_state(&pos.up())),
        )
    }
}
fn supports_small_dripleaf(support_block: &Block, underwater: bool) -> bool {
    if support_block.has_tag(&tag::Block::MINECRAFT_SUPPORTS_SMALL_DRIPLEAF) {
        return true;
    }
    underwater && support_block.has_tag(&tag::Block::MINECRAFT_SUPPORTS_VEGETATION)
}
