//! What every fish shares: vanilla's `AbstractFish`.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::ai::control::fish_move_control::FishMoveControl;
use crate::entity::ai::goal::{
    avoid_entity::AvoidEntityGoal, escape_danger::EscapeDangerGoal, wander_around::WanderAroundGoal,
};
use crate::entity::ai::pathfinder::Navigator;
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::mob::{Mob, MobEntity};

/// Vanilla's AbstractFish.registerGoals and createNavigation.
pub fn init(mob_entity: &MobEntity) {
    let mut navigator = Navigator::water_bound(false);
    navigator.set_pathfinding_malus(PathType::Water, 0.0);
    let entity = &mob_entity.living_entity.entity;
    navigator.set_mob_dimensions(entity.width(), entity.height());
    *mob_entity
        .navigator
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = navigator;
    *mob_entity
        .move_control
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Box::new(FishMoveControl::default());

    let mut goals = mob_entity
        .goals_selector
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    goals.add_goal(0, EscapeDangerGoal::new(1.25));
    goals.add_goal(
        2,
        Box::new(AvoidEntityGoal::new(&EntityType::PLAYER, 8.0, 1.6, 1.4)),
    );
    goals.add_goal(4, Box::new(WanderAroundGoal::swimming(1.0, 40)));
}

/// Vanilla 26.3's AbstractFish.travelInWater; dry fish use normal land travel.
pub fn travel(mob: &dyn Mob, caller: &dyn EntityBase) -> bool {
    let mob_entity = mob.get_mob_entity();
    let living = &mob_entity.living_entity;
    let entity = &living.entity;
    if !entity.touching_water.load(Ordering::Relaxed) {
        return false;
    }

    entity.update_velocity_from_input(living.movement_input.load(), f64::from(0.01f32));
    entity.move_entity(caller, entity.velocity.load());

    let mut velocity = entity.velocity.load() * 0.9;
    let chasing = mob_entity
        .target
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    if !chasing {
        velocity.y -= 0.005;
    }
    entity.velocity.store(velocity);
    true
}

/// Vanilla's `AbstractFish.aiStep` on dry land, where a stranded fish hops about.
pub fn flop(mob: &dyn Mob, sound: Sound) {
    let entity = mob.get_entity();
    if entity.touching_water.load(Ordering::Relaxed)
        || !entity.on_ground.load(Ordering::Relaxed)
        || !entity.vertical_collision.load(Ordering::Relaxed)
    {
        return;
    }
    let mut rng = mob.get_random();
    let velocity = entity.velocity.load();
    entity.velocity.store(Vector3::new(
        velocity.x + f64::from((rng.random::<f32>() * 2.0 - 1.0) * 0.05),
        velocity.y + f64::from(0.4f32),
        velocity.z + f64::from((rng.random::<f32>() * 2.0 - 1.0) * 0.05),
    ));
    entity.on_ground.store(false, Ordering::Relaxed);
    entity.velocity_dirty.store(true, Ordering::Relaxed);
    // LivingEntity.makeSound / getVoicePitch, then Entity.playSound's silent check.
    let pitch = (rng.random::<f32>() - rng.random::<f32>()) * 0.2 + 1.0;
    if !entity.is_silent() {
        entity.world.load().play_sound_fine(
            sound,
            SoundCategory::Neutral,
            &entity.pos.load(),
            1.0,
            pitch,
        );
    }
}
