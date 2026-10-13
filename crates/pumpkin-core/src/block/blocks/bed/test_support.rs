use std::sync::Arc;

use crate::{
    block::{BlockHitResult, registry::BlockActionResult},
    entity::player::Player,
    world::World,
};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

/// Uses a bed through the block registry entry point used by the Java use-on packet handler.
pub fn use_bed(
    world: &Arc<World>,
    player: &Arc<Player>,
    block: &Block,
    position: &BlockPos,
) -> BlockActionResult {
    let server = world.server.upgrade().unwrap();
    let face = BlockDirection::Up;
    let cursor = Vector3::new(0.5, 1.0, 0.5);
    server.block_registry.on_use(
        block,
        player,
        position,
        &BlockHitResult {
            face: &face,
            cursor_pos: &cursor,
        },
        &server,
        world,
    )
}
