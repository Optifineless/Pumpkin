use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::CandleLikeProperties,
    entity::EntityPose,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockAccessor;
use pumpkin_world::world::BlockFlags;

use crate::block::{GetStateForNeighborUpdateArgs, OnScheduledTickArgs};
use crate::{
    block::{
        BlockIsReplacing,
        registry::BlockActionResult,
        {BlockBehaviour, CanPlaceAtArgs, CanUpdateAtArgs, OnPlaceArgs, UseWithItemArgs},
    },
    entity::EntityBase,
};

#[pumpkin_block_from_tag("minecraft:candles")]
pub struct CandleBlock;

impl BlockBehaviour for CandleBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if args.player.get_entity().pose.load() != EntityPose::Crouching
            && let BlockIsReplacing::Itself(state_id) = args.replacing
        {
            let mut properties = CandleLikeProperties::from_state_id(state_id);
            if properties.candles < 4 {
                properties.candles += 1;
            }
            return properties.to_state_id(args.block);
        }

        let mut properties = CandleLikeProperties::default(args.block);
        properties.waterlogged = args.replacing.water_source();
        properties.to_state_id(args.block)
    }

    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        // CandleBlock.useItemOn handles only empty-hand extinguishing. Candle stacking
        // falls through to BlockItem.place, which consumes the placed item.
        if !args.item_stack.is_empty() {
            return BlockActionResult::Pass;
        }
        let properties =
            CandleLikeProperties::from_state_id(args.world.get_block_state_id(args.position));
        if !properties.lit
            || !args
                .player
                .abilities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .allow_modify_world
        {
            return BlockActionResult::Pass;
        }
        extinguish(args.world, args.block, args.position);
        BlockActionResult::Success
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_at(args.block_accessor, args.position)
    }

    fn can_update_at(&self, args: CanUpdateAtArgs<'_>) -> bool {
        let b = BlockAccessor::get_block(args.world, args.position);
        args.player.get_entity().pose.load() != EntityPose::Crouching
            && CandleLikeProperties::from_state_id(args.state_id).candles != 4
            && args.block.id == b.id
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if !can_place_at(args.world.as_ref(), args.position) {
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
}

fn can_place_at(block_accessor: &dyn BlockAccessor, position: &BlockPos) -> bool {
    let (support_block, state) = block_accessor.get_block_and_state(&position.down());
    !support_block.is_waterlogged(state.id) && state.is_center_solid(BlockDirection::Up)
}

/// Extinguishes a candle or candle cake, with AbstractCandleBlock.extinguish effects.
/// The block must be a candle or candle cake at the supplied position.
pub(crate) fn extinguish(
    world: &std::sync::Arc<crate::world::World>,
    block: &Block,
    position: &BlockPos,
) {
    let state = world.get_block_state_id(position);
    if block.has_tag(&tag::Block::MINECRAFT_CANDLES) {
        let mut properties = CandleLikeProperties::from_state_id(state);
        properties.lit = false;
        world.set_block_state(
            position,
            properties.to_state_id(block),
            BlockFlags::NOTIFY_ALL,
        );
    } else {
        let properties = pumpkin_data::block_properties::RedstoneOreLikeProperties { lit: false };
        world.set_block_state(
            position,
            properties.to_state_id(block),
            BlockFlags::NOTIFY_ALL,
        );
    }
    // AbstractCandleBlock.extinguish uses Level.addParticle, a no-op on the server.
    world.play_sound(
        Sound::BlockCandleExtinguish,
        SoundCategory::Blocks,
        &position.to_centered_f64(),
    );
    world.emit_game_event(GameEvent::BlockChange.name(), position.to_centered_f64());
}
