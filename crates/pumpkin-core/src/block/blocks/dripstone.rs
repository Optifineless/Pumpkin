use std::sync::Arc;

use crate::{
    block::{
        BlockBehaviour, BrokenArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
        OnLandedUponArgs, OnPlaceArgs, OnScheduledTickArgs, PathComputationType, PlacedArgs,
    },
    entity::player::Player,
    world::World,
};
use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId,
    block_properties::{PointedDripstoneLikeProperties, SpeleothemThickness, VerticalDirection},
    damage::DamageType,
    tag::{self, Taggable},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

#[pumpkin_block("minecraft:pointed_dripstone")]
pub struct DripstoneBlock;

impl DripstoneBlock {
    /// Returns the enhanced fall distance for an upward tip, given a pointed-dripstone state.
    pub(crate) fn stalagmite_fall_distance(state: BlockStateId, distance: f64) -> Option<f64> {
        // PointedDripstoneBlock.fallOn also adjusts distance for FallingBlockEntity's override.
        const STALAGMITE_FALL_DISTANCE_OFFSET: f64 = 2.5;
        let props = PointedDripstoneLikeProperties::from_state_id(state);
        (props.vertical_direction == VerticalDirection::Up
            && props.thickness == SpeleothemThickness::Tip)
            .then_some(distance + STALAGMITE_FALL_DISTANCE_OFFSET)
    }
}

impl BlockBehaviour for DripstoneBlock {
    fn on_landed_upon(&self, args: OnLandedUponArgs<'_>) {
        // PointedDripstoneBlock.fallOn; adapted from upstream #3416 to vanilla 26.3.
        const STALAGMITE_FALL_DAMAGE_MODIFIER: f32 = 2.0;
        let Some(living) = args.entity.get_living_entity() else {
            return;
        };
        if let Some(distance) = Self::stalagmite_fall_distance(
            args.world.get_block_state_id(args.position),
            f64::from(args.fall_distance),
        ) {
            living.handle_fall_damage_from(
                args.entity,
                distance as f32,
                STALAGMITE_FALL_DAMAGE_MODIFIER,
                DamageType::STALAGMITE,
            );
        } else {
            living.handle_fall_damage(args.entity, args.fall_distance, 1.0);
        }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state = args.world.get_block_state_id(args.position);
        let props = PointedDripstoneLikeProperties::from_state_id(state);
        // SpeleothemBlock.tick rechecks stalagmite survival; a scheduled stalactite still falls.
        if props.vertical_direction == VerticalDirection::Up
            && !can_survive(
                args.world.as_ref(),
                args.position,
                args.block,
                props.vertical_direction,
            )
        {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        } else {
            spawn_falling_stalactite(args.world, *args.position);
        }
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        if args.direction.is_none() {
            let props = PointedDripstoneLikeProperties::from_state_id(args.state.id);
            return can_survive(
                args.block_accessor,
                args.position,
                args.block,
                props.vertical_direction,
            );
        }
        can_place_at_pos(
            args.block_accessor,
            args.position,
            args.direction,
            args.player,
        )
    }
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut dripstone_props = PointedDripstoneLikeProperties::default(args.block);
        dripstone_props.waterlogged = args.replacing.water_source();
        let Some(support_block_ver_dir) = get_support_block_vertical_direction(
            args.world,
            args.position,
            Some(args.direction),
            Some(args.player),
        ) else {
            //this shouldn't happen
            return Block::AIR.default_state.id;
        };

