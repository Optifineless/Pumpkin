use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::data_component_impl::{AxolotlVariantImpl, BucketEntityDataImpl, CustomNameImpl};
use pumpkin_data::{entity::EntityType, item_stack::ItemStack, sound::Sound};
use pumpkin_nbt::compound::NbtCompound;

use super::axolotl::{AxolotlEntity, AxolotlVariant};
use crate::entity::{ai::brain::memory::types, mob::Mob};

/// Restores MobBucketItem.spawn's bucket data after finalization and marks bucket persistence.
pub fn load_from_bucket(mob: &dyn Mob, stack: &ItemStack) {
    let entity = mob.get_entity();
    if let Some(name) = stack.get_data_component::<CustomNameImpl>() {
        entity.set_custom_name(name.name.clone());
    }
    let empty = NbtCompound::new();
    let tag = stack
        .get_data_component::<BucketEntityDataImpl>()
        .and_then(|data| data.nbt.as_ref())
        .unwrap_or(&empty);
    load_default_data(mob, tag);
    // EntityType.createDefaultStackConfig applies implicit variants before spawning.
    if let Some(fish) = mob
        .cast_any()
        .downcast_ref::<super::tropical_fish::TropicalFishEntity>()
    {
        fish.apply_bucket_components(stack);
    }
    if let Some(fish) = mob.cast_any().downcast_ref::<super::salmon::SalmonEntity>() {
        fish.apply_bucket_components(stack);
    }
    // AbstractFish.setFromBucket and requiresCustomPersistence.
    mob.set_spawned_from_bucket(true);
    if let Some(ageable) = mob.as_ageable() {
        if let Some(age) = tag.get_int("Age") {
            ageable.set_age(age);
        }
        ageable.set_age_locked(tag.get_bool("AgeLocked").unwrap_or(false));
    }
    if let Some(axolotl) = mob.cast_any().downcast_ref::<AxolotlEntity>() {
        // Axolotl.applyImplicitComponents precedes Axolotl.loadFromBucketTag.
        if let Some(variant) = stack.get_data_component::<AxolotlVariantImpl>() {
            let variant = AxolotlVariant::from_id(variant.variant_id().unwrap_or(0));
            axolotl.set_variant(variant);
        }
        axolotl.set_from_bucket(true);
        let mut brain = mob
            .get_mob_entity()
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        brain.register_memory(types::HAS_HUNTING_COOLDOWN.id());
        if let Some(ticks) = tag.get_long("HuntingCooldown") {
            brain.set_with_expiry(types::HAS_HUNTING_COOLDOWN, true, ticks);
        } else {
            brain.erase(types::HAS_HUNTING_COOLDOWN.id());
        }
    }
}

// Bucketable.loadDefaultDataFromBucketTag only restores this whitelist, never arbitrary entity NBT.
fn load_default_data(mob: &dyn Mob, tag: &NbtCompound) {
    let entity = mob.get_entity();
    let mob_entity = mob.get_mob_entity();
    if let Some(value) = tag.get_bool("NoAI") {
        mob_entity.set_no_ai(value);
    }
    if let Some(value) = tag.get_bool("Silent") {
        entity.set_silent(value);
    }
    if let Some(value) = tag.get_bool("NoGravity") {
        entity.set_has_no_gravity(value);
    }
    if let Some(value) = tag.get_bool("Glowing") {
        entity.set_glowing(value);
    }
    if let Some(value) = tag.get_bool("Invulnerable") {
        entity.set_invulnerable(value);
    }
    if tag.get_bool("PersistenceRequired") == Some(true) {
        mob_entity.persistence_required.store(true, Relaxed);
    }
    if let Some(value) = tag.get_float("Health") {
        mob_entity.living_entity.set_health(value);
    }
}

/// Mob.playAmbientSound dispatch for the bucketable classes.
pub fn ambient_sound(mob: &dyn Mob) -> Option<Sound> {
    let entity = mob.get_entity();
    if entity.is_silent() {
        return None;
    }
    match entity.entity_type {
        kind if kind == &EntityType::AXOLOTL => Some(if entity.touching_water.load(Relaxed) {
            Sound::EntityAxolotlIdleWater
        } else {
            Sound::EntityAxolotlIdleAir
        }),
        kind if kind == &EntityType::COD => Some(Sound::EntityCodAmbient),
        kind if kind == &EntityType::SALMON => Some(Sound::EntitySalmonAmbient),
        kind if kind == &EntityType::TROPICAL_FISH => Some(Sound::EntityTropicalFishAmbient),
        _ => None,
    }
}

/// Computes EntityType.getYOffset for bucket spawning with tryMoveDown=true and movedUp=false.
pub fn spawn_position(
    world: &crate::world::World,
    entity: &dyn crate::entity::EntityBase,
    pos: pumpkin_util::math::position::BlockPos,
) -> pumpkin_util::math::vector3::Vector3<f64> {
    use pumpkin_util::math::{
        boundingbox::BoundingBox,
        vector3::{Axis, Vector3},
    };
    let bounds = entity.get_entity().bounding_box.load();
    let movement = Vector3::new(0.0, -1.0, 0.0);
    let mut time = 1.0;
    for collision in world
        .get_block_collisions(BoundingBox::from_block(&pos), entity)
        .0
    {
        if let Some(next) = bounds.calculate_collision_time(&collision, movement, Axis::Y, time) {
            time = next;
        }
    }
    Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y) + 1.0 - time,
        f64::from(pos.0.z) + 0.5,
    )
}
