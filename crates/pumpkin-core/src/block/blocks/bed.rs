use std::sync::Arc;

use crate::block::entities::bed::BedBlockEntity;
use pumpkin_data::block_properties::BedPart;
use pumpkin_data::translation;
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block_from_tag;
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
mod followup2_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

#[pumpkin_block_from_tag("minecraft:beds")]
pub struct BedBlock;

impl BlockBehaviour for BedBlock {
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
        {
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

impl BedBlock {
    pub(super) fn use_bed(
        world: &Arc<World>,
        player: &Arc<Player>,
        block: &Block,
        position: &BlockPos,
    ) -> BlockActionResult {
        let Some((bed_head_pos, bed_foot_pos, state_id)) =
            super::abstract_bed::head_and_foot(world, block, *position)
        else {
            return BlockActionResult::Consume;
        };
        let bed_props = BedProperties::from_state_id(state_id);

        let rule = super::abstract_bed::bed_rule(world, block);
        // AbstractBedBlock.useWithoutItem dispatches destruction through the bed species.
        if rule.destroy_on_use {
            if let Some(message) = rule.error_message.and_then(|json| {
                serde_json::from_str::<pumpkin_util::text::TextComponent>(json).ok()
            }) {
                player.send_system_message_raw(&message, true);
            }
            if block == &Block::STRAW_BED {
                super::straw_bed::StrawBedBlock::destroy_after_use(world, bed_head_pos);
                return BlockActionResult::SuccessServer;
            }
            world.break_block(&bed_head_pos, None, BlockFlags::SKIP_DROPS);
            world.break_block(&bed_foot_pos, None, BlockFlags::SKIP_DROPS);

            world.explode_bad_respawn(bed_head_pos.to_centered_f64());

            return BlockActionResult::SuccessServer;
        }

        // AbstractBedBlock.kickVillagerOutOfBed, adapted from upstream #3693.
        if bed_props.occupied {
            let bounds =
                pumpkin_util::math::boundingbox::BoundingBox::full_block().at_pos(bed_head_pos);
            if !world.get_entities_at_box(&bounds).iter().any(|entity| {
                entity
                    .cast_any()
                    .downcast_ref::<crate::entity::passive::villager::VillagerEntity>()
                    .is_some_and(|villager| villager.wake_up_if_sleeping_at(bed_head_pos))
            }) {
                player.send_system_message_raw(
                    &pumpkin_macros::translate_cross!(
                        translation::java::BLOCK_MINECRAFT_BED_OCCUPIED,
                        translation::bedrock::TILE_BED_OCCUPIED
                    ),
                    true,
                );
            }
            return BlockActionResult::SuccessServer;
        }

        player.start_sleep_in_bed(world, block, (bed_head_pos, bed_foot_pos, state_id), rule)
    }
}

impl BedBlock {
    pub fn set_occupied(
        occupied: bool,
        world: &Arc<World>,
        block: &Block,
        block_pos: &BlockPos,
        state_id: BlockStateId,
    ) {
        // AbstractBedBlock.updateShape synchronizes OCCUPIED from the head to its partner.
        if Block::from_state_id(state_id) != block {
            return;
        }
        let Some((head, _, head_state)) =
            super::abstract_bed::head_and_foot(world, block, *block_pos)
        else {
            return;
        };
        let mut bed_props = BedProperties::from_state_id(head_state);
        bed_props.occupied = occupied;
        world.set_block_state(&head, bed_props.to_state_id(block), BlockFlags::NOTIFY_ALL);
    }
}
