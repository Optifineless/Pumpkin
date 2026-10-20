use super::{
    HappyGhastEntity,
    goals::{HappyFloatGoal, RandomFloatAroundGoal, TemptGoal},
};
use crate::entity::{
    ageable::AgeableMob,
    ai::{
        control::{
            Control, MoveControlTrait,
            ghast_move_control::{swept_collision, visit_swept_blocks},
            happy_ghast_look_control::HappyGhastLookControl,
            look_control::LookControl,
            move_control::MoveControl,
        },
        pathfinder::Navigator,
    },
    mob::Mob,
};
use pumpkin_data::{
    attributes::Attributes,
    fluid::Fluid,
    tag::{self, Taggable},
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use rand::RngExt;
use std::sync::{PoisonError, atomic::Ordering::Relaxed};

impl HappyGhastEntity {
    pub(crate) fn facing_target(&self) -> Option<Vector3<f64>> {
        self.facing_target.load()
    }

    pub(super) fn uses_adult_flight(&self) -> bool {
        self.flight_enabled && !self.is_baby()
    }

    // Reconcile before serverAiStep takes its selectors; age/equipment remain common-owned.
    pub(super) fn reconcile_flight(&self) {
        let adult = self.uses_adult_flight();
        if self.adult_flight.swap(adult, Relaxed) == adult {
            return;
        }
        let mob = &self.mob_entity;
        mob.clear_ai_goals(self);
        mob.navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        if adult {
            mob.configure_movement(Navigator::ground(), HappyGhastMoveControl::default());
            mob.configure_look(HappyGhastLookControl::default());
            let mut goals = mob
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            goals.add_goal(3, Box::new(HappyFloatGoal::default()));
            goals.add_goal(4, Box::new(TemptGoal::default()));
            goals.add_goal(5, Box::new(RandomFloatAroundGoal));
        } else {
            mob.configure_movement(Navigator::ground(), MoveControl::default());
            mob.configure_look(LookControl::default());
            self.install_legacy_goals();
        }
    }

    pub(super) fn random_float(&self) -> f32 {
        #[cfg(test)]
        if let Some(value) = self
            .random_draws
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            return value as f32;
        }
        self.get_random().random()
    }

    pub(super) fn random_double(&self) -> f64 {
        #[cfg(test)]
        if let Some(value) = self
            .random_draws
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            return value;
        }
        self.get_random().random()
    }
}

/// Ghast.GhastMoveControl(careful=true), kept separate from the hostile controller.
#[derive(Default)]
pub(super) struct HappyGhastMoveControl {
    inner: MoveControl,
    float_duration: i32,
}
impl Control for HappyGhastMoveControl {}
impl MoveControlTrait for HappyGhastMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        if mob
            .cast_any()
            .downcast_ref::<HappyGhastEntity>()
            .is_some_and(HappyGhastEntity::is_on_still_timeout)
        {
            self.set_wait();
            mob.get_mob_entity().movement_speed.store(0.0);
            mob.get_mob_entity()
                .living_entity
                .movement_input
                .store(Vector3::new(0.0, 0.0, 0.0));
            entity.velocity.store(Vector3::new(0.0, 0.0, 0.0));
        }
        if !self.has_wanted() {
            return;
        }
        let previous = self.float_duration;
        self.float_duration -= 1;
        if previous > 0 {
            return;
        }
        self.float_duration += mob.get_random().random_range(0..5) + 2;
        let travel = self.wanted_position().0 - entity.pos.load();
        if can_reach(mob, travel) {
            let speed = mob
                .get_mob_entity()
                .living_entity
                .get_attribute_value(&Attributes::FLYING_SPEED)
                * 5.0
                / 3.0;
            if travel.length() >= 1.0e-5 {
                entity
                    .velocity
                    .store(entity.velocity.load() + travel * (speed / travel.length()));
            }
        } else {
            self.set_wait();
        }
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.inner.set_wanted_position(x, y, z, speed);
    }
    fn set_wait(&mut self) {
        self.inner.operation = crate::entity::ai::control::move_control::Operation::Wait;
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

fn can_reach(mob: &dyn Mob, travel: Vector3<f64>) -> bool {
    let entity = mob.get_entity();
    let bounds = entity.bounding_box.load();
    let destination = bounds.shift(travel).expand(1.0, 1.0, 1.0);
    for pos in BlockPos::iterate(
        BlockPos::floored_v(destination.min),
        BlockPos::floored_v(destination.max),
    ) {
        if !block_traversal_possible(mob, &pos, None) {
            return false;
        }
    }
    visit_swept_blocks(bounds, travel, |pos| {
        bounds.intersects(&BoundingBox::from_block(pos))
            || block_traversal_possible(mob, pos, Some((bounds, travel)))
    })
}

fn block_traversal_possible(
    mob: &dyn Mob,
    pos: &BlockPos,
    path: Option<(BoundingBox, Vector3<f64>)>,
) -> bool {
    let entity = mob.get_entity();
    let world = entity.world.load();
    let Some(state) = world.get_block_state_if_loaded(pos) else {
        return false;
    };
    if state.is_air() {
        return true;
    }
    if state
        .id
        .to_block()
        .has_tag(&tag::Block::MINECRAFT_HAPPY_GHAST_AVOIDS)
    {
        return false;
    }
    let origin = Vector3::new(f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z));
    let clear = state.get_block_collision_shapes_at(pos).all(|shape| {
        path.is_some_and(|(bounds, travel)| {
            !collided_with_shape_moving_from(bounds, travel, shape.shift(origin))
        })
    });
    let (fluid, fluid_state) = crate::world::World::fluid_state_from_block_state(state.id);
    if !fluid_state.is_empty {
        if world.get_block_state_if_loaded(&pos.up()).is_none() {
            return false;
        }
        let height = f64::from(world.get_fluid_height(pos, fluid, &fluid_state));
        let fluid_box = BoundingBox::new(origin, origin + Vector3::new(1.0, height, 1.0));
        if path.is_none_or(|(bounds, travel)| {
            collided_with_shape_moving_from(bounds, travel, fluid_box)
        }) {
            return path.is_some()
                && ((fluid.matches_type(&Fluid::WATER) && entity.touching_water.load(Relaxed))
                    || (fluid.matches_type(&Fluid::LAVA) && entity.touching_lava.load(Relaxed)));
        }
    }
    clear
}

// AABB.collidedAlongVector requires overlap on stationary axes, not mere face contact.
fn collided_with_shape_moving_from(
    bounds: BoundingBox,
    travel: Vector3<f64>,
    shape: BoundingBox,
) -> bool {
    for (low, high, delta, shape_low, shape_high) in [
        (
            bounds.min.x,
            bounds.max.x,
            travel.x,
            shape.min.x,
            shape.max.x,
        ),
        (
            bounds.min.y,
            bounds.max.y,
            travel.y,
            shape.min.y,
            shape.max.y,
        ),
        (
            bounds.min.z,
            bounds.max.z,
            travel.z,
            shape.min.z,
            shape.max.z,
        ),
    ] {
        if (delta <= 0.0 && high <= shape_low) || (delta >= 0.0 && low >= shape_high) {
            return false;
        }
    }
    swept_collision(bounds, travel, shape)
}
