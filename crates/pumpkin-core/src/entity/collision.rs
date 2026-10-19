use super::{Entity, EntityBase};
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::{
    boundingbox::BoundingBox,
    position::BlockPos,
    vector3::{Axis, Vector3},
};
use std::sync::atomic::Ordering::SeqCst;

const GROUND_PROBE: f32 = 1.0e-5;
const SUPPORT_PROBE: f64 = 1.0e-6;
const COLLISION_EPSILON: f64 = 1.0e-7;

impl Entity {
    pub(super) fn apply_collision_movement_velocity(
        &self,
        motion: Vector3<f64>,
        displacement: Vector3<f64>,
        multiplier: f64,
        stepped: bool,
    ) {
        let mut velocity = displacement * multiplier;
        // A collision step changes position, not the incident vertical velocity used by
        // Block.updateEntityMovementAfterFallOn (including restitution overrides).
        if stepped {
            velocity.y = motion.y;
        }
        self.velocity.store(velocity);
    }

    // Vanilla 26.3 Entity.collide; players still bypass move_entity.
    #[expect(clippy::float_cmp)]
    pub(super) fn adjust_movement_for_collisions(
        &self,
        movement: Vector3<f64>,
        caller: &dyn EntityBase,
    ) -> (Vector3<f64>, bool) {
        if movement.length_squared() == 0.0 {
            return (movement, false);
        }
        let mut did_step = false;
        let was_on_ground = self.on_ground.load(SeqCst);
        let bounds = self.bounding_box.load();
        let world = self.world.load_full();
        let (mut collisions, mut positions) =
            world.get_block_collisions(bounds.stretch(movement), caller);
        let mut resolved = collide_with_shapes(movement, bounds, &collisions);
        let on_ground_after_collision = movement.y < 0.0 && movement.y != resolved.y;
        let step_height = caller.get_living_entity().map_or(0.0, |living| {
            living.get_attribute_value(&Attributes::STEP_HEIGHT) as f32
        });

        if step_height > 0.0
            && (on_ground_after_collision || was_on_ground)
            && (movement.x != resolved.x || movement.z != resolved.z)
        {
            let grounded = if on_ground_after_collision {
                bounds.shift(Vector3::new(0.0, resolved.y, 0.0))
            } else {
                bounds
            };
            let step_movement = Vector3::new(movement.x, f64::from(step_height), movement.z);
            let mut swept = grounded.stretch(step_movement);
            if !on_ground_after_collision {
                swept = swept.stretch(Vector3::new(0.0, -f64::from(GROUND_PROBE), 0.0));
            }
            // A rejected retry retains the already bounded flat movement, without changing flags.
            if self.collision_sweep_is_bounded(swept, step_movement) {
                let (step_collisions, step_positions) = world.get_block_collisions(swept, caller);
                let candidates = collect_candidate_step_up_heights(
                    grounded,
                    &step_collisions,
                    step_height,
                    resolved.y as f32,
                );
                for height in candidates {
                    let mut stepped = collide_with_shapes(
                        Vector3::new(movement.x, f64::from(height), movement.z),
                        grounded,
                        &step_collisions,
                    );
                    if stepped.horizontal_length_squared() > resolved.horizontal_length_squared() {
                        stepped.y -= bounds.min.y - grounded.min.y;
                        resolved = stepped;
                        did_step = true;
                        collisions = step_collisions;
                        positions = step_positions;
                        break;
                    }
                }
            }
        }

        let on_ground = movement.y < 0.0 && movement.y != resolved.y;
        self.on_ground.store(on_ground, SeqCst);
        self.supporting_block_pos.store(if on_ground {
            supporting_block(bounds.shift(resolved), &collisions, &positions).or_else(|| {
                // Entity.checkSupportingBlock keeps source-footprint support on the
                // collision tick that carries an entity off a ledge.
                supporting_block(
                    bounds.shift(Vector3::new(0.0, resolved.y, 0.0)),
                    &collisions,
                    &positions,
                )
            })
        } else {
            None
        });
        self.horizontal_collision
            .store(movement.x != resolved.x || movement.z != resolved.z, SeqCst);
        (resolved, did_step)
    }
}

