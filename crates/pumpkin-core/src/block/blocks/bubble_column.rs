use std::sync::Arc;

use pumpkin_data::block_properties::BubbleColumnLikeProperties;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

use crate::block::{
    BlockBehaviour, BlockMetadata, GetStateForNeighborUpdateArgs, OnEntityCollisionArgs,
    OnNeighborUpdateArgs, OnScheduledTickArgs, PlacedArgs, fluid::water::WATER_FLOW_SPEED,
};
use crate::world::World;

#[cfg(test)]
mod runtime_test_support;
#[cfg(test)]
mod runtime_tests;

const BUBBLE_COLUMN_CHECK_DELAY: u8 = 20;
const CHECK_PERIOD: u8 = 5;

const UPWARD_ACCELERATION: f64 = 0.06;
const UPWARD_MAX_SPEED: f64 = 0.7;
const SURFACE_UPWARD_ACCELERATION: f64 = 0.1;
const SURFACE_UPWARD_MAX_SPEED: f64 = 1.8;
const DOWNWARD_ACCELERATION: f64 = -0.03;
const DOWNWARD_MIN_SPEED: f64 = -0.3;

pub struct BubbleColumnBlock;

impl BlockMetadata for BubbleColumnBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::BUBBLE_COLUMN, BlockId::WATER].into()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BubbleColumnKind {
    Upward,
    Downward,
}

fn bubble_column_state(kind: BubbleColumnKind) -> BlockStateId {
    BubbleColumnLikeProperties {
        r#drag: matches!(kind, BubbleColumnKind::Downward),
    }
    .to_state_id(&Block::BUBBLE_COLUMN)
}

fn kind_from_support(block: &Block) -> Option<BubbleColumnKind> {
    if block.has_tag(&tag::Block::MINECRAFT_ENABLES_BUBBLE_COLUMN_PUSH_UP) {
        Some(BubbleColumnKind::Upward)
    } else if block.has_tag(&tag::Block::MINECRAFT_ENABLES_BUBBLE_COLUMN_DRAG_DOWN) {
        Some(BubbleColumnKind::Downward)
    } else {
        None
    }
}

fn kind_from_state(state: BlockStateId) -> BubbleColumnKind {
    let props = BubbleColumnLikeProperties::from_state_id(state);
    if props.r#drag {
        BubbleColumnKind::Downward
    } else {
        BubbleColumnKind::Upward
    }
}

fn is_source_water_state(state: BlockStateId) -> bool {
    // BubbleColumnBlock.canOccupy requires a LiquidBlock, not contained water.
    if !matches!(state.to_block_id(), BlockId::WATER | BlockId::LAVA) {
        return false;
    }
    let Some(fluid) = Fluid::from_state_id(state) else {
        return false;
    };

    fluid.has_tag(&tag::Fluid::MINECRAFT_BUBBLE_COLUMN_CAN_OCCUPY)
        && fluid.states.iter().any(|fluid_state| {
            fluid_state.block_state_id == state && fluid_state.is_source && fluid_state.level >= 8
        })
}

fn can_occupy(state: BlockStateId) -> bool {
    state.to_block_id() == BlockId::BUBBLE_COLUMN || is_source_water_state(state)
}

fn get_column_state(below_state: BlockStateId, occupy_state: BlockStateId) -> BlockStateId {
    if below_state.to_block_id() == BlockId::BUBBLE_COLUMN {
        below_state
    } else if let Some(kind) = kind_from_support(below_state.to_block()) {
        bubble_column_state(kind)
    } else if occupy_state.to_block_id() == BlockId::BUBBLE_COLUMN {
        Block::WATER.default_state.id
    } else {
        occupy_state
    }
}

fn update_column(world: &Arc<World>, position: &BlockPos) {
    let Some(state) = world.get_block_state_id_if_loaded(position) else {
        return;
    };
    if !can_occupy(state) {
        return;
    }
    let below_state = world.get_block_state_id(&position.down());
    let column_state = get_column_state(below_state, state);
    // BubbleColumnBlock.updateColumn continues after an unchanged first write.
    if world
        .set_block_state_if(
            position,
            column_state,
            BlockFlags::NOTIFY_LISTENERS,
            |current| current == state,
        )
        .is_none()
    {
        return;
    }

    let mut above = position.up();
    while let Some(state) = world.get_block_state_id_if_loaded(&above) {
        if !can_occupy(state) {
            break;
        }
        let Some(replaced) = world.set_block_state_if(
            &above,
            column_state,
            BlockFlags::NOTIFY_LISTENERS,
            |current| current == state,
        ) else {
            break;
        };
        if replaced == column_state {
            break;
        }
        above = above.up();
    }
}

