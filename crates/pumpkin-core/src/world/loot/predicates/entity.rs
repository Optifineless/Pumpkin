use super::{item_predicate, registry_matches};
use crate::world::loot::{EntityLootState, LootContextParameters};
use pumpkin_data::item_stack::ItemStack;
use serde_json::Value;
pub(in crate::world::loot) fn entity_predicate(
    predicate: &Value,
    entity: &EntityLootState,
) -> Option<bool> {
    entity_predicate_with_ancestors(predicate, entity, &[])
}
fn entity_predicate_with_ancestors<'a>(
    predicate: &Value,
    entity: &'a EntityLootState,
    ancestors: &[&'a EntityLootState],
) -> Option<bool> {
    // EntityPredicate.combine, EntityFlagsPredicate, VehiclePredicate and PassengerPredicate.
    if predicate.is_null() {
        return Some(true);
    }
    let entity = ancestors
        .iter()
        .copied()
        .find(|ancestor| entity.entity_id.is_some() && ancestor.entity_id == entity.entity_id)
        .unwrap_or(entity);
    let mut ancestors = ancestors.to_vec();
    ancestors.push(entity);
    for (key, value) in predicate.as_object()? {
        let matched = match key.trim_start_matches("minecraft:") {
            "entity_type" | "type" => entity
                .entity_type
                .is_some_and(|kind| registry_matches(value, kind)),
            "components" => {
                for (name, expected) in value.as_object()? {
                    let actual = entity.components.get(name);
                    if actual.is_none()
                        && !pumpkin_util::loot_table::SUPPORTED_ENTITY_COMPONENTS
                            .contains(&name.as_str())
                    {
                        // Unavailable component implementations must also fail under inversion.
                        return None;
                    }
                    if actual != Some(expected) {
                        return Some(false);
                    }
                }
                true
            }
            "flags" => {
                for (flag, expected) in value.as_object()? {
                    if entity.is_living == Some(false)
                        && matches!(flag.as_str(), "is_baby" | "is_fall_flying")
                    {
                        continue;
                    }
                    if entity.flags.get(flag).copied()? != expected.as_bool()? {
                        return Some(false);
                    }
                }
                true
            }
            "type_specific/fishing_hook" => {
                // FishingHookPredicate.CODEC/matches ignore unknown keys and accept no requirement.
                value.as_object()?;
                value.get("in_open_water").is_none_or(|expected| {
                    entity
                        .fishing_open_water
                        .is_some_and(|actual| Some(actual) == expected.as_bool())
                })
            }
            "type_specific/sheep" => {
                if value.as_object()?.keys().any(|key| key != "sheared") {
                    return None;
                }
                let Some(sheared) = entity.sheared else {
                    return Some(false);
                };
                value
                    .get("sheared")
                    .and_then(Value::as_bool)
                    .is_none_or(|expected| expected == sheared)
            }
            "vehicle" => match entity.vehicle.as_deref() {
                Some(vehicle) => entity_predicate_with_ancestors(value, vehicle, &ancestors)?,
                None => false,
            },
            "passenger" => {
                let mut matches = false;
                for passenger in &entity.passengers {
                    if entity_predicate_with_ancestors(value, passenger, &ancestors)? {
                        matches = true;
                        break;
                    }
                }
                matches
            }
            "equipment" => {
                if entity.is_living != Some(true) {
                    return Some(false);
                }
                for (slot, predicate) in value.as_object()? {
                    let stack = entity.equipment.get(slot).unwrap_or(ItemStack::EMPTY);
                    if !item_predicate(predicate, stack)? {
                        return Some(false);
                    }
                }
                true
            }
            _ => return None,
        };
        if !matched {
            return Some(false);
        }
    }
    Some(true)
}
pub(in crate::world::loot) fn enchantment_level(stack: Option<&ItemStack>, name: &str) -> i32 {
    let name = name.trim_start_matches("minecraft:");
    stack
        .map_or(0, |stack| {
            pumpkin_data::Enchantment::from_name(name).map_or(0, |e| stack.get_enchantment_level(e))
        })
        .max(0)
}
pub(in crate::world::loot) fn attacker_enchantment_level(
    params: &LootContextParameters,
    name: &str,
) -> i32 {
    // EnchantmentHelper.getEnchantmentLevel uses Enchantment.getSlotItems, excluding empty stacks.
    let Some(entity) = params
        .attacking_entity_state
        .as_ref()
        .filter(|entity| entity.is_living == Some(true))
    else {
        return 0;
    };
    let Some(enchantment) =
        pumpkin_data::Enchantment::from_name(name.trim_start_matches("minecraft:"))
    else {
        return 0;
    };
    entity
        .equipment
        .iter()
        .filter(|(_, stack)| !stack.is_empty())
        .filter(|(slot, _)| {
            enchantment
                .slots
                .iter()
                .any(|allowed| slot_matches(allowed, slot))
        })
        .map(|(_, stack)| stack.get_enchantment_level(enchantment))
        .max()
        .unwrap_or(0)
        .max(0)
}
fn slot_matches(allowed: &pumpkin_data::enchantment::AttributeModifierSlot, slot: &str) -> bool {
    use pumpkin_data::enchantment::AttributeModifierSlot as Slot;
    match allowed {
        Slot::Any => true,
        Slot::Hand => matches!(slot, "mainhand" | "offhand"),
        Slot::Armor => matches!(slot, "head" | "chest" | "legs" | "feet"),
        Slot::MainHand => slot == "mainhand",
        Slot::OffHand => slot == "offhand",
        Slot::Feet => slot == "feet",
        Slot::Legs => slot == "legs",
        Slot::Chest => slot == "chest",
        Slot::Head => slot == "head",
        Slot::Body => slot == "body",
        Slot::Saddle => slot == "saddle",
    }
}
