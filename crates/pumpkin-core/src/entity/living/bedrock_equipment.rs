use super::LivingEntity;
use pumpkin_data::data_component_impl::EquipmentSlot;

impl LivingEntity {
    pub(super) fn bedrock_equipment_slots(&self, slot: &EquipmentSlot) -> (u8, u8) {
        let selected = self
            .entity
            .world
            .load()
            .get_player_by_id(self.entity_id())
            .filter(|player| std::ptr::eq(&raw const player.living_entity, self))
            .map_or(0, |player| player.inventory().get_selected_slot());
        // MobEquipmentPacketPayload uses the container-local slot and current selected hotbar slot.
        let inventory_slot = if *slot == EquipmentSlot::MAIN_HAND {
            selected
        } else {
            0
        };
        (inventory_slot, selected)
    }
}
