use std::sync::Arc;

use crate::block::entities::bed::BedBlockEntity;
use pumpkin_data::block_properties::BedPart;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::block::OnLandedUponArgs;
use crate::block::UpdateEntityMovementAfterFallOnArgs;
use crate::block::bounce_entity_after_fall;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, NormalUseArgs, OnPlaceArgs,
    PathComputationType, PlacedArgs,
};
use crate::entity::{EntityBase, player::Player};
use crate::world::World;

type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;

#[cfg(test)]
mod tests;

#[pumpkin_block("minecraft:straw_bed")]
pub struct StrawBedBlock;

impl BlockBehaviour for StrawBedBlock {
    fn player_will_destroy(&self, args: crate::block::PlayerWillDestroyArgs<'_>) {
        super::abstract_bed::player_will_destroy(args);
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        if let Some(player) = args.player {
            let facing = player.get_entity().get_horizontal_facing();
            return args
                .block_accessor
                .get_block_state(args.position)
                .replaceable()
                && args
                    .block_accessor
                    .get_block_state(&args.position.offset(facing.to_offset()))
                    .replaceable()
                && super::abstract_bed::head_inside_border(
                    args.world,
                    args.position.offset(facing.to_offset()),
                );
        }
        false
    }

    fn on_landed_upon(&self, args: OnLandedUponArgs<'_>) {
        if let Some(living) = args.entity.get_living_entity() {
            living.handle_fall_damage(args.entity, args.fall_distance * 0.5, 1.0);
        }
    }

    fn update_entity_movement_after_fall_on(&self, args: UpdateEntityMovementAfterFallOnArgs<'_>) {
        bounce_entity_after_fall(args.entity, 0.66);
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut bed_props = BedProperties::default(args.block);
        bed_props.facing = args.player.get_entity().get_horizontal_facing();
        bed_props.part = BedPart::Foot;
        bed_props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        // AbstractBedBlock.setPlacedBy applies only to placement of the foot.
        if BedProperties::from_state_id(args.state_id).part != BedPart::Foot {
            return;
        }
        let bed_entity = BedBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(bed_entity));

        let mut bed_head_props = BedProperties::from_state_id(args.state_id);
        bed_head_props.part = BedPart::Head;

        let bed_head_pos = args.position.offset(bed_head_props.facing.to_offset());
        args.world.set_block_state(
            &bed_head_pos,
            bed_head_props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );

        let bed_head_entity = BedBlockEntity::new(bed_head_pos);
        args.world.add_block_entity(Arc::new(bed_head_entity));
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        super::abstract_bed::update_shape(&args)
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        Self::use_bed(args.world, args.player, args.block, args.position)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl StrawBedBlock {
    pub fn destroy_after_use(world: &Arc<World>, bed_head_pos: BlockPos) {
        let (block, state) = world.get_block_and_state_id(&bed_head_pos);
        if block != &Block::STRAW_BED {
            return;
        }
        let bed_props = BedProperties::from_state_id(state);
        let (head_pos, foot_pos) = if bed_props.part == BedPart::Head {
            (
                bed_head_pos,
                bed_head_pos.offset(bed_props.facing.opposite().to_offset()),
            )
        } else {
            (
                bed_head_pos.offset(bed_props.facing.to_offset()),
                bed_head_pos,
            )
        };
        world.play_block_sound(
            Sound::BlockStrawBedBreakLeave,
            SoundCategory::Blocks,
            head_pos,
        );
        world.break_block(
            &head_pos,
            None,
            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL,
        );
        world.break_block(
            &foot_pos,
            None,
            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL,
        );
    }

    fn use_bed(
        world: &Arc<World>,
        player: &Arc<Player>,
        block: &Block,
        position: &BlockPos,
    ) -> BlockActionResult {
        // AbstractBedBlock.useWithoutItem shares admission for both bed species.
        super::bed::BedBlock::use_bed(world, player, block, position)
    }
}