// Entity.collectCandidateStepUpHeights uses float heights, sorted and deduplicated.
#[expect(clippy::float_cmp)]
fn collect_candidate_step_up_heights(
    bounds: BoundingBox,
    collisions: &[BoundingBox],
    max_step_height: f32,
    step_height_to_skip: f32,
) -> Vec<f32> {
    let mut candidates: Vec<_> = collisions
        .iter()
        .flat_map(|shape| [shape.min.y, shape.max.y])
        .map(|y| (y - bounds.min.y) as f32)
        .filter(|height| {
            *height >= 0.0 && *height <= max_step_height && *height != step_height_to_skip
        })
        .collect();
    candidates.sort_unstable_by(f32::total_cmp);
    candidates.dedup();
    candidates
}

// Entity.collideWithShapes clips Y first, then the larger horizontal component.
// Each axis sees the box shifted by previously resolved axes, never a diagonal sweep.
fn collide_with_shapes(
    movement: Vector3<f64>,
    mut bounds: BoundingBox,
    collisions: &[BoundingBox],
) -> Vector3<f64> {
    if collisions.is_empty() {
        return movement;
    }
    let axes = if movement.x.abs() < movement.z.abs() {
        [Axis::Y, Axis::Z, Axis::X]
    } else {
        [Axis::Y, Axis::X, Axis::Z]
    };
    let mut resolved = Vector3::default();
    for axis in axes {
        let mut distance = movement.get_axis(axis);
        for collision in collisions {
            if distance.abs() < COLLISION_EPSILON {
                distance = 0.0;
                break;
            }
            distance = collide_shape(axis, bounds, *collision, distance);
        }
        let mut delta = Vector3::default();
        delta.set_axis(axis, distance);
        resolved.set_axis(axis, delta.get_axis(axis));
        bounds = bounds.shift(delta);
    }
    resolved
}

// VoxelShape.collideX for Pumpkin's extracted collision AABBs, including its epsilon.
fn collide_shape(axis: Axis, bounds: BoundingBox, shape: BoundingBox, distance: f64) -> f64 {
    if Axis::excluding(axis).into_iter().any(|other| {
        bounds.max.get_axis(other) - COLLISION_EPSILON < shape.min.get_axis(other)
            || bounds.min.get_axis(other) + COLLISION_EPSILON >= shape.max.get_axis(other)
    }) {
        return distance;
    }
    if distance > 0.0 {
        let gap = shape.min.get_axis(axis) - bounds.max.get_axis(axis);
        if bounds.max.get_axis(axis) - COLLISION_EPSILON < shape.min.get_axis(axis)
            && gap >= -COLLISION_EPSILON
        {
            return distance.min(gap);
        }
    } else if distance < 0.0 {
        let gap = shape.max.get_axis(axis) - bounds.min.get_axis(axis);
        if bounds.min.get_axis(axis) + COLLISION_EPSILON >= shape.max.get_axis(axis)
            && gap <= COLLISION_EPSILON
        {
            return distance.max(gap);
        }
    }
    distance
}

fn supporting_block(
    bounds: BoundingBox,
    collisions: &[BoundingBox],
    positions: &[(usize, BlockPos)],
) -> Option<BlockPos> {
    // Select support under the final footprint, including the newly climbed block.
    let mut center = (bounds.min + bounds.max) * 0.5;
    center.y = bounds.min.y;
    collisions
        .iter()
        .enumerate()
        .filter(|(_, shape)| {
            (shape.max.y - bounds.min.y).abs() < SUPPORT_PROBE
                && shape.max.x > bounds.min.x
                && shape.min.x < bounds.max.x
                && shape.max.z > bounds.min.z
                && shape.min.z < bounds.max.z
        })
        .filter_map(|(index, _)| positions.iter().find(|(end, _)| index < *end))
        .map(|(_, position)| *position)
        .min_by(|a, b| {
            a.0.to_f64()
                .add_raw(0.5, 0.5, 0.5)
                .squared_distance_to_vec(&center)
                .total_cmp(
                    &b.0.to_f64()
                        .add_raw(0.5, 0.5, 0.5)
                        .squared_distance_to_vec(&center),
                )
        })
}
