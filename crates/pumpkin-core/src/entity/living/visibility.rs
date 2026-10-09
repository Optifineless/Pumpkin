use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::data_component_impl::{EquipmentSlot, EquippableImpl, IDSet, MobVisibilityImpl};
use pumpkin_inventory::entity_equipment::EntityEquipment;

use super::LivingEntity;
use crate::entity::{Entity, death_loot::equipment_slots_in_vanilla_order};
use pumpkin_data::tag::Taggable;

const HUMANOID_ARMOR: [EquipmentSlot; 4] = [
    EquipmentSlot::HEAD,
    EquipmentSlot::CHEST,
    EquipmentSlot::LEGS,
    EquipmentSlot::FEET,
];

impl LivingEntity {
    /// Returns the target's visibility multiplier for the given observer.
    #[must_use]
    pub fn visibility_percent(&self, targeting_entity: Option<&Entity>) -> f64 {
        // LivingEntity.getVisibilityPercent (26.3): item MOB_VISIBILITY, matching EQUIPPABLE slot.
        let mut percent = if self.entity.is_sneaking() { 0.8 } else { 1.0 };
        let equipment = self
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.invisible.load(Relaxed) {
            let cover = Self::armor_cover_percentage(&equipment).max(0.1);
            percent *= 0.7 * f64::from(cover);
        }
        if let Some(observer) = targeting_entity {
            for slot in &equipment_slots_in_vanilla_order() {
                if let Some(stack) = equipment.equipment.get(slot)
                    && !stack.is_empty()
                    && let Some(visibility) = stack.get_data_component::<MobVisibilityImpl>()
                    && let Some(equippable) = stack.get_data_component::<EquippableImpl>()
                    && slot == equippable.slot
                    && match &visibility.targeting_entity_types {
                        IDSet::Tag(tag) => {
                            observer.entity_type.is_tagged_with(tag).unwrap_or(false)
                        }
                        IDSet::IDs(types) => types.contains(&observer.entity_type),
                    }
                {
                    percent *= f64::from(visibility.visibility);
                }
            }
        }
        percent.clamp(0.0, 10.0)
    }

    fn armor_cover_percentage(equipment: &EntityEquipment) -> f32 {
        // LivingEntity.getArmorCoverPercentage counts nonempty humanoid armor slots.
        let worn = HUMANOID_ARMOR
            .iter()
            .filter(|slot| {
                equipment
                    .equipment
                    .get(*slot)
                    .is_some_and(|stack| !stack.is_empty())
            })
            .count();
        worn as f32 / HUMANOID_ARMOR.len() as f32
    }
}
