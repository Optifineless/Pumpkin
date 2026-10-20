use std::{any::Any, sync::Arc};

use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_util::math::position::BlockPos;

use crate::{
    block::entities::{BlockEntity, piston::PistonBlockEntity},
    world::World,
};

#[derive(Default)]
pub(super) struct Inputs {
    states: Vec<(BlockPos, BlockStateId)>,
    pistons: Vec<PistonInput>,
}

struct PistonInput {
    position: BlockPos,
    entity: Option<Arc<dyn BlockEntity>>,
    progress: Option<u32>,
}

impl Inputs {
    pub(super) fn read(&mut self, world: &World, pos: BlockPos) -> Option<&'static BlockState> {
        if !world.is_in_height_limit(pos.0.y) {
            return Some(Block::AIR.default_state);
        }
        let state = world.get_block_state_if_loaded(&pos)?;
        // Record before native shape/context callbacks can change an earlier input.
        self.states.push((pos, state.id));
        if state.id.to_block() == &Block::MOVING_PISTON {
            let entity = world.get_block_entity(&pos);
            let progress = entity.as_ref().and_then(piston_progress);
            self.pistons.push(PistonInput {
                position: pos,
                entity,
                progress,
            });
        }
        Some(state)
    }

    pub(super) fn is_current(&self, world: &World) -> bool {
        self.states
            .iter()
            .all(|(pos, state)| world.get_block_state_id_if_loaded(pos) == Some(*state))
            && self.pistons.iter().all(|input| {
                let current = world
                    .block_entities
                    .get(&input.position.chunk_position())
                    .and_then(|entities| entities.get(&input.position).cloned());
                match (&input.entity, current) {
                    (Some(expected), Some(current)) => {
                        Arc::ptr_eq(expected, &current)
                            && piston_progress(&current) == input.progress
                    }
                    (None, None) => true,
                    _ => false,
                }
            })
    }
}

fn piston_progress(entity: &Arc<dyn BlockEntity>) -> Option<u32> {
    // Trait upcasting avoids the native as_any callback during final validation.
    let any: &dyn Any = entity.as_ref();
    any.downcast_ref::<PistonBlockEntity>()
        .map(|piston| piston.current_progress.load().to_bits())
}
