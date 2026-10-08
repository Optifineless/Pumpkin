use super::{
    Control, MoveControlTrait,
    move_control::{MoveControl, Operation},
};
use crate::entity::mob::Mob;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

// Vec3.normalize and BlockGetter.forEachBlockIntersectedBetween.
const MIN_TRAVEL: f64 = 1.0e-5;

/// Ghast.GhastMoveControl, using swept collision shapes before accelerating.
#[derive(Default)]
pub struct GhastMoveControl {
    inner: MoveControl,
    float_duration: i32,
}
impl Control for GhastMoveControl {}
impl MoveControlTrait for GhastMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        if !self.has_wanted() {
            return;
        }
        let previous = self.float_duration;
        self.float_duration -= 1;
        if previous > 0 {
            return;
        }
        self.float_duration += rand::random_range(0..5) + 2;
        let entity = mob.get_entity();
        let travel = self.wanted_position().0 - entity.pos.load();
        if can_reach(mob, travel) {
            let speed = mob
                .get_mob_entity()
                .living_entity
                .get_attribute_value(&Attributes::FLYING_SPEED)
                * 5.0
                / 3.0;
            let acceleration = if travel.length() < MIN_TRAVEL {
                Vector3::new(0.0, 0.0, 0.0)
            } else {
                travel * (speed / travel.length())
            };
            entity.velocity.store(entity.velocity.load() + acceleration);
        } else {
            self.inner.operation = Operation::Wait;
        }
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.inner.set_wanted_position(x, y, z, speed);
    }
    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }
    fn wanted_position(&self) -> (Vector3<f64>, f64) {
        (
            Vector3::new(
                self.inner.wanted_x,
                self.inner.wanted_y,
                self.inner.wanted_z,
            ),
            self.inner.speed_modifier,
        )
    }
}

// GhastMoveControl.canReach / Entity.collidedWithShapeMovingFrom: test the whole
// segment against each collision shape, excluding blocks the mob already occupies.
fn can_reach(mob: &dyn Mob, travel: Vector3<f64>) -> bool {
    let entity = mob.get_entity();
    let bounds = entity.bounding_box.load();
    let world = entity.world.load();
    // GhastMoveControl.canReach -> BlockGetter.forEachBlockIntersectedBetween.
    visit_swept_blocks(bounds, travel, |pos| {
        if bounds.intersects(&BoundingBox::from_block(pos)) {
            return true;
        }
        let state = world.get_block_state(pos);
        for shape in state.get_block_collision_shapes_at(pos) {
            let obstacle = shape.shift(pos.to_f64());
            if swept_collision(bounds, travel, obstacle) {
                return false;
            }
        }
        true
    })
}

// Traverse the moving minimum corner, testing a conservative box at each crossed cell.
// The shape sweep rejects the extra boundary cells; reads grow with travel length, not box volume.
fn visit_swept_blocks(
    bounds: BoundingBox,
    delta: Vector3<f64>,
    mut visit: impl FnMut(&BlockPos) -> bool,
) -> bool {
    use crate::world::World;
    use rustc_hash::FxHashSet;
    use std::cell::RefCell;
    let size = bounds.max - bounds.min;
    let extent = Vector3::new(
        size.x.ceil() as i32,
        size.y.ceil() as i32,
        size.z.ceil() as i32,
    );
    let visited = RefCell::new(FxHashSet::default());
    let callback = RefCell::new(&mut visit);
    let check = |cell: &BlockPos, _| {
        let end = BlockPos(cell.0 + extent);
        for pos in BlockPos::iterate(*cell, end) {
            if visited.borrow_mut().insert(pos) && !callback.borrow_mut()(&pos) {
                return Some(());
            }
        }
        None
    };
    if delta.length_squared() < MIN_TRAVEL * MIN_TRAVEL {
        check(&bounds.min_block_pos(), None).is_none()
    } else {
        World::traverse_blocks(bounds.min, bounds.min + delta, check).is_none()
    }
}

fn swept_collision(bounds: BoundingBox, delta: Vector3<f64>, obstacle: BoundingBox) -> bool {
    use pumpkin_util::math::vector3::Axis;
    bounds.intersects(&obstacle)
        || [Axis::X, Axis::Y, Axis::Z].into_iter().any(|axis| {
            bounds
                .calculate_collision_time(&obstacle, delta, axis, 1.0)
                .is_some()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swept_flight_stops_at_thin_shapes_but_can_pass_beside_them() {
        let bounds = BoundingBox::new(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 1.0, 1.0));
        let obstacle = BoundingBox::new(Vector3::new(2.0, 0.0, 0.0), Vector3::new(2.125, 1.0, 1.0));
        assert!(swept_collision(
            bounds,
            Vector3::new(5.0, 0.0, 0.0),
            obstacle
        ));
        assert!(!swept_collision(
            bounds,
            Vector3::new(0.0, 0.0, 5.0),
            obstacle
        ));
    }

    #[test]
    fn diagonal_sweep_visits_every_crossed_block_without_scanning_the_stretched_volume() {
        use std::collections::HashSet;
        let bounds = BoundingBox::new(
            Vector3::new(0.25, 0.25, 0.25),
            Vector3::new(4.25, 4.25, 4.25),
        );
        let delta = Vector3::new(16.0, 12.0, -16.0);
        let mut visited = HashSet::new();
        assert!(visit_swept_blocks(bounds, delta, |pos| {
            visited.insert(*pos);
            true
        }));
        let swept = bounds.stretch(delta);
        let broad: Vec<_> =
            BlockPos::iterate(swept.min_block_pos(), swept.max_block_pos()).collect();
        for pos in &broad {
            if swept_collision(bounds, delta, BoundingBox::from_block(pos)) {
                assert!(visited.contains(pos), "missed {pos:?}");
            }
        }
        assert!(visited.len() < broad.len() / 2);
        assert!(!visit_swept_blocks(bounds, delta, |_| false));
    }
}
