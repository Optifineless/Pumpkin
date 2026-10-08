//! Movement shared by vanilla control and travel implementations.
use crate::entity::{
    EntityBase,
    ai::{
        control::{MoveControlTrait, look_control::LookControlTrait},
        pathfinder::Navigator,
    },
    mob::{Mob, MobEntity},
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{PoisonError, atomic::Ordering};

/// Runs shared `AgeableMob`/Mob finalization after a species initializes its spawn state.
pub(crate) fn finalize_spawn_after_species<M: Mob + ?Sized>(
    mob: &M,
    world: &std::sync::Arc<crate::world::World>,
    view: &crate::world::spawn_view::SpawnView<'_>,
    group: Option<super::spawn::SpawnGroupData>,
) -> Option<super::spawn::SpawnGroupData> {
    super::finalize::finalize_spawn(mob, world, view, group)
}

impl MobEntity {
    /// Installs a species' navigation and move control, preserving its dimensions.
    pub fn configure_movement(
        &self,
        mut navigator: Navigator,
        control: impl MoveControlTrait + 'static,
    ) {
        let entity = &self.living_entity.entity;
        navigator.set_mob_dimensions(entity.width(), entity.height());
        navigator.configure_species_maluses(entity.entity_type);
        *self
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = navigator;
        *self
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(control);
    }

    /// Installs the species' look controller; existing goal look requests use it unchanged.
    pub fn configure_look(&self, control: impl LookControlTrait + 'static) {
        *self
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(control);
    }

    /// Mob.setSpeed updates both travel acceleration and forward input.
    pub fn set_speed(&self, speed: f32) {
        self.movement_speed.store(speed);
        let input = self.living_entity.movement_input.load();
        self.living_entity
            .movement_input
            .store(Vector3::new(input.x, input.y, f64::from(speed)));
    }
}

/// The moveRelative/move/drag sequence shared by aquatic travelInWater overrides.
pub fn travel_in_water(mob: &dyn Mob, caller: &dyn EntityBase, speed: f32, sink: bool) -> bool {
    let living = &mob.get_mob_entity().living_entity;
    let entity = &living.entity;
    if !entity.touching_water.load(Ordering::Relaxed) {
        return false;
    }
    entity.update_velocity_from_input(living.movement_input.load(), f64::from(speed));
    entity.move_entity(caller, entity.velocity.load());
    let mut velocity = entity.velocity.load() * 0.9;
    if sink {
        velocity.y -= 0.005;
    }
    entity.velocity.store(velocity);
    true
}

/// LivingEntity.travelFlying (26.3), including water and lava drag.
pub fn travel_flying(
    mob: &dyn Mob,
    caller: &dyn EntityBase,
    water_speed: f32,
    lava_speed: f32,
    air_speed: f32,
) -> bool {
    let living = &mob.get_mob_entity().living_entity;
    let entity = &living.entity;
    let (speed, drag) = if entity.touching_water.load(Ordering::Relaxed) {
        (water_speed, f64::from(0.8f32))
    } else if entity.touching_lava.load(Ordering::Relaxed) {
        (lava_speed, 0.5)
    } else {
        (air_speed, f64::from(0.91f32))
    };
    entity.update_velocity_from_input(living.movement_input.load(), f64::from(speed));
    entity.move_entity(caller, entity.velocity.load());
    entity.velocity.store(entity.velocity.load() * drag);
    true
}

/// Guardian.aiStep and Dolphin.tick use this land impulse (with different horizontal strengths).
pub fn flop_on_land(mob: &dyn Mob, horizontal: f32) {
    let entity = mob.get_entity();
    if entity.touching_water.load(Ordering::Relaxed) || !entity.on_ground.load(Ordering::Relaxed) {
        return;
    }
    entity.velocity.store(
        entity.velocity.load()
            + Vector3::new(
                f64::from((rand::random::<f32>() * 2.0 - 1.0) * horizontal),
                0.5,
                f64::from((rand::random::<f32>() * 2.0 - 1.0) * horizontal),
            ),
    );
    entity.yaw.store(rand::random::<f32>() * 360.0);
    entity.on_ground.store(false, Ordering::Relaxed);
    entity.velocity_dirty.store(true, Ordering::Relaxed);
}
