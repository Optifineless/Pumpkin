use std::sync::Arc;

use pumpkin_data::block_properties::SuspiciousSandLikeProperties;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockId, BlockStateId};
use pumpkin_world::world::BlockFlags;

use crate::block::entities::brushable_block::BrushableBlockBlockEntity;
use crate::block::{BlockBehaviour, BlockMetadata, BrokenArgs, OnPlaceArgs, PlacedArgs};

pub struct BrushableBlock;

impl BlockMetadata for BrushableBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::SUSPICIOUS_SAND, BlockId::SUSPICIOUS_GRAVEL].into()
    }
}

impl BrushableBlock {
    pub fn brush(
        world: &Arc<crate::world::World>,
        pos: &pumpkin_util::math::position::BlockPos,
        block: &Block,
    ) {
        let state = world.get_block_state(pos);
        if Block::from_state_id(state.id) != block {
            return;
        }
        let mut props = SuspiciousSandLikeProperties::from_state_id(state.id);
        let Some(be) = world.get_block_entity(pos) else {
            return;
        };
        let Some(brush_be) = be.as_any().downcast_ref::<BrushableBlockBlockEntity>() else {
            return;
        };
        let is_gravel = block.id == BlockId::SUSPICIOUS_GRAVEL;
        let hits = {
            let hits = brush_be
                .hits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (*hits).max(i32::from(props.dusted)).saturating_add(1)
        };
        let complete = hits >= 4;
        props.dusted = (hits as u8).min(3);
        let new_state = if complete {
            if is_gravel {
                Block::GRAVEL.default_state.id
            } else {
                Block::SAND.default_state.id
            }
        } else {
            props.to_state_id(block)
        };
        if !world.set_block_state_from_observed(pos, state.id, new_state, BlockFlags::NOTIFY_ALL) {
            return;
        }
        {
            let mut current = brush_be
                .hits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *current = (*current).max(hits);
        };
        if complete {
            let item = brush_be
                .item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(item) = item {
                world.drop_stack(pos, item);
            }
        }
        let sound = match (is_gravel, complete) {
            (true, true) => Sound::ItemBrushBrushingGravelComplete,
            (false, true) => Sound::ItemBrushBrushingSandComplete,
            (true, false) => Sound::ItemBrushBrushingGravel,
            (false, false) => Sound::ItemBrushBrushingSand,
        };
        world.play_sound(sound, SoundCategory::Blocks, &pos.to_f64());
    }
}

impl BlockBehaviour for BrushableBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let props = SuspiciousSandLikeProperties::default(args.block);
        props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        let entity = BrushableBlockBlockEntity::new(*args.position);
        args.world
            .add_block_entity_from_observed(Arc::new(entity), args.state_id);
    }

    fn broken(&self, args: BrokenArgs<'_>) {
        if let Some(be) = args.world.get_block_entity(args.position)
            && let Some(brush_be) = be.as_any().downcast_ref::<BrushableBlockBlockEntity>()
            && let Some(contained) = brush_be
                .item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        {
            args.world.drop_stack(args.position, contained);
        }
    }
}
