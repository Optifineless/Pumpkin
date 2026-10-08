use super::{
    EnchantmentHelper,
    definition::{definition_matches_slot, enchantment_definition},
};
use crate::entity::{equipment_damage::EquippedItem, player::Player};
use pumpkin_data::data_component_impl::EquipmentSlot;
use rand::RngExt;

impl EnchantmentHelper {
    /// Selects a damaged equipped item with a slot-matching `REPAIR_WITH_XP` effect.
    /// Each matching enchantment contributes a candidate, as in `EnchantmentHelper.getRandomItemWith`.
    pub fn get_random_item_with_repair_effect(player: &Player) -> Option<EquippedItem> {
        let world = player.world();
        let mut candidates = Vec::new();
        for slot in [
            EquipmentSlot::MAIN_HAND,
            EquipmentSlot::OFF_HAND,
            EquipmentSlot::FEET,
            EquipmentSlot::LEGS,
            EquipmentSlot::CHEST,
            EquipmentSlot::HEAD,
            EquipmentSlot::BODY,
            EquipmentSlot::SADDLE,
        ] {
            let item = EquippedItem::capture(player, &slot);
            if item.stack.is_empty()
                || !item.stack.is_damageable()
                || item.stack.is_unbreakable()
                || item.stack.get_damage() <= 0
            {
                continue;
            }
            Self::run_iteration_on_item(&item.stack, |enchantment, _level| {
                if let Some(definition) = enchantment_definition(Some(&world), enchantment)
                    && definition_matches_slot(&definition, &slot)
                    && definition
                        .get_compound("effects")
                        .is_some_and(|effects| effects.get("minecraft:repair_with_xp").is_some())
                {
                    candidates.push(item.clone());
                }
            });
        }
        if candidates.is_empty() {
            None
        } else {
            let index = rand::rng().random_range(0..candidates.len());
            Some(candidates.swap_remove(index))
        }
    }
}
