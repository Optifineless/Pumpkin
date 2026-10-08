use super::World;
use crate::{block::entities::piston::PistonBlockEntity, entity::EntityBase};
use pumpkin_data::{
    Block, BlockState,
    block_properties::{
        PistonHeadLikeProperties, ScaffoldingLikeProperties, StickyPistonLikeProperties,
    },
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};

/// Tests EntityCollisionContext.isAbove using the original feet, never swept query bounds.
pub fn is_above(bottom: f64, block_y: i32, shape_top: f64) -> bool {
    bottom > f64::from(block_y) + shape_top - f64::from(1.0e-5f32)
}

impl World {
    /// Visits local collision boxes using BlockState.getCollisionShape's entity or empty context.
    /// Callers translate by `pos` exactly once; movement intersects its swept query separately.
    pub(crate) fn for_each_collision_shape(
        &self,
        state: &BlockState,
        pos: &BlockPos,
        entity: Option<&dyn EntityBase>,
        visit: &mut dyn FnMut(BoundingBox),
    ) {
        let block = state.id.to_block();
        if block == &Block::POWDER_SNOW {
            if let Some(entity) = entity
                && let Some(shape) =
                    crate::block::blocks::powder_snow::collision_shape_for_entity(entity, pos)
            {
                visit(shape);
            }
        } else if block == &Block::SCAFFOLDING {
            scaffolding_shapes(state, pos, entity, visit);
        } else if block == &Block::LAVA || block == &Block::WATER {
            // LiquidBlock.getCollisionShape asks the living entity for its support shape.
            if let Some(mob) = entity.and_then(EntityBase::get_mob)
                && let Some(shape) = mob.liquid_collision_shape(pos)
            {
                visit(shape);
            }
        } else if block == &Block::MOVING_PISTON {
            if let Some(block_entity) = self.get_block_entity(pos)
                && let Some(piston) = block_entity.as_any().downcast_ref::<PistonBlockEntity>()
            {
                self.piston_collision_shapes(piston, pos, visit);
            }
        } else {
            state.get_block_collision_shapes_at(pos).for_each(visit);
        }
    }

    // MovingPistonBlock.getCollisionShape -> PistonMovingBlockEntity.getCollisionShape.
    fn piston_collision_shapes(
        &self,
        piston: &PistonBlockEntity,
        pos: &BlockPos,
        visit: &mut dyn FnMut(BoundingBox),
    ) {
        let moved = piston.pushed_block_state;
        let block = moved.id.to_block();
        // A forged moved-state tag must not recursively resolve this same block entity.
        if block == &Block::MOVING_PISTON {
            return;
        }
        if !piston.extending
            && piston.source
            && (block == &Block::PISTON || block == &Block::STICKY_PISTON)
        {
            let mut props = StickyPistonLikeProperties::from_state_id(moved.id);
            props.extended = true;
            let base = BlockState::from_id(props.to_state_id(block));
            self.for_each_collision_shape(base, pos, None, &mut *visit);
        }
        let progress = piston.current_progress.load();
        let state = if piston.source {
            let mut head = PistonHeadLikeProperties::default(&Block::PISTON_HEAD);
            head.facing = piston.facing.to_facing();
            head.short = piston.extending != (1.0 - progress < 0.25);
            BlockState::from_id(head.to_state_id(&Block::PISTON_HEAD))
        } else {
            moved
        };
        let offset = piston.facing.to_offset().to_f64()
            * f64::from(if piston.extending {
                progress - 1.0
            } else {
                1.0 - progress
            });
        // Moved blocks use the empty context. Pumpkin's piston pushes use set_pos, not collision queries.
        self.for_each_collision_shape(state, pos, None, &mut |shape: BoundingBox| {
            visit(shape.shift(offset));
        });
    }
}

// ScaffoldingBlock.getCollisionShape; generated collision boxes contain SHAPE_STABLE.
fn scaffolding_shapes(
    state: &BlockState,
    pos: &BlockPos,
    entity: Option<&dyn EntityBase>,
    visit: &mut dyn FnMut(BoundingBox),
) {
    let above = |top| {
        entity.is_none_or(|e| is_above(e.get_entity().bounding_box.load().min.y, pos.0.y, top))
    };
    if above(1.0) && entity.is_none_or(|e| !e.get_entity().is_sneaking()) {
        Block::SCAFFOLDING
            .default_state
            .get_block_collision_shapes_at(pos)
            .for_each(visit);
    } else {
        let props = ScaffoldingLikeProperties::from_state_id(state.id);
        if props.distance != 0 && props.bottom && above(0.0) {
            // ScaffoldingBlock.SHAPE_UNSTABLE_BOTTOM = Block.column(16, 0, 2).
            visit(BoundingBox::new_array([0.0, 0.0, 0.0], [1.0, 0.125, 1.0]));
        }
    }
}
