use super::Entity;
use crate::world::World;
use pumpkin_data::{
    Block, BlockStateId,
    tag::{self, Taggable},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::level::SyncChunk;
use std::sync::Arc;

pub(super) struct BlockImage {
    pub pos: BlockPos,
    pub state: BlockStateId,
    chunk: SyncChunk,
}

impl BlockImage {
    fn capture(world: &World, pos: BlockPos) -> Option<Self> {
        if !World::is_valid_horizontally(pos) {
            return None;
        }
        let (coordinate, relative) = pos.chunk_and_chunk_relative_position();
        let chunk = world.level.loaded_chunks.get(&coordinate)?.clone();
        let state = if world.is_in_height_limit(pos.0.y) {
            chunk.section.get_block_absolute_y(
                relative.x as usize,
                relative.y,
                relative.z as usize,
            )?
        } else {
            // Level.getBlockState returns known void air outside dimension height.
            Block::VOID_AIR.default_state.id
        };
        Some(Self { pos, state, chunk })
    }

    fn is_current(&self, world: &World) -> bool {
        world
            .level
            .loaded_chunks
            .get(&self.pos.chunk_position())
            .is_some_and(|current| Arc::ptr_eq(current.value(), &self.chunk))
    }
}

pub(super) struct Blocks {
    pub support: BlockImage,
    pub effect: BlockImage,
}

impl Blocks {
    pub fn capture(
        world: &World,
        entity: &Entity,
        cached_support: Option<BlockPos>,
    ) -> Option<Self> {
        let position = entity.pos.load();
        // Entity.getOnPos(float): widen Java's float offset before subtracting.
        let supporting = cached_support.unwrap_or_else(|| {
            BlockPos::new(
                position.x.floor() as i32,
                (position.y - f64::from(1.0e-5f32)).floor() as i32,
                position.z.floor() as i32,
            )
        });
        let support = BlockImage::capture(world, supporting)?;
        let block = Block::from_state_id(support.state);
        let retain_y = cached_support.is_some()
            && (block.has_tag(&tag::Block::MINECRAFT_FENCES)
                || block.has_tag(&tag::Block::MINECRAFT_WALLS)
                || block.has_tag(&tag::Block::MINECRAFT_FENCE_GATES));
        let effect_pos = BlockPos::new(
            supporting.0.x,
            if retain_y {
                supporting.0.y
            } else {
                (position.y - f64::from(0.2f32)).floor() as i32
            },
            supporting.0.z,
        );
        let effect = BlockImage::capture(world, effect_pos)?;
        Some(Self { support, effect })
    }

    pub fn is_current(&self, world: &World) -> bool {
        self.support.is_current(world) && self.effect.is_current(world)
    }
}