        dripstone_props.vertical_direction = flip_dir(support_block_ver_dir);
        dripstone_props.to_state_id(&Block::POINTED_DRIPSTONE)
    }
    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let (len, vertical_dir) = get_stalagmite_or_stalactice_len_and_dir_from_tip_pos(
                args.world,
                args.position,
                args.state_id,
            );
            match vertical_dir {
                VerticalDirection::Up => {
                    update_stalagmite(args.world, len, args.position);
                }
                VerticalDirection::Down => {
                    update_stalactite(args.world, len, args.position);
                }
            }
        }
    }
    fn broken(&self, args: BrokenArgs<'_>) {
        {
            let broken_dripstone_props =
                PointedDripstoneLikeProperties::from_state_id(args.state.id);
            let new_tip_pos = match broken_dripstone_props.vertical_direction {
                VerticalDirection::Up => args.position.down(),
                VerticalDirection::Down => args.position.up(),
            };

            let (len, vertical_dir) = get_stalagmite_or_stalactice_len_and_dir_from_tip_pos(
                args.world,
                &new_tip_pos,
                args.state.id,
            );
            match vertical_dir {
                VerticalDirection::Up => {
                    update_stalagmite(args.world, len, &new_tip_pos);
                }
                VerticalDirection::Down => {
                    update_stalactite(args.world, len, &new_tip_pos);
                }
            }
        }
    }
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let mut dripstone_props = PointedDripstoneLikeProperties::from_state_id(args.state_id);
        // SpeleothemBlock.updateShape retains the state while the delayed fall/break is pending.
        if !matches!(args.direction, BlockDirection::Up | BlockDirection::Down) {
            return args.state_id;
        }
        if dripstone_props.vertical_direction == VerticalDirection::Down
            && args
                .world
                .level
                .is_block_tick_scheduled(args.position, args.block)
        {
            return args.state_id;
        }
        let support_direction = match dripstone_props.vertical_direction {
            VerticalDirection::Up => BlockDirection::Down,
            VerticalDirection::Down => BlockDirection::Up,
        };
        if args.direction == support_direction
            && !can_survive(
                args.world,
                args.position,
                args.block,
                dripstone_props.vertical_direction,
            )
        {
            let delay = if dripstone_props.vertical_direction == VerticalDirection::Down {
                2
            } else {
                1
            };
            args.world
                .schedule_block_tick(args.block, *args.position, delay, TickPriority::Normal);
            return args.state_id;
        }
        if dripstone_props.thickness != SpeleothemThickness::TipMerge {
            return args.state_id;
        }
        match dripstone_props.vertical_direction {
            VerticalDirection::Up => {
                let block_above = args.world.get_block(&args.position.up());
                if block_above != &Block::POINTED_DRIPSTONE {
                    dripstone_props.thickness = SpeleothemThickness::Tip;
                    return dripstone_props.to_state_id(args.block);
                }
            }
            VerticalDirection::Down => {
                let block_below = args.world.get_block(&args.position.down());
                if block_below != &Block::POINTED_DRIPSTONE {
                    dripstone_props.thickness = SpeleothemThickness::Tip;
                    return dripstone_props.to_state_id(args.block);
                }
            }
        }
        args.state_id
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

// SpeleothemBlock.spawnFallingStalactite gives only the tip the whole column's damage scale.
fn spawn_falling_stalactite(world: &Arc<World>, start: BlockPos) {
    const MIN_FALL_DAMAGE_PER_DISTANCE: i32 = 6;
    const FALL_DAMAGE_MAX: i32 = 40;
    let mut pos = start;
    while world
        .get_block(&pos)
        .has_tag(&tag::Block::MINECRAFT_SPELEOTHEMS)
    {
        let state = world.get_block_state_id(&pos);
        let props = PointedDripstoneLikeProperties::from_state_id(state);
        if props.vertical_direction != VerticalDirection::Down {
            break;
        }
        let falling = crate::entity::falling::FallingEntity::replace_spawn(world, pos, state);
        if matches!(
            props.thickness,
            SpeleothemThickness::Tip | SpeleothemThickness::TipMerge
        ) {
            let size = (1 + start.0.y - pos.0.y).max(MIN_FALL_DAMAGE_PER_DISTANCE);
            falling.set_hurts_entities(size as f32, FALL_DAMAGE_MAX);
            break;
        }
        pos = pos.down();
    }
}

