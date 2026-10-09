use std::sync::Arc;

use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs, PathComputationType, RandomTickArgs,
};
use crate::world::World;
use pumpkin_data::block_properties::FarmlandLikeProperties;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockAccessor;
use pumpkin_world::world::BlockFlags;

type FarmlandProperties = FarmlandLikeProperties;

#[pumpkin_block("minecraft:farmland")]
pub struct FarmlandBlock;

// FarmlandBlock.MAX_MOISTURE.
const MAX_MOISTURE: u8 = 7;

impl BlockBehaviour for FarmlandBlock {
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        // FarmlandBlock.tick rechecks survival after the delayed update.
        if !can_place_at(args.world.as_ref(), args.position) {
            // TODO: push up entities
            args.world.set_block_state(
                args.position,
                Block::DIRT.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
        }
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if !can_place_at(args.world, args.position) {
            return Block::DIRT.default_state.id;
        }
        args.block.default_state.id
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.direction == BlockDirection::Up && !can_place_at(args.world, args.position) {
            args.world
                .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        }
        args.state_id
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_at(args.block_accessor, args.position)
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        // FarmlandBlock.randomTick: rain/water only change moisture below the maximum.
        let props = FarmlandProperties::from_state_id(args.world.get_block_state_id(args.position));
        if is_water_nearby(args.world, args.position)
            || args.world.is_raining_at(&args.position.up())
        {
            if props.moisture < MAX_MOISTURE {
                change_moisture(&args, props, i32::from(MAX_MOISTURE));
            }
        } else if props.moisture > 0 {
            let moisture = i32::from(props.moisture) - 1;
            change_moisture(&args, props, moisture);
        } else if !should_maintain_farmland(args.world.as_ref(), args.position) {
            // TODO: push up entities
            args.world.set_block_state(
                args.position,
                Block::DIRT.default_state.id,
                BlockFlags::NOTIFY_NEIGHBORS,
            );
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

// FarmlandBlock.canSurvive / shouldMaintainFarmland.
fn can_place_at(world: &dyn BlockAccessor, block_pos: &BlockPos) -> bool {
    !world.get_block_state(&block_pos.up()).is_solid() || should_maintain_farmland(world, block_pos)
}

fn should_maintain_farmland(world: &dyn BlockAccessor, block_pos: &BlockPos) -> bool {
    world
        .get_block(&block_pos.up())
        .has_tag(&tag::Block::MINECRAFT_MAINTAINS_FARMLAND)
}

fn change_moisture(args: &RandomTickArgs<'_>, mut props: FarmlandProperties, mut moisture: i32) {
    if let Some(server) = args.world.server.upgrade() {
        let mut event =
            crate::plugin::api::events::block::moisture_change::MoistureChangeEvent::new(
                *args.position,
                args.world.clone(),
                moisture,
            );
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return;
        }
        moisture = event.new_moisture;
    }
    props.moisture = moisture.clamp(0, i32::from(MAX_MOISTURE)) as u8;
    // FarmlandBlock.randomTick uses UPDATE_CLIENTS for moisture changes.
    args.world.set_block_state(
        args.position,
        props.to_state_id(args.block),
        BlockFlags::NOTIFY_LISTENERS,
    );
}

fn is_water_nearby(world: &Arc<World>, block_pos: &BlockPos) -> bool {
    for dx in -4..=4 {
        for dy in 0..=1 {
            for dz in -4..=4 {
                let check_pos = block_pos.offset(Vector3 {
                    x: dx,
                    y: dy,
                    z: dz,
                });
                //TODO this should use tag water. It does not seem to work rn.
                if world.get_block(&check_pos) == &Block::WATER {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "farmland_tests.rs"]
mod tests;
