use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId, entity::EntityType, world::WorldEvent,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

use crate::block::entities::skull::SkullBlockEntity;
use crate::{
    block::{
        BlockBehaviour, OnPlaceArgs, PathComputationType, PlacedArgs,
        blocks::skull_block::SkullBlock,
    },
    entity::{Entity, boss::wither::WitherEntity},
    world::World,
};

pub struct WitherPattern {
    blocks: [BlockPos; 7],
    observed: [BlockStateId; 7],
    center: BlockPos,
}

#[must_use]
pub fn find_wither_pattern(world: &Arc<World>, skull_pos: &BlockPos) -> Option<WitherPattern> {
    let is_soul_block = |block: &Block| block == &Block::SOUL_SAND || block == &Block::SOUL_SOIL;

    for dir in [BlockDirection::North, BlockDirection::West] {
        let opposite = dir.opposite();
        for center in [
            *skull_pos,
            skull_pos.offset(opposite.to_offset()),
            skull_pos.offset(dir.to_offset()),
        ] {
            let top_middle = center.down();
            let base = top_middle.down();
            let arm1 = top_middle.offset(dir.to_offset());
            let arm2 = top_middle.offset(opposite.to_offset());
            let skull1_pos = arm1.up();
            let skull2_pos = arm2.up();

            let blocks = [center, skull1_pos, skull2_pos, top_middle, arm1, arm2, base];
            let observed = blocks.map(|pos| world.get_block_state_id(&pos));
            if observed[3..]
                .iter()
                .all(|state| is_soul_block(state.to_block()))
                && blocks[..3].iter().zip(&observed[..3]).all(|(pos, state)| {
                    *pos == *skull_pos || state.to_block() == &Block::WITHER_SKELETON_SKULL
                })
            {
                return Some(WitherPattern {
                    blocks,
                    observed,
                    center: top_middle,
                });
            }
        }
    }

    None
}

fn spawn_wither(world: &Arc<World>, pattern: &WitherPattern) {
    if pattern.observed[..3]
        .iter()
        .any(|state| state.to_block() != &Block::WITHER_SKELETON_SKULL)
    {
        return;
    }
    let updates = pattern
        .blocks
        .iter()
        .zip(pattern.observed)
        .map(|(pos, observed)| (*pos, observed, BlockStateId::AIR, BlockFlags::NOTIFY_ALL))
        .collect::<Vec<_>>();
    if !world.set_block_states_from_observed(&updates) {
        return;
    }
    for pos in pattern.blocks {
        world.sync_world_event(
            WorldEvent::ParticlesDestroyBlock,
            pos,
            Block::SOUL_SAND.default_state.id.as_u16().into(),
        );
    }

    let entity = Entity::new(
        world.clone(),
        pattern.center.to_centered_f64(),
        &EntityType::WITHER,
    );
    let wither = WitherEntity::new(entity);
    wither.make_invulnerable();
    world.spawn_entity(wither);
}

#[pumpkin_block("wither_skeleton_skull")]
pub struct WitherSkeletonSkullBlock;

impl BlockBehaviour for WitherSkeletonSkullBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        SkullBlock::on_place(&SkullBlock, args)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.world.add_block_entity_from_observed(
            Arc::new(SkullBlockEntity::new(*args.position)),
            args.state_id,
        ) {
            return;
        }

        if let Some(pattern) = find_wither_pattern(args.world, args.position) {
            spawn_wither(args.world, &pattern);
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
