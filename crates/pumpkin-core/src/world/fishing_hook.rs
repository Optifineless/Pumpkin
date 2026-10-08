use super::World;
use crate::entity::{Entity, projectile::fishing_bobber::FishingBobberEntity};
use pumpkin_data::entity::EntityType;

impl World {
    // FishingHook.remove/updateOwnerInfo runs before the hook leaves the entity list.
    pub(super) fn clear_fishing_hook_owner(&self, entity: &Entity) {
        if entity.entity_type == &EntityType::FISHING_BOBBER
            && let Some(hook) = self.get_entity_by_id(entity.entity_id)
            && let Some(hook) = hook.cast_any().downcast_ref::<FishingBobberEntity>()
        {
            hook.clear_owner();
        }
    }
}
