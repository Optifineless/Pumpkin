use crate::block::blocks::falling::FallingBlock;
use crate::block::registry::BlockActionResult;
use crate::block::{
    AttackArgs, BlockBehaviour, GetStateForNeighborUpdateArgs, NormalUseArgs, OnScheduledTickArgs,
    PathComputationType, PlacedArgs,
};
use crate::world::World;
use pumpkin_data::{BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use rand::{RngExt, rng};
use std::sync::Arc;

#[pumpkin_block("minecraft:dragon_egg")]
pub struct DragonEggBlock;

impl DragonEggBlock {
    // DragonEggBlock.getDelayAfterPlace
    const DELAY_AFTER_PLACE: u8 = 5;

    // DragonEggBlock.teleport hardcodes these radii and 1,000 candidate attempts.
    const HORIZONTAL_TELEPORT_RADIUS: i32 = 16;
    const VERTICAL_TELEPORT_RADIUS: i32 = 8;
    const MAX_TELEPORT_ATTEMPTS: usize = 1000;

    const fn pack_difference_in_position(dx: i32, dy: i32, dz: i32) -> i32 {
        // BlockUtil.packDifferenceInPosition, using DragonEggBlock's radii.
        ((dx + Self::HORIZONTAL_TELEPORT_RADIUS) & 0xff) << 16
            | ((dy + Self::VERTICAL_TELEPORT_RADIUS) & 0xff) << 8
            | ((dz + Self::HORIZONTAL_TELEPORT_RADIUS) & 0xff)
    }

    fn teleport(world: &Arc<World>, pos: &BlockPos) {
        let mut random = rng();
        let state = world.get_block_state_id(pos);
        for _ in 0..Self::MAX_TELEPORT_ATTEMPTS {
            let horizontal = Self::HORIZONTAL_TELEPORT_RADIUS;
            let vertical = Self::VERTICAL_TELEPORT_RADIUS;
            let dx = random.random_range(0..horizontal) - random.random_range(0..horizontal);
            let dy = random.random_range(0..vertical) - random.random_range(0..vertical);
            let dz = random.random_range(0..horizontal) - random.random_range(0..horizontal);
            let target = BlockPos::new(pos.0.x + dx, pos.0.y + dy, pos.0.z + dz);
            if !world.is_in_height_limit(target.0.y)
                || !world
                    .worldborder
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains(f64::from(target.0.x), f64::from(target.0.z))
                || !world.get_block_state(&target).is_air()
                || world.get_block_state(&target.down()).is_air()
            {
                continue;
            }
            // BlockUtil.packDifferenceInPosition encodes signed offsets biased by the radii.
            let packed = Self::pack_difference_in_position(dx, dy, dz);
            world.sync_world_event(
                pumpkin_data::world::WorldEvent::ParticlesDragonEggTeleport,
                *pos,
                packed,
            );
            world.set_block_state(
                &target,
                state,
                pumpkin_world::world::BlockFlags::NOTIFY_LISTENERS,
            );
            world.set_block_state(
                pos,
                pumpkin_data::Block::AIR.default_state.id,
                pumpkin_world::world::BlockFlags::NOTIFY_ALL,
            );
            return;
        }
    }
}

impl BlockBehaviour for DragonEggBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        FallingBlock::placed_with_delay(&args, Self::DELAY_AFTER_PLACE);
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        FallingBlock::get_state_for_neighbor_update_with_delay(&args, Self::DELAY_AFTER_PLACE)
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        Self::teleport(args.world, args.position);
        BlockActionResult::Success
    }

    fn attack(&self, args: AttackArgs<'_>) {
        Self::teleport(args.world, args.position);
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        FallingBlock::on_scheduled_tick(&FallingBlock, args);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

#[cfg(test)]
#[path = "block_attack_tests.rs"]
mod tests;
