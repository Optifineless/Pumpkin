use super::PathNavigation;
use crate::world::World;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

// Shapes.EPSILON, used by VoxelShape.calculateFace and clip.
const EPSILON: f64 = 1e-7;

// FlyingPathNavigation.isStableDestination -> BlockState.entityCanStandOn -> Block.isFaceFull.
pub(super) fn entity_can_stand_on(
    world: &World,
    pos: &BlockPos,
    entity: &dyn crate::entity::EntityBase,
) -> bool {
    let mut faces = Vec::new();
    world.for_each_collision_shape(
        world.get_block_state(pos),
        pos,
        Some(entity),
        &mut |shape| {
            // VoxelShape.calculateFace slices the occupied cells at 0.9999999.
            if shape.min.y <= 1.0 - EPSILON && shape.max.y > 1.0 - EPSILON {
                faces.push(shape);
            }
        },
    );
    let mut xs = vec![0.0, 1.0];
    let mut zs = vec![0.0, 1.0];
    for shape in &faces {
        xs.extend([shape.min.x.clamp(0.0, 1.0), shape.max.x.clamp(0.0, 1.0)]);
        zs.extend([shape.min.z.clamp(0.0, 1.0), shape.max.z.clamp(0.0, 1.0)]);
    }
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    zs.sort_by(f64::total_cmp);
    zs.dedup();
    xs.windows(2).all(|x| {
        zs.windows(2).all(|z| {
            faces.iter().any(|face| {
                face.min.x <= x[0] && face.max.x >= x[1] && face.min.z <= z[0] && face.max.z >= z[1]
            })
        })
    })
}

// PathNavigation.isClearForMovementBetween -> BlockGetter.clip with COLLIDER and ANY/NONE fluids.
pub fn is_clear_for_movement_between(
    entity: &dyn crate::entity::EntityBase,
    start: Vector3<f64>,
    stop: Vector3<f64>,
    height: f32,
    fluids: bool,
) -> bool {
    let world = entity.get_entity().world.load();
    let end = Vector3::new(stop.x, stop.y + f64::from(height) * 0.5, stop.z);
    World::traverse_blocks(start, end, |pos, _| {
        let state = world.get_block_state(pos);
        let hits_shape = |shape: BoundingBox| clips_movement_shape(start, end, shape.at_pos(*pos));
        let mut block_hit = false;
        world.for_each_collision_shape(state, pos, Some(entity), &mut |shape| {
            block_hit |= hits_shape(shape);
        });
        let fluid_hit = fluids && {
            let (fluid, state) = world.get_fluid_and_fluid_state(pos);
            fluid.id != pumpkin_data::fluid::Fluid::EMPTY.id && {
                let height = world.get_fluid_height(pos, fluid, &state);
                let min = pos.0.to_f64();
                clips_movement_shape(
                    start,
                    end,
                    BoundingBox::new(min, min + Vector3::new(1.0, f64::from(height), 1.0)),
                )
            }
        };
        (block_hit || fluid_hit).then_some(())
    })
    .is_none()
}

/// Tests a world-space collision box using vanilla `VoxelShape.clip` / `AABB.clip` semantics.
///
/// Pass the original segment endpoints; the block traversal epsilon is separate.
#[must_use]
pub fn clips_movement_shape(from: Vector3<f64>, to: Vector3<f64>, shape: BoundingBox) -> bool {
    let direction = to - from;
    if direction.length_squared() < EPSILON {
        return false;
    }
    let test = from + direction * 0.001;
    if test.x >= shape.min.x
        && test.x < shape.max.x
        && test.y >= shape.min.y
        && test.y < shape.max.y
        && test.z >= shape.min.z
        && test.z < shape.max.z
    {
        return true;
    }
    let origin = [from.x, from.y, from.z];
    let delta = [direction.x, direction.y, direction.z];
    let min = [shape.min.x, shape.min.y, shape.min.z];
    let max = [shape.max.x, shape.max.y, shape.max.z];
    (0..3).any(|axis| {
        let plane = if delta[axis] > EPSILON {
            min[axis]
        } else if delta[axis] < -EPSILON {
            max[axis]
        } else {
            return false;
        };
        let t = (plane - origin[axis]) / delta[axis];
        t > 0.0
            && t < 1.0
            && (0..3).filter(|&other| other != axis).all(|other| {
                let coordinate = origin[other] + t * delta[other];
                coordinate > min[other] - EPSILON && coordinate < max[other] + EPSILON
            })
    })
}

impl PathNavigation {
    // PathNavigation.getGroundY -> WalkNodeEvaluator.getFloorLevel.
    pub(super) fn get_ground_y(world: &World, target: Vector3<f64>) -> f64 {
        let below = BlockPos::floored_v(target).down();
        let state = world.get_block_state(&below);
        if state.is_air() {
            return target.y;
        }
        let mut height: f64 = 0.0;
        world.for_each_collision_shape(state, &below, None, &mut |shape| {
            height = height.max(shape.max.y);
        });
        f64::from(below.0.y) + height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_collision_ray_leaves_faces_and_misses_endpoint_contact() {
        let shape = BoundingBox::full_block();
        assert!(!clips_movement_shape(
            Vector3::new(0.5, 1.0, 0.5),
            Vector3::new(0.5, 2.0, 0.5),
            shape,
        ));
        assert!(!clips_movement_shape(
            Vector3::new(-1.0, 0.5, 0.5),
            Vector3::new(0.0, 0.5, 0.5),
            shape,
        ));
        assert!(clips_movement_shape(
            Vector3::new(-1.0, 0.5, 0.5),
            Vector3::new(1.0, 0.5, 0.5),
            shape,
        ));
    }
}
