use pumpkin_data::BlockStateId;
use pumpkin_util::math::{
    boundingbox::BoundingBox,
    position::BlockPos,
    vector3::{Axis, Vector3},
};

use crate::world::World;

/// Lifts entities out of the collision volume added by a state change, before writing that state.
pub fn push_entities_up(
    old_state: BlockStateId,
    new_state: BlockStateId,
    world: &World,
    pos: &BlockPos,
) -> BlockStateId {
    // Block.pushEntitiesUp: BooleanOp.ONLY_SECOND, then collide down from one block above.
    let mut added = Vec::new();
    world.for_each_collision_shape(new_state.to_state(), pos, None, &mut |shape| {
        added.push(shape);
    });
    let mut old = Vec::new();
    world.for_each_collision_shape(old_state.to_state(), pos, None, &mut |shape| {
        old.push(shape);
    });
    for old_box in old {
        added = added
            .into_iter()
            .flat_map(|new_box| subtract_box(new_box, old_box))
            .collect();
    }
    let mut shapes = added.into_iter().map(|shape| shape.at_pos(*pos));
    let Some(first) = shapes.next() else {
        return new_state;
    };
    let mut bounds = first;
    let mut added = vec![first];
    for shape in shapes {
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            bounds.min.set_axis(
                axis,
                bounds.min.get_axis(axis).min(shape.min.get_axis(axis)),
            );
            bounds.max.set_axis(
                axis,
                bounds.max.get_axis(axis).max(shape.max.get_axis(axis)),
            );
        }
        added.push(shape);
    }
    for entity in world.get_all_at_box(&bounds) {
        // EntityGetter.getEntities defaults to EntitySelector.NO_SPECTATORS.
        if entity.is_spectator() || entity.get_entity().is_removed() {
            continue;
        }
        let base = entity.get_entity();
        let above = base.bounding_box.load().shift(Vector3::new(0.0, 1.0, 0.0));
        let mut time = 1.0;
        for shape in &added {
            if let Some(hit) =
                above.calculate_collision_time(shape, Vector3::new(0.0, -1.0, 0.0), Axis::Y, time)
            {
                time = hit;
            }
        }
        entity.teleport(
            base.pos.load().add_raw(0.0, 1.0 - time, 0.0),
            None,
            None,
            base.world.load_full(),
        );
    }
    new_state
}

// Shapes.joinUnoptimized(ONLY_SECOND), represented by the fork's generated collision boxes.
fn subtract_box(mut new_box: BoundingBox, old_box: BoundingBox) -> Vec<BoundingBox> {
    if !new_box.intersects(&old_box) {
        return vec![new_box];
    }
    let mut pieces = Vec::new();
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let low = new_box.min.get_axis(axis).max(old_box.min.get_axis(axis));
        let high = new_box.max.get_axis(axis).min(old_box.max.get_axis(axis));
        if new_box.min.get_axis(axis) < low {
            let mut piece = new_box;
            piece.max.set_axis(axis, low);
            pieces.push(piece);
            new_box.min.set_axis(axis, low);
        }
        if new_box.max.get_axis(axis) > high {
            let mut piece = new_box;
            piece.min.set_axis(axis, high);
            pieces.push(piece);
            new_box.max.set_axis(axis, high);
        }
    }
    pieces
}
