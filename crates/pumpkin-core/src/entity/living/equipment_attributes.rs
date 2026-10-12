use super::LivingEntity;

impl LivingEntity {
    pub(crate) fn reconcile_player_equipment_attributes(&self) {
        if let Some(player) = self
            .entity
            .world
            .load()
            .get_player_by_uuid(self.entity.entity_uuid)
            && std::ptr::eq(
                std::ptr::from_ref(&player.living_entity),
                std::ptr::from_ref(self),
            )
        {
            player.reconcile_equipment_attributes();
        }
    }
}
