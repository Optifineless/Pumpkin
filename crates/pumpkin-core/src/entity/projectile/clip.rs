use pumpkin_data::{BlockDirection, BlockState};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

const EPSILON: f64 = 1.0e-7;

// VoxelShape.clip: the ray starts just inside the shape when its 0.001 step is contained.
pub fn clip_block(
    state: &BlockState,
    pos: &BlockPos,
    from: Vector3<f64>,
    to: Vector3<f64>,
) -> Option<(f64, BlockDirection)> {
    let movement = to - from;
    if movement.length_squared() < EPSILON {
        return None;
    }
    let from = from - pos.0.to_f64();
    let test = from + movement * 0.001;
    if state.get_block_collision_shapes_at(pos).any(|shape| {
        test.x >= shape.min.x
            && test.x < shape.max.x
            && test.y >= shape.min.y
            && test.y < shape.max.y
            && test.z >= shape.min.z
            && test.z < shape.max.z
    }) {
        let direction = nearest_direction(movement);
        return Some((0.001, direction.opposite()));
    }
    state
        .get_block_collision_shapes_at(pos)
        .filter_map(|shape| clip_box(from, movement, shape))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

// Direction.getApproximateNearest uses float dot products and keeps the first direction on ties.
pub(super) fn nearest_direction(movement: Vector3<f64>) -> BlockDirection {
    let movement = Vector3::new(movement.x as f32, movement.y as f32, movement.z as f32);
    let mut direction = BlockDirection::North;
    // Java Float.MIN_VALUE is the smallest positive subnormal, unlike Rust f32::MIN.
    let mut highest_dot = f32::from_bits(1);
    for candidate in BlockDirection::all() {
        let normal = candidate.to_offset();
        let dot = movement.x * normal.x as f32
            + movement.y * normal.y as f32
            + movement.z * normal.z as f32;
        if dot > highest_dot {
            highest_dot = dot;
            direction = candidate;
        }
    }
    direction
}

// AABB.clip / clipPoint: only entering planes strictly inside the ray segment count.
pub fn clip_box(
    from: Vector3<f64>,
    movement: Vector3<f64>,
    bounds: BoundingBox,
) -> Option<(f64, BlockDirection)> {
    let origin = [from.x, from.y, from.z];
    let delta = [movement.x, movement.y, movement.z];
    let min = [bounds.min.x, bounds.min.y, bounds.min.z];
    let max = [bounds.max.x, bounds.max.y, bounds.max.z];
    let min_faces = [
        BlockDirection::West,
        BlockDirection::Down,
        BlockDirection::North,
    ];
    let max_faces = [
        BlockDirection::East,
        BlockDirection::Up,
        BlockDirection::South,
    ];
    (0..3)
        .filter_map(|axis| {
            let (plane, face) = if delta[axis] > EPSILON {
                (min[axis], min_faces[axis])
            } else if delta[axis] < -EPSILON {
                (max[axis], max_faces[axis])
            } else {
                return None;
            };
            let t = (plane - origin[axis]) / delta[axis];
            (t > 0.0
                && t < 1.0
                && (0..3).filter(|&other| other != axis).all(|other| {
                    let coordinate = origin[other] + t * delta[other];
                    coordinate > min[other] - EPSILON && coordinate < max[other] + EPSILON
                }))
            .then_some((t, face))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::Block;

    #[test]
    fn partial_block_faces_and_inside_entity_rays_match_vanilla_clipping() {
        let bounds = BoundingBox::new(Vector3::new(0.25, 0.0, 0.25), Vector3::new(0.75, 1.0, 0.75));
        assert_eq!(
            clip_box(
                Vector3::new(-0.5, 0.5, 0.5),
                Vector3::new(2.0, 0.0, 0.0),
                bounds
            ),
            Some((0.375, BlockDirection::West))
        );
        assert_eq!(
            clip_box(
                Vector3::new(0.5, 0.5, 0.5),
                Vector3::new(1.0, 0.0, 0.0),
                bounds
            ),
            None
        );
        assert_eq!(
            clip_block(
                Block::STONE.default_state,
                &BlockPos::new(0, 0, 0),
                Vector3::new(0.5, 0.5, 0.5),
                Vector3::new(2.0, 0.5, 0.5)
            ),
            Some((0.001, BlockDirection::West))
        );
        assert_eq!(
            clip_block(
                Block::STONE.default_state,
                &BlockPos::new(0, 0, 0),
                Vector3::new(0.5, 0.5, 0.5),
                Vector3::new(1.500_000_001, 1.5, 0.5)
            ),
            Some((0.001, BlockDirection::Down))
        );
    }
}
