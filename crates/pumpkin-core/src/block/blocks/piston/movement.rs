use std::sync::Arc;

use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId,
    block_properties::{MovingPistonLikeProperties, PistonHeadLikeProperties, PistonType},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use rustc_hash::FxHashMap;

use crate::{
    block::{blocks::piston::PistonHandler, entities::piston::PistonBlockEntity},
    world::World,
};

use super::PistonProps;

type StateWrite = (BlockPos, BlockStateId, BlockStateId, BlockFlags);

pub(super) struct PistonMove {
    pub position: BlockPos,
    pub observed: BlockStateId,
    pub direction: BlockDirection,
    pub extending: bool,
    pub sticky: bool,
    pub pull: Option<BlockStateId>,
    pub event_data: u8,
}

struct MovementPlan {
    updates: Vec<StateWrite>,
    entities: Vec<PistonBlockEntity>,
    broken: Vec<(BlockPos, BlockStateId, bool)>,
    moved: Vec<(BlockPos, BlockStateId)>,
}

pub(super) fn move_piston(world: &Arc<World>, movement: &PistonMove) -> bool {
    let Some(plan) = plan_move(world, movement) else {
        return false;
    };
    if !world.set_block_states_from_observed(&plan.updates) {
        return false;
    }
    let mut plan = plan;
    for entity in plan.entities.drain(..) {
        if let Some((_, _, expected, _)) = plan
            .updates
            .iter()
            .find(|(position, ..)| *position == entity.position)
        {
            world.add_block_entity_from_observed(Arc::new(entity), *expected);
        }
    }
    publish_movement(world, movement, &plan);
    true
}

// PistonBaseBlock.moveBlocks resolves every source before admitting the terrain transaction.
fn resolve_movement<'a>(world: &'a Arc<World>, movement: &PistonMove) -> Option<PistonHandler<'a>> {
    let mut handler = PistonHandler::new(
        world,
        movement.position,
        movement.direction,
        movement.extending,
    );
    if movement.extending || movement.pull.is_some() {
        let resolved = handler.calculate_push();
        if !handler.observations_current || (!resolved && movement.extending) {
            return None;
        }
        if !resolved {
            handler.moved_blocks.clear();
            handler.broken_blocks.clear();
        }
    }
    if let Some(expected) = movement.pull {
        let source = movement
            .position
            .offset_dir(movement.direction.to_offset(), 2);
        if handler.observed.get(&source) != Some(&expected) {
            return None;
        }
    }
    let head = movement.position.offset(movement.direction.to_offset());
    let head_state = world.get_block_state_id(&head);
    if !movement.extending
        && !head_state.to_state().is_air()
        && head_state.to_block() != &Block::PISTON_HEAD
        && head_state.to_block() != &Block::MOVING_PISTON
    {
        return None;
    }
    if handler
        .observed
        .get(&head)
        .is_some_and(|old| *old != head_state)
    {
        return None;
    }
    handler.observed.insert(head, head_state);
    handler
        .observed
        .insert(movement.position, movement.observed);
    Some(handler)
}

fn plan_move(world: &Arc<World>, movement: &PistonMove) -> Option<MovementPlan> {
    let handler = resolve_movement(world, movement)?;
    let mut updates = handler
        .observed
        .iter()
        .map(|(pos, state)| (*pos, (*state, *state, BlockFlags::empty())))
        .collect::<FxHashMap<_, _>>();
    let moved = handler
        .moved_blocks
        .iter()
        .map(|pos| (*pos, handler.observed[pos]))
        .collect::<Vec<_>>();
    let broken = admit_broken(world, &handler.broken_blocks, &handler.observed)?;
    for (pos, old, _) in &broken {
        updates.insert(
            *pos,
            (
                *old,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::FORCE_STATE,
            ),
        );
    }
    for (pos, state) in &moved {
        updates.insert(
            *pos,
            (
                *state,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::FORCE_STATE | BlockFlags::MOVED,
            ),
        );
    }
    let mut entities = Vec::new();
    plan_base_and_head(movement, &mut updates, &mut entities);
    let direction = if movement.extending {
        movement.direction
    } else {
        movement.direction.opposite()
    };
    for (pos, state) in moved.iter().rev() {
        let target = pos.offset(direction.to_offset());
        let old = handler.observed.get(&target).copied()?;
        let props = moving_properties(movement, false);
        updates.insert(
            target,
            (
                old,
                props.to_state_id(&Block::MOVING_PISTON),
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::MOVED,
            ),
        );
        entities.push(moving_entity(movement, target, state.to_state(), false));
    }
    Some(MovementPlan {
        updates: order_updates(movement, updates, &moved, &broken),
        entities,
        broken,
        moved,
    })
}

fn order_updates(
    movement: &PistonMove,
    mut updates: FxHashMap<BlockPos, (BlockStateId, BlockStateId, BlockFlags)>,
    moved: &[(BlockPos, BlockStateId)],
    broken: &[(BlockPos, BlockStateId, bool)],
) -> Vec<StateWrite> {
    let direction = if movement.extending {
        movement.direction
    } else {
        movement.direction.opposite()
    };
    let mut positions = Vec::new();
    if !movement.extending {
        positions.push(movement.position);
    }
    positions.extend(broken.iter().map(|(pos, _, _)| *pos));
    positions.extend(
        moved
            .iter()
            .rev()
            .map(|(pos, _)| pos.offset(direction.to_offset())),
    );
    positions.push(movement.position.offset(movement.direction.to_offset()));
    positions.extend(moved.iter().map(|(pos, _)| *pos));
    if movement.extending {
        positions.push(movement.position);
    }
    let mut ordered = Vec::with_capacity(updates.len());
    for pos in positions {
        if let Some((old, new, flags)) = updates.remove(&pos) {
            ordered.push((pos, old, new, flags));
        }
    }
    ordered.extend(
        updates
            .into_iter()
            .map(|(pos, (old, new, flags))| (pos, old, new, flags)),
    );
    ordered
}