// SpeleothemBlock.isValidSpeleothemPlacement requires the same block and tip direction.
fn can_survive(
    world: &dyn BlockAccessor,
    pos: &BlockPos,
    block: &Block,
    tip: VerticalDirection,
) -> bool {
    let (behind, face) = match tip {
        VerticalDirection::Up => (pos.down(), BlockDirection::Up),
        VerticalDirection::Down => (pos.up(), BlockDirection::Down),
    };
    let state = world.get_block_state(&behind);
    state.is_side_solid(face)
        || (world
            .get_block(&behind)
            .has_tag(&tag::Block::MINECRAFT_SPELEOTHEMS)
            && world.get_block(&behind) == block
            && PointedDripstoneLikeProperties::from_state_id(state.id).vertical_direction == tip)
}
fn update_stalagmite(world: &Arc<World>, stalagmite_len: u8, tip_pos: &BlockPos) {
    let block_above = world.get_block(&tip_pos.up());
    if block_above == &Block::POINTED_DRIPSTONE {
        modify_dripstone_thickness_to(world, tip_pos, SpeleothemThickness::TipMerge);
        modify_dripstone_thickness_to(world, &tip_pos.up(), SpeleothemThickness::TipMerge);
    } else {
        modify_dripstone_thickness_to(world, tip_pos, SpeleothemThickness::Tip);
    }
    match stalagmite_len {
        2 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(1),
                SpeleothemThickness::Frustum,
            );
        }
        3 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(1),
                SpeleothemThickness::Frustum,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(2),
                SpeleothemThickness::Base,
            );
        }
        4 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(1),
                SpeleothemThickness::Frustum,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(2),
                SpeleothemThickness::Middle,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(3),
                SpeleothemThickness::Base,
            );
        }
        5 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(1),
                SpeleothemThickness::Frustum,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(2),
                SpeleothemThickness::Middle,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.down_height(3),
                SpeleothemThickness::Middle,
            );
        }
        _ => {}
    }
}

fn update_stalactite(world: &Arc<World>, stalagmite_len: u8, tip_pos: &BlockPos) {
    let block_below = world.get_block(&tip_pos.down());
    if block_below == &Block::POINTED_DRIPSTONE {
        modify_dripstone_thickness_to(world, tip_pos, SpeleothemThickness::TipMerge);
        modify_dripstone_thickness_to(world, &tip_pos.down(), SpeleothemThickness::TipMerge);
    } else {
        modify_dripstone_thickness_to(world, tip_pos, SpeleothemThickness::Tip);
    }
    match stalagmite_len {
        2 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(1),
                SpeleothemThickness::Frustum,
            );
        }
        3 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(1),
                SpeleothemThickness::Frustum,
            );
            modify_dripstone_thickness_to(world, &tip_pos.up_height(2), SpeleothemThickness::Base);
        }
        4 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(1),
                SpeleothemThickness::Frustum,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(2),
                SpeleothemThickness::Middle,
            );
            modify_dripstone_thickness_to(world, &tip_pos.up_height(3), SpeleothemThickness::Base);
        }
        5 => {
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(1),
                SpeleothemThickness::Frustum,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(2),
                SpeleothemThickness::Middle,
            );
            modify_dripstone_thickness_to(
                world,
                &tip_pos.up_height(3),
                SpeleothemThickness::Middle,
            );
        }
        _ => {}
    }
}
fn get_stalagmite_or_stalactice_len_and_dir_from_tip_pos(
    world: &Arc<World>,
    position: &BlockPos,
    block_state_id: BlockStateId,
) -> (u8, VerticalDirection) {
    let props = PointedDripstoneLikeProperties::from_state_id(block_state_id);

    let mut dripstone_len = 1;
    let mut next_dripstone_pos = offset_pos_by_vertical_dir(position, props.vertical_direction);
    //We dont care if it's longer than 5 blocks because of how thickness system works.
    while dripstone_len < 5 {
        if world.get_block(&next_dripstone_pos) != &Block::POINTED_DRIPSTONE {
            break;
        }
        next_dripstone_pos =
            offset_pos_by_vertical_dir(&next_dripstone_pos, props.vertical_direction);
        dripstone_len += 1;
    }
    (dripstone_len, props.vertical_direction)
}
fn can_place_at_pos(
    block_accessor: &dyn BlockAccessor,
    position: &BlockPos,
    placing_direction: Option<BlockDirection>,
    player_option: Option<&Player>,
) -> bool {
    // Determine support block
    let Some(support_block_vertical_direction) = get_support_block_vertical_direction(
        block_accessor,
        position,
        placing_direction,
        player_option,
    ) else {
        return false;
    };
    let support_pos = match support_block_vertical_direction {
        VerticalDirection::Up => position.up(),
        VerticalDirection::Down => position.down(),
    };
    let support_block = block_accessor.get_block(&support_pos);
    if can_support_dripstone(support_block) {
        return true;
    }
    false
}

