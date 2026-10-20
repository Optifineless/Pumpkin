use super::{FlyingPathNavigation, Navigator, PathNavigationTrait, navigation_geometry};
use crate::{entity::EntityBase, world::World};
use pumpkin_util::math::vector3::Vector3;

impl Navigator {
    /// Creates `HappyGhast.BabyFlyingPathNavigation` without changing other flying navigators.
    #[must_use]
    pub fn ghastling() -> Self {
        let mut navigation = FlyingPathNavigation::new();
        navigation.inner.ghastling = true;
        navigation.set_can_open_doors(false);
        navigation.set_can_float(true);
        navigation.set_required_path_length(48.0);
        Self::new(navigation)
    }
}

// BabyFlyingPathNavigation.canMoveDirectly: COLLIDER shapes, Fluid.NONE.
pub(super) fn can_move_directly(
    caller: &dyn EntityBase,
    start: Vector3<f64>,
    stop: Vector3<f64>,
) -> bool {
    let entity = caller.get_entity();
    let world = entity.world.load();
    let end = Vector3::new(stop.x, stop.y + f64::from(entity.height()) * 0.5, stop.z);
    World::traverse_blocks(start, end, |pos, _| {
        let Some(state) = world.get_block_state_if_loaded(pos) else {
            return Some(());
        };
        let mut hit = false;
        world.for_each_collision_shape(state, pos, Some(caller), &mut |shape| {
            hit |= navigation_geometry::clips_movement_shape(start, end, shape.at_pos(*pos));
        });
        hit.then_some(())
    })
    .is_none()
}

#[cfg(test)]
#[path = "ghastling_navigation_tests.rs"]
mod tests;
