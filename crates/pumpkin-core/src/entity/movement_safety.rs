use super::Entity;
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use std::sync::atomic::Ordering::Relaxed;

// Deliberate safety divergence: Entity.move/collideBoundingBox has no generic speed cap.
// Keep even malformed plugin impulses below a fixed amount of synchronous block work.
const MAX_MOVEMENT_PER_AXIS: f64 = 128.0;
// Allow large vanilla hitboxes and finite stacked explosion launches such as (40,40,40).
const MAX_ADDITIONAL_COLLISION_BLOCKS: f64 = 262_144.0;

impl Entity {
    /// Rejects non-finite or oversized collision sweeps and clears their pending motion.
    pub(crate) fn accept_movement(&self, movement: Vector3<f64>) -> bool {
        self.accept_collision_sweep(self.bounding_box.load().stretch(movement), movement)
    }

    /// Checks the actual query bounds, including a caller's collision margin, before block reads.
    pub(crate) fn accept_collision_sweep(
        &self,
        swept: BoundingBox,
        movement: Vector3<f64>,
    ) -> bool {
        if movement_is_bounded(self.bounding_box.load(), swept, movement) {
            return true;
        }
        self.velocity.store(Vector3::default());
        self.velocity_dirty.store(true, Relaxed);
        self.horizontal_collision.store(false, Relaxed);
        self.vertical_collision.store(false, Relaxed);
        false
    }
}

fn movement_is_bounded(bounds: BoundingBox, swept: BoundingBox, movement: Vector3<f64>) -> bool {
    if [movement.x, movement.y, movement.z]
        .into_iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_MOVEMENT_PER_AXIS)
    {
        return false;
    }
    match (collision_cells(bounds), collision_cells(swept)) {
        (Some(base), Some(expanded)) => expanded - base <= MAX_ADDITIONAL_COLLISION_BLOCKS,
        _ => false,
    }
}

fn collision_cells(bounds: BoundingBox) -> Option<f64> {
    // Use World.get_block_collisions' inclusive cells, subtracting the entity's own query.
    let min = bounds.min.add_raw(0.0, -0.50001, 0.0);
    let max = bounds.max.add_raw(-1.0e-9, -1.0e-9, -1.0e-9);
    let mut cells = 1.0;
    for (min, max) in [(min.x, max.x), (min.y, max.y), (min.z, max.z)] {
        if !min.is_finite()
            || !max.is_finite()
            || min < f64::from(i32::MIN)
            || max >= f64::from(i32::MAX)
            || min > max
        {
            return None;
        }
        cells *= max.floor() - min.floor() + 1.0;
    }
    Some(cells)
}
