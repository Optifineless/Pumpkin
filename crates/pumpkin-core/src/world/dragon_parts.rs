use std::sync::Arc;

use super::World;
use crate::entity::{EntityBase, boss::ender_dragon::EnderDragonEntity};

impl World {
    /// Resolves tracked entities, players and dragon parts; parts are never ticked or saved alone.
    pub fn get_entity_or_part(&self, id: i32) -> Option<Arc<dyn EntityBase>> {
        // ServerLevel.getEntityOrPart: regular lookup first, then dragonParts.
        self.get_entity_by_id(id).or_else(|| {
            self.dragon_parts
                .get(&id)
                .map(|part| part.clone() as Arc<dyn EntityBase>)
        })
    }

    pub(super) fn start_tracking_dragon_parts(&self, entity: &dyn EntityBase) {
        // ServerLevel.EntityCallbacks.onTrackingStart registers only accepted, tracked dragons.
        if let Some(dragon) = entity.cast_any().downcast_ref::<EnderDragonEntity>() {
            for part in &dragon.parts {
                self.dragon_parts
                    .insert(part.entity.entity_id, part.clone());
            }
        }
    }

    pub(crate) fn stop_tracking_dragon_parts(&self, entity: &dyn EntityBase) {
        // ServerLevel.EntityCallbacks.onTrackingEnd, including chunk unloads.
        if let Some(dragon) = entity.cast_any().downcast_ref::<EnderDragonEntity>() {
            for part in &dragon.parts {
                self.dragon_parts.remove(&part.entity.entity_id);
            }
        }
    }
}

#[cfg(test)]
#[path = "dragon_parts_tests.rs"]
mod tests;
