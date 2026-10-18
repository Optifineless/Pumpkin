use super::LivingEntity;
use crate::entity::EntityBase;
use pumpkin_data::data_component_impl::{EquipmentType, EquippableImpl, IDSet, MobVisibilityImpl};
use pumpkin_data::tag::Taggable;
use std::sync::atomic::Ordering;

impl LivingEntity {
    /// Mirrors vanilla 26.3 `LivingEntity.getVisibilityPercent`.
    pub(crate) fn get_visibility_percent(&self, targeting_entity: Option<&dyn EntityBase>) -> f64 {
        let equipment = self.snapshot_equipped_stacks();
        let mut visibility = 1.0;
        if self.entity.is_sneaking() {
            visibility *= 0.8;
        }
        if self.entity.invisible.load(Ordering::Relaxed) {
            let covered = equipment
                .iter()
                .filter(|(slot, stack)| {
                    slot.slot_type() == EquipmentType::HumanoidArmor && !stack.is_empty()
                })
                .count();
            // getArmorCoverPercentage returns float, including its 0.1F minimum.
            let cover = (covered as f32 / 4.0).max(0.1);
            visibility *= 0.7 * f64::from(cover);
        }
        if let Some(targeting_entity) = targeting_entity {
            let entity_type = targeting_entity.get_entity().entity_type;
            for (slot, stack) in equipment {
                // ItemStack.getComponents returns EMPTY for a consumed stack.
                if stack.is_empty() {
                    continue;
                }
                let Some(component) = stack.get_data_component::<MobVisibilityImpl>() else {
                    continue;
                };
                let Some(equippable) = stack.get_data_component::<EquippableImpl>() else {
                    continue;
                };
                let matches = match &component.targeting_entity_types {
                    IDSet::Tag(tag) => entity_type.is_tagged_with(tag).unwrap_or(false),
                    IDSet::IDs(types) => types.contains(&entity_type),
                };
                if slot == *equippable.slot && matches {
                    visibility *= f64::from(component.visibility);
                }
            }
        }
        visibility.clamp(0.0, 10.0)
    }
}