fn get_support_block_vertical_direction(
    block_accessor: &dyn BlockAccessor,
    position: &BlockPos,
    placing_direction_wrapper: Option<BlockDirection>,
    player_option: Option<&Player>,
) -> Option<VerticalDirection> {
    let Some(placing_direction) = placing_direction_wrapper else {
        //then this is basically called by a neighbor update check
        let (block, state) = block_accessor.get_block_and_state(position);
        if block != &Block::POINTED_DRIPSTONE {
            return None;
        }
        let props = PointedDripstoneLikeProperties::from_state_id(state.id);
        return Some(flip_dir(props.vertical_direction));
    };
    match block_direction_to_vertical_direction(placing_direction) {
        Some(ver_dir) => match ver_dir {
            VerticalDirection::Up => {
                let block_above = block_accessor.get_block(&position.up());
                let block_below = block_accessor.get_block(&position.down());
                if can_support_dripstone(block_above) {
                    return Some(VerticalDirection::Up);
                } else if can_support_dripstone(block_below) {
                    return Some(VerticalDirection::Down);
                }
                None
            }
            VerticalDirection::Down => {
                let block_above = block_accessor.get_block(&position.up());
                let block_below = block_accessor.get_block(&position.down());
                if can_support_dripstone(block_below) {
                    return Some(VerticalDirection::Down);
                } else if can_support_dripstone(block_above) {
                    return Some(VerticalDirection::Up);
                }
                None
            }
        },
        None => player_option.map_or(Some(VerticalDirection::Up), |player| {
            let (_, pitch) = player.rotation();
            let (can_place_above, can_place_below) = {
                let block_above = block_accessor.get_block(&position.up());
                let block_below = block_accessor.get_block(&position.down());
                (
                    can_support_dripstone(block_above),
                    can_support_dripstone(block_below),
                )
            };
            match (can_place_above, can_place_below) {
                (true, true) => {
                    if pitch > 0.0 {
                        Some(VerticalDirection::Down)
                    } else {
                        Some(VerticalDirection::Up)
                    }
                }
                (false, false) => None,
                (true, false) => Some(VerticalDirection::Up),
                (false, true) => Some(VerticalDirection::Down),
            }
        }),
    }
}
fn can_support_dripstone(support_block: &Block) -> bool {
    if support_block == &Block::POINTED_DRIPSTONE {
        return true;
    }
    support_block.default_state.is_solid_render()
}
fn modify_dripstone_thickness_to(
    world: &Arc<World>,
    pos: &BlockPos,
    new_thickness: SpeleothemThickness,
) {
    let (block, support_block_state_id) = world.get_block_and_state_id(pos);

    if block != &Block::POINTED_DRIPSTONE {
        //this shouldn't happen
        return;
    }
    let mut support_props = PointedDripstoneLikeProperties::from_state_id(support_block_state_id);
    if support_props.thickness == new_thickness {
        return;
    }
    support_props.thickness = new_thickness;
    world.set_block_state(
        pos,
        support_props.to_state_id(&Block::POINTED_DRIPSTONE),
        BlockFlags::empty(),
    );
}
fn offset_pos_by_vertical_dir(pos: &BlockPos, ver_dir: VerticalDirection) -> BlockPos {
    match ver_dir {
        VerticalDirection::Up => pos.down(),
        VerticalDirection::Down => pos.up(),
    }
}
const fn block_direction_to_vertical_direction(dir: BlockDirection) -> Option<VerticalDirection> {
    match dir {
        BlockDirection::Up => Some(VerticalDirection::Up),
        BlockDirection::Down => Some(VerticalDirection::Down),
        _ => None,
    }
}
fn flip_dir(dir: VerticalDirection) -> VerticalDirection {
    if dir == VerticalDirection::Up {
        return VerticalDirection::Down;
    }
    VerticalDirection::Up
}
