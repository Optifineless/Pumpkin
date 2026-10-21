use std::sync::Arc;

use pumpkin_data::block_properties::{NetherWartLikeProperties, blocks_movement};
use pumpkin_data::dimension::Dimension;
use pumpkin_data::{Block, BlockDirection, BlockId, BlockState, BlockStateId, Enchantment};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::block::{
    BlockBehaviour, BlockMetadata, BrokenArgs, OnNeighborUpdateArgs, OnScheduledTickArgs,
    PlacedArgs, RandomTickArgs,
};
use crate::world::World;

/// Melts ice at the given position into water (or removes it where water evaporates).
pub fn melt(world: &Arc<World>, position: &BlockPos) {
    let observed = world.get_block_state_id(position);
    if matches!(observed.to_block_id(), BlockId::ICE | BlockId::FROSTED_ICE) {
        melt_from_observed(world, position, observed);
    }
}

fn melt_from_observed(world: &Arc<World>, position: &BlockPos, observed: BlockStateId) -> bool {
    let next = if world.dimension.water_evaporates {
        BlockStateId::AIR
    } else {
        Block::WATER.default_state.id
    };
    world.set_block_state_from_observed(position, observed, next, BlockFlags::NOTIFY_ALL)
}

/// Returns whether the frosted ice block at `pos` has fewer adjacent frosted ice blocks than `limit`.
pub fn fewer_neighbors_than(world: &World, pos: &BlockPos, limit: usize) -> bool {
    let mut count = 0;
    for dir in BlockDirection::all() {
        let neighbor_pos = pos.offset(dir.to_offset());
        if world.get_block(&neighbor_pos) == &Block::FROSTED_ICE {
            count += 1;
            if count >= limit {
                return false;
            }
        }
    }
    true
}

fn slightly_melt(world: &Arc<World>, pos: &BlockPos, block: &Block, age: u8) -> Option<bool> {
    if age < 3 {
        let mut new_props = NetherWartLikeProperties::default(block);
        new_props.r#age = age + 1;
        let mut old_props = new_props;
        old_props.r#age = age;
        world
            .set_block_state_from_observed(
                pos,
                old_props.to_state_id(block),
                new_props.to_state_id(block),
                BlockFlags::NOTIFY_ALL,
            )
            .then_some(false)
    } else {
        let mut props = NetherWartLikeProperties::default(block);
        props.r#age = age;
        melt_from_observed(world, pos, props.to_state_id(block)).then_some(true)
    }
}

pub struct IceBlock;

impl BlockMetadata for IceBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::ICE].into()
    }
}

impl BlockBehaviour for IceBlock {
    fn broken(&self, args: BrokenArgs<'_>) {
        {
            let held_item = args.player.inventory().held_item();
            let has_silk_touch = held_item.get_enchantment_level(&Enchantment::SILK_TOUCH) > 0;
            if !has_silk_touch {
                if args.world.dimension.water_evaporates {
                    args.world.set_block_state_from_observed(
                        args.position,
                        super::growing_plant::fluid_state_after_break(args.state.id),
                        BlockStateId::AIR,
                        BlockFlags::NOTIFY_ALL,
                    );
                    return;
                }

                let below_pos = args.position.down();
                let (below_block, below_state_id) = args.world.get_block_and_state_id(&below_pos);
                let below_state = BlockState::from_id(below_state_id);
                if blocks_movement(below_state, below_block.id)
                    || below_state.is_liquid()
                    || below_state.is_solid()
                {
                    args.world.set_block_state_from_observed(
                        args.position,
                        super::growing_plant::fluid_state_after_break(args.state.id),
                        Block::WATER.default_state.id,
                        BlockFlags::NOTIFY_ALL,
                    );
                }
            }
        }
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let state = args.world.get_block_state(args.position);
        if state.id.to_block() != args.block {
            return;
        }
        let block_light = args.world.get_block_light_level(args.position).unwrap_or(0);
        if block_light > (11u8.saturating_sub(state.opacity)) {
            melt_from_observed(args.world, args.position, state.id);
        }
    }
}

pub struct FrostedIceBlock;

impl BlockMetadata for FrostedIceBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::FROSTED_ICE].into()
    }
}

impl BlockBehaviour for FrostedIceBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let delay = rand::rng().random_range(60..=120);
            args.world
                .schedule_block_tick(args.block, *args.position, delay, TickPriority::Normal);
        }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let should_check_melt = rand::rng().random_range(0..3) == 0
            || fewer_neighbors_than(args.world, args.position, 4);

        if should_check_melt {
            let state_id = args.world.get_block_state_id(args.position);
            if state_id.to_block() != args.block {
                return;
            }
            let state = state_id.to_state();
            let props = NetherWartLikeProperties::from_state_id(state_id);
            let age = props.r#age;

            let brightness = if args.world.dimension == Dimension::THE_END {
                args.world.get_block_light_level(args.position).unwrap_or(0)
            } else {
                args.world.get_max_local_raw_brightness(args.position)
            };

            let threshold = 11u8.saturating_sub(age).saturating_sub(state.opacity);
            let melted = if brightness > threshold {
                let Some(melted) = slightly_melt(args.world, args.position, args.block, age) else {
                    return;
                };
                melted
            } else {
                false
            };
            if melted {
                for dir in BlockDirection::all() {
                    let neighbor_pos = args.position.offset(dir.to_offset());
                    let (neighbor_block, neighbor_state_id) =
                        args.world.get_block_and_state_id(&neighbor_pos);
                    if neighbor_block == &Block::FROSTED_ICE {
                        let neighbor_props =
                            NetherWartLikeProperties::from_state_id(neighbor_state_id);
                        if slightly_melt(
                            args.world,
                            &neighbor_pos,
                            neighbor_block,
                            neighbor_props.r#age,
                        ) == Some(false)
                        {
                            let delay = rand::rng().random_range(20..=40);
                            args.world.schedule_block_tick(
                                neighbor_block,
                                neighbor_pos,
                                delay,
                                TickPriority::Normal,
                            );
                        }
                    }
                }
                return;
            }
        }

        let delay = rand::rng().random_range(20..=40);
        args.world
            .schedule_block_tick(args.block, *args.position, delay, TickPriority::Normal);
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let observed = args.world.get_block_state_id(args.position);
        if observed.to_block() != args.block {
            return;
        }
        if args.source_block == &Block::FROSTED_ICE
            && fewer_neighbors_than(args.world, args.position, 2)
        {
            melt_from_observed(args.world, args.position, observed);
        }
    }

    fn broken(&self, args: BrokenArgs<'_>) {
        IceBlock.broken(args);
    }
}