fn admit_broken(
    world: &Arc<World>,
    positions: &[BlockPos],
    observed: &FxHashMap<BlockPos, BlockStateId>,
) -> Option<Vec<(BlockPos, BlockStateId, bool)>> {
    let mut broken = Vec::new();
    for pos in positions.iter().rev() {
        let state = observed[pos];
        let mut event = crate::plugin::api::events::block::block_break::BlockBreakEvent::new(
            None,
            state.to_block(),
            *pos,
            0,
            true,
        );
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return None;
        }
        broken.push((*pos, state, event.drop));
    }
    Some(broken)
}

fn plan_base_and_head(
    movement: &PistonMove,
    updates: &mut FxHashMap<BlockPos, (BlockStateId, BlockStateId, BlockFlags)>,
    entities: &mut Vec<PistonBlockEntity>,
) {
    let block = movement.observed.to_block();
    let mut props = PistonProps::from_state_id(movement.observed);
    props.extended = movement.extending;
    let moving = moving_properties(movement, true).to_state_id(&Block::MOVING_PISTON);
    let base = if movement.extending {
        props.to_state_id(block)
    } else {
        moving
    };
    let base_flags = BlockFlags::NOTIFY_ALL
        | if movement.extending {
            BlockFlags::MOVED
        } else {
            BlockFlags::FORCE_STATE
        };
    updates.insert(movement.position, (movement.observed, base, base_flags));
    let head = movement.position.offset(movement.direction.to_offset());
    let head_state = updates[&head].0;
    let head_flags = if movement.extending {
        BlockFlags::NOTIFY_LISTENERS | BlockFlags::MOVED
    } else {
        BlockFlags::NOTIFY_ALL
    };
    updates.insert(
        head,
        (
            head_state,
            if movement.extending {
                moving
            } else {
                BlockStateId::AIR
            },
            head_flags,
        ),
    );
    if movement.extending {
        let mut head_props = PistonHeadLikeProperties::default(&Block::PISTON_HEAD);
        head_props.facing = movement.direction.to_facing();
        head_props.r#type = if movement.sticky {
            PistonType::Sticky
        } else {
            PistonType::Normal
        };
        entities.push(moving_entity(
            movement,
            head,
            head_props.to_state_id(&Block::PISTON_HEAD).to_state(),
            true,
        ));
    } else {
        props.facing = BlockDirection::by_index((movement.event_data & 7) as usize)
            .unwrap_or(BlockDirection::North)
            .to_facing();
        entities.push(moving_entity(
            movement,
            movement.position,
            props.to_state_id(block).to_state(),
            true,
        ));
    }
}

fn moving_properties(movement: &PistonMove, source: bool) -> MovingPistonLikeProperties {
    let mut props = MovingPistonLikeProperties::default(&Block::MOVING_PISTON);
    props.facing = movement.direction.to_facing();
    props.r#type = if source && movement.sticky {
        PistonType::Sticky
    } else {
        PistonType::Normal
    };
    props
}

fn moving_entity(
    movement: &PistonMove,
    position: BlockPos,
    state: &'static BlockState,
    source: bool,
) -> PistonBlockEntity {
    PistonBlockEntity {
        position,
        facing: movement.direction,
        pushed_block_state: state,
        current_progress: 0.0.into(),
        last_progress: 0.0.into(),
        extending: movement.extending,
        source,
    }
}

fn publish_movement(world: &Arc<World>, movement: &PistonMove, plan: &MovementPlan) {
    if !movement.extending {
        world.update_neighbors(&movement.position, None);
    }
    for (pos, state, next, _) in &plan.updates {
        if *next == BlockStateId::AIR && plan.moved.iter().any(|(source, _)| source == pos) {
            world.block_registry.prepare(
                world,
                pos,
                state.to_block(),
                *state,
                BlockFlags::NOTIFY_LISTENERS,
            );
            world
                .block_registry
                .update_neighbors(world, pos, BlockFlags::NOTIFY_LISTENERS);
            world.block_registry.prepare(
                world,
                pos,
                &Block::AIR,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_LISTENERS,
            );
        }
    }
    for (pos, state, drop) in &plan.broken {
        if *drop {
            let params = crate::world::loot::LootContextParameters {
                block_state: Some(state.to_state()),
                position: Some(pos.to_f64()),
                killed_by_player: Some(false),
                ..Default::default()
            };
            crate::block::drop_loot(world, state.to_block(), pos, true, &params);
        }
        if state.to_block() != &Block::FIRE && state.to_block() != &Block::SOUL_FIRE {
            world.send_block_destroy_effect(pos, *state, None);
        }
        world
            .block_registry
            .on_state_replaced(world, state.to_block(), pos, *state, false);
        world.block_registry.prepare(
            world,
            pos,
            state.to_block(),
            *state,
            BlockFlags::NOTIFY_LISTENERS,
        );
        world.update_neighbors_at(pos, state.to_block(), None);
    }
    for (pos, state) in plan.moved.iter().rev() {
        world.update_neighbors_at(pos, state.to_block(), None);
    }
    if movement.extending {
        world.update_neighbors_at(
            &movement.position.offset(movement.direction.to_offset()),
            &Block::PISTON_HEAD,
            None,
        );
    }
}
