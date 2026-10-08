use std::sync::Arc;

use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::world::World;

/// Enchantment entity effect that sets an entity on fire for a duration calculated from the level.
#[derive(Clone, Debug, PartialEq)]
pub struct Ignite {
    pub duration: LevelBasedValue,
}

impl Ignite {
    #[must_use]
    pub const fn new(duration: LevelBasedValue) -> Self {
        Self { duration }
    }

    /// Applies the ignite effect to an entity for the given enchantment level.
    pub fn apply_to_entity(&self, level: i32, entity: &Entity) {
        let seconds = self.duration.calculate(level);
        if let Some(target) = entity.world.load().get_entity_by_id(entity.entity_id) {
            ignite_target(target.as_ref(), seconds);
        } else {
            ignite_target(entity, seconds);
        }
    }
}

impl EnchantmentEntityEffectExt for Ignite {
    fn apply(
        &self,
        _world: &Arc<World>,
        enchantment_level: i32,
        _owner: Option<&Arc<Player>>,
        entity: Option<&Entity>,
        _position: Vector3<f64>,
    ) {
        if let Some(entity) = entity {
            self.apply_to_entity(enchantment_level, entity);
        }
    }
}

// Ignite.apply dispatches through LivingEntity.igniteForTicks for living targets.
pub(crate) fn ignite_target(target: &dyn EntityBase, seconds: f32) {
    target.set_on_fire_for(seconds);
    let entity = target.get_entity();
    entity.set_on_fire(entity.fire_ticks.load(std::sync::atomic::Ordering::Relaxed) > 0);
}
