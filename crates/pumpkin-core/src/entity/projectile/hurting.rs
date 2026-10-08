use crate::entity::{Entity, EntityBase};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

pub(super) fn is_hurting(entity: &Entity) -> bool {
    [
        &EntityType::DRAGON_FIREBALL,
        &EntityType::FIREBALL,
        &EntityType::SMALL_FIREBALL,
        &EntityType::WITHER_SKULL,
        &EntityType::WIND_CHARGE,
        &EntityType::BREEZE_WIND_CHARGE,
    ]
    .contains(&entity.entity_type)
}

// AbstractHurtingProjectile.applyInertia and the WitherSkull / AbstractWindCharge overrides.
pub(super) fn movement(caller: &dyn EntityBase, acceleration: f64) -> Vector3<f64> {
    let entity = caller.get_entity();
    let wind =
        [&EntityType::WIND_CHARGE, &EntityType::BREEZE_WIND_CHARGE].contains(&entity.entity_type);
    let dangerous = caller
        .cast_any()
        .downcast_ref::<super::wither_skull::WitherSkullEntity>()
        .is_some_and(super::wither_skull::WitherSkullEntity::is_dangerous);
    let inertia = if wind {
        1.0
    } else if entity.is_in_water() {
        f64::from(0.8f32)
    } else if dangerous {
        f64::from(0.73f32)
    } else {
        f64::from(0.95f32)
    };
    let movement = entity.velocity.load();
    (movement + movement.normalize() * acceleration) * inertia
}

pub(super) fn before_move(caller: &dyn EntityBase) -> bool {
    let entity = caller.get_entity();
    if !is_hurting(entity) {
        return true;
    }
    // EntityReference.getEntity already rejects removed owners. Missing current chunks discard hurting projectiles.
    if !entity
        .world
        .load()
        .level
        .is_chunk_loaded(&entity.chunk_pos.load())
    {
        entity.remove();
        return false;
    }
    entity.velocity_dirty.store(true, Ordering::Relaxed);
    true
}

// AbstractHurtingProjectile.shouldBurn; WitherSkull and AbstractWindCharge return false.
pub(super) fn should_burn(entity: &Entity) -> bool {
    [&EntityType::FIREBALL, &EntityType::SMALL_FIREBALL].contains(&entity.entity_type)
}
