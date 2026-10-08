use super::strider::StriderEntity;
use crate::entity::EntityBase;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use std::sync::atomic::Ordering::Relaxed;

// Strider.getLiquidCollisionShape: Block.column(16, 0, 8).
const LIQUID_COLLISION_HEIGHT: f64 = 0.5;

fn above_surface(bottom: f64, block_y: i32) -> bool {
    crate::world::collision_shapes::is_above(bottom, block_y, LIQUID_COLLISION_HEIGHT)
}

impl StriderEntity {
    // Strider.tick runs floatStrider after the superclass tick.
    pub(super) fn float_strider(&self) {
        let entity = self.get_entity();
        if !entity.touching_lava.load(Relaxed) {
            return;
        }
        let pos = entity.block_pos.load();
        let above_is_lava = entity
            .world
            .load()
            .get_fluid(&pos.up())
            .has_tag(&tag::Fluid::MINECRAFT_LAVA);
        if above_surface(entity.bounding_box.load().min.y, pos.0.y) && !above_is_lava {
            entity.on_ground.store(true, Relaxed);
        } else {
            entity
                .velocity
                .store(submerged_velocity(entity.velocity.load()));
        }
    }
}

fn submerged_velocity(velocity: Vector3<f64>) -> Vector3<f64> {
    velocity * 0.5 + Vector3::new(0.0, 0.05, 0.0)
}

/// Returns the local source-lava support shape for `World::get_block_collisions`.
///
/// Mirrors `LiquidBlock.getCollisionShape` and `EntityCollisionContext.canStandOnFluid`;
/// callers shift the shape by `pos` and intersect it with the swept movement bounds.
pub fn liquid_collision_shape(entity: &dyn EntityBase, pos: &BlockPos) -> Option<BoundingBox> {
    let mob = entity.get_mob()?;
    let world = entity.get_entity().world.load();
    let (fluid, state) = world.get_fluid_and_fluid_state(pos);
    if !state.is_source
        || !mob.can_stand_on_fluid(fluid)
        || fluid.matches_type(world.get_fluid(&pos.up()))
        || !above_surface(entity.get_entity().bounding_box.load().min.y, pos.0.y)
    {
        return None;
    }
    Some(BoundingBox::new(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, LIQUID_COLLISION_HEIGHT, 1.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strider_buoyancy_and_surface_tolerance() {
        assert_eq!(
            submerged_velocity(Vector3::new(0.4, -0.1, -0.2)),
            Vector3::new(0.2, 0.0, -0.1)
        );
        assert!(above_surface(60.5, 60));
        assert!(!above_surface(60.499, 60));
    }
}
