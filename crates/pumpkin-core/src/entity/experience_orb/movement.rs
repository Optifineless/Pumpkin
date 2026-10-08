use super::ExperienceOrbEntity;
use crate::entity::Entity;
use pumpkin_data::{
    BlockDirection,
    tag::{self, Taggable},
};
use pumpkin_util::math::{
    boundingbox::BoundingBox,
    position::BlockPos,
    vector3::{Axis, Vector3},
};

impl ExperienceOrbEntity {
    // ExperienceOrb.getBlockPosBelowThatAffectsMyMovement -> Entity.getOnPos(0.999999F).
    pub(super) fn get_block_pos_below_that_affects_my_movement(&self) -> BlockPos {
        if let Some(support) = self.entity.supporting_block_pos.load() {
            let block = self.entity.world.load().get_block(&support);
            if block.has_tag(&tag::Block::MINECRAFT_WALLS)
                || block.has_tag(&tag::Block::MINECRAFT_FENCE_GATES)
            {
                return support;
            }
        }
        self.entity.get_pos_with_y_offset(f64::from(0.999_999f32)).0
    }

    // ExperienceOrb.setUnderwaterMovement uses float constants promoted to double.
    pub(super) fn underwater_movement(velocity: Vector3<f64>) -> Vector3<f64> {
        Vector3::new(
            velocity.x * f64::from(0.99f32),
            (velocity.y + f64::from(5.0e-4f32)).min(f64::from(0.06f32)),
            velocity.z * f64::from(0.99f32),
        )
    }

    // ExperienceOrb.unstuckIfPossible -> CollisionGetter.findFreePosition.
    pub(super) fn unstuck_if_possible(&self, max_distance: f64) {
        let entity = &self.entity;
        let size = Vector3::new(
            f64::from(entity.width()),
            f64::from(entity.height()),
            f64::from(entity.width()),
        );
        let center = entity.pos.load().add_raw(0.0, size.y / 2.0, 0.0);
        let allowed = BoundingBox::new(
            center.add_raw(
                -max_distance / 2.0,
                -max_distance / 2.0,
                -max_distance / 2.0,
            ),
            center.add_raw(max_distance / 2.0, max_distance / 2.0, max_distance / 2.0),
        );
        let world = entity.world.load();
        let (collisions, _) =
            world.get_block_collisions(allowed.expand(size.x, size.y, size.z), self);
        let (min_x, min_z, max_x, max_z) = {
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let limit = f64::from(border.portal_teleport_boundary);
            let half = border.new_diameter / 2.0;
            (
                (border.center_x - half).max(-limit),
                (border.center_z - half).max(-limit),
                (border.center_x + half).min(limit),
                (border.center_z + half).min(limit),
            )
        };
        let mut free = vec![allowed];
        for collision in collisions {
            // WorldBorder.isWithinBounds(AABB) checks both corners with a float epsilon.
            if collision.min.x < min_x
                || collision.min.z < min_z
                || collision.max.x - f64::from(1.0e-5f32) >= max_x
                || collision.max.z - f64::from(1.0e-5f32) >= max_z
            {
                continue;
            }
            let expanded = collision.expand(size.x / 2.0, size.y / 2.0, size.z / 2.0);
            free = free
                .into_iter()
                .flat_map(|area| subtract(area, expanded))
                .collect();
        }
        if let Some(closest) = free
            .iter()
            .map(|area| {
                // VoxelShape.closestPointTo / Mth.clamp; avoid f64::clamp's NaN-bound panic.
                Vector3::new(
                    center.x.max(area.min.x).min(area.max.x),
                    center.y.max(area.min.y).min(area.max.y),
                    center.z.max(area.min.z).min(area.max.z),
                )
            })
            .min_by(|a, b| {
                a.sub(&center)
                    .length_squared()
                    .total_cmp(&b.sub(&center).length_squared())
            })
        {
            entity.set_pos(closest.add_raw(0.0, -size.y / 2.0, 0.0));
        }
    }
}

impl Entity {
    /// Pushes toward the nearest open neighbour using `Entity.moveTowardsClosestSpace`'s direction order.
    /// Only updates velocity; callers set their synchronization flag when unsticking.
    pub fn move_towards_closest_space(&self, center: Vector3<f64>) {
        let position = BlockPos::floored_v(center);
        let delta = center.sub(&position.0.to_f64());
        let world = self.world.load();
        let mut direction = BlockDirection::Up;
        let mut closest = f64::MAX;
        for candidate in [
            BlockDirection::North,
            BlockDirection::South,
            BlockDirection::West,
            BlockDirection::East,
            BlockDirection::Up,
        ] {
            if world
                .get_block_state(&position.offset(candidate.to_offset()))
                .is_full_cube()
            {
                continue;
            }
            let component = delta.get_axis(candidate.to_axis().into());
            let distance = if candidate.positive() {
                1.0 - component
            } else {
                component
            };
            if distance < closest {
                closest = distance;
                direction = candidate;
            }
        }
        let speed = rand::random::<f32>() * 0.2 + 0.1;
        let mut movement = self.velocity.load() * 0.75;
        movement.set_axis(
            direction.to_axis().into(),
            f64::from(if direction.positive() { speed } else { -speed }),
        );
        self.velocity.store(movement);
    }
}

// Shapes.join(allowedCenters, expandedCollisions, ONLY_FIRST), with rectangular pieces.
fn subtract(mut area: BoundingBox, obstacle: BoundingBox) -> Vec<BoundingBox> {
    if !area.intersects(&obstacle) {
        return vec![area];
    }
    let mut pieces = Vec::with_capacity(6);
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let min = area.min.get_axis(axis);
        let max = area.max.get_axis(axis);
        let obstacle_min = obstacle.min.get_axis(axis).max(min);
        let obstacle_max = obstacle.max.get_axis(axis).min(max);
        if min < obstacle_min {
            let mut piece = area;
            piece.max.set_axis(axis, obstacle_min);
            pieces.push(piece);
            area.min.set_axis(axis, obstacle_min);
        }
        if max > obstacle_max {
            let mut piece = area;
            piece.min.set_axis(axis, obstacle_max);
            pieces.push(piece);
            area.max.set_axis(axis, obstacle_max);
        }
    }
    pieces
}
