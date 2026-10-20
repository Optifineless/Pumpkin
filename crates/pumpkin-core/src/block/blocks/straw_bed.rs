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
    PathComputationType, PlacedArgs, PlayerWillDestroyArgs,
};
use crate::entity::{EntityBase, player::Player};
use crate::world::World;

type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;

#[pumpkin_block("minecraft:straw_bed")]
pub struct StrawBedBlock;

impl BlockBehaviour for StrawBedBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        super::bed::BedBlock::can_place_bed_at(&args)
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
        let bed_entity = BedBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(bed_entity));

        let mut bed_head_props = BedProperties::default(args.block);
        bed_head_props.facing = BedProperties::from_state_id(args.state_id).facing;
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

    fn player_will_destroy(&self, args: PlayerWillDestroyArgs<'_>) {
        super::bed_pair::player_will_destroy(args);
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        super::bed_pair::update_shape(&args)
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
        let mut foot_props = bed_props;
        foot_props.part = BedPart::Foot;
        let mut head_props = bed_props;
        head_props.part = BedPart::Head;
        let head_state = head_props.to_state_id(&Block::STRAW_BED);
        // Do not consume an invalid pair left by a PlayerBedLeaveEvent callback.
        if world.get_block_state_id(&head_pos) != head_state
            || world.get_block_state_id(&foot_pos) != foot_props.to_state_id(&Block::STRAW_BED)
        {
            return;
        }
        // StrawBedBlock.destroyBed removes the head, then explicitly admits its captured partner.
        if world
            .break_straw_bed_half(
                &head_pos,
                head_state,
                BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL,
                crate::world::neighbor_updater::UPDATE_LIMIT,
            )
            .is_some()
        {
            world.play_block_sound(
                Sound::BlockStrawBedBreakLeave,
                SoundCategory::Blocks,
                head_pos,
            );
        }
    }

    fn use_bed(
        world: &Arc<World>,
        player: &Arc<Player>,
        block: &Block,
        position: &BlockPos,
    ) -> BlockActionResult {
        let Some((bed_head_pos, bed_foot_pos, state_id)) = world.bed_pair_for_use(block, *position)
        else {
            return BlockActionResult::Consume;
        };
        // StrawBedBlock retains its own destruction behavior and environment rule.
        if world.dimension.straw_bed_rule.destroy_on_use {
            if let Some(message) = world.dimension.straw_bed_rule.error_message
                && let Ok(message) = serde_json::from_str(message)
            {
                player.send_system_message_raw(&message, true);
            }
            Self::destroy_after_use(world, bed_head_pos);
            return BlockActionResult::SuccessServer;
        }

        super::bed::BedBlock::start_sleep_in_bed(
            world,
            player,
            (bed_head_pos, bed_foot_pos, state_id),
            world.dimension.straw_bed_rule,
            pumpkin_data::statistic::CustomStatistic::SleepInStrawBed,
        )
    }
}