fn try_schedule_bubble_block_column(
    world: &World,
    position: &BlockPos,
    state: BlockStateId,
    below_state: BlockStateId,
) {
    // LiquidBlock.tryScheduleBubbleBlockColumn schedules the water, not its support.
    if is_source_water_state(state) && kind_from_support(below_state.to_block()).is_some() {
        world.schedule_block_tick(
            &Block::WATER,
            *position,
            BUBBLE_COLUMN_CHECK_DELAY,
            TickPriority::Normal,
        );
    }
}

fn bubble_column_vertical_velocity(
    current_y: f64,
    kind: BubbleColumnKind,
    at_surface: bool,
) -> f64 {
    match (kind, at_surface) {
        (BubbleColumnKind::Upward, true) => {
            (current_y + SURFACE_UPWARD_ACCELERATION).min(SURFACE_UPWARD_MAX_SPEED)
        }
        (BubbleColumnKind::Upward, false) => {
            (current_y + UPWARD_ACCELERATION).min(UPWARD_MAX_SPEED)
        }
        (BubbleColumnKind::Downward, _) => {
            (current_y + DOWNWARD_ACCELERATION).max(DOWNWARD_MIN_SPEED)
        }
    }
}

fn bubble_column_velocity(
    current: Vector3<f64>,
    kind: BubbleColumnKind,
    at_surface: bool,
) -> Vector3<f64> {
    Vector3::new(
        current.x,
        bubble_column_vertical_velocity(current.y, kind, at_surface),
        current.z,
    )
}

impl BlockBehaviour for BubbleColumnBlock {
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        {
            if args.block != &Block::BUBBLE_COLUMN {
                return;
            }

            let kind = kind_from_state(args.state.id);
            let at_surface = args.world.get_block(&args.position.up()) == &Block::AIR;
            let entity = args.entity.get_entity();
            entity.velocity.store(bubble_column_velocity(
                entity.velocity.load(),
                kind,
                at_surface,
            ));

            if let Some(player) = args.entity.get_player() {
                player.breath_manager.reset(player);
            }
        }
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if args.block == &Block::WATER {
            try_schedule_bubble_block_column(
                args.world,
                args.position,
                args.world.get_block_state_id(args.position),
                args.world.get_block_state_id(&args.position.down()),
            );
        }
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        if args.block == &Block::WATER {
            // LiquidBlock.neighborChanged reads the actual support, not the notification's block.
            try_schedule_bubble_block_column(
                args.world,
                args.position,
                args.world.get_block_state_id(args.position),
                args.world.get_block_state_id(&args.position.down()),
            );
        }
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.block == &Block::WATER {
            // Only LiquidBlock.updateShape's column scheduling is handled here.
            if args.direction == BlockDirection::Down {
                try_schedule_bubble_block_column(
                    args.world,
                    args.position,
                    args.state_id,
                    args.neighbor_state_id,
                );
            }
        } else if args.block == &Block::BUBBLE_COLUMN {
            // BubbleColumnBlock.updateShape always keeps its source-water fluid ticking.
            args.world.schedule_fluid_tick(
                &Fluid::FLOWING_WATER,
                *args.position,
                WATER_FLOW_SPEED,
                TickPriority::Normal,
            );
            let below = args.world.get_block(&args.position.down());
            let supported = below == &Block::BUBBLE_COLUMN || kind_from_support(below).is_some();
            if !supported
                || args.direction == BlockDirection::Down
                || (args.direction == BlockDirection::Up
                    && args.neighbor_state_id.to_block_id() != BlockId::BUBBLE_COLUMN
                    && can_occupy(args.neighbor_state_id))
            {
                args.world.schedule_block_tick(
                    &Block::BUBBLE_COLUMN,
                    *args.position,
                    CHECK_PERIOD,
                    TickPriority::Normal,
                );
            }
        }
        args.state_id
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        update_column(args.world, args.position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_tags_map_to_expected_kinds() {
        assert_eq!(
            kind_from_support(&Block::SOUL_SAND),
            Some(BubbleColumnKind::Upward)
        );
        assert_eq!(
            kind_from_support(&Block::MAGMA_BLOCK),
            Some(BubbleColumnKind::Downward)
        );
        assert_eq!(kind_from_support(&Block::STONE), None);
    }
}
