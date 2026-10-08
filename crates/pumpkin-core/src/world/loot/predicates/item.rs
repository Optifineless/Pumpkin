use super::{item_component, range_matches, registry_matches};
use crate::data::datapack::context_provider_loader::json_value_to_nbt;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{DataComponentImpl, EnchantmentsImpl, read_data},
    item_stack::ItemStack,
};
use serde_json::Value;
pub(in crate::world::loot) fn read_loot_component(
    id: DataComponent,
    value: &Value,
) -> Option<Box<dyn DataComponentImpl>> {
    // SetComponentsFunction: only plain text survives the current text NBT writer.
    if id == DataComponent::CustomName {
        let text = value.as_str().or_else(|| {
            let object = value.as_object()?;
            (object.len() == 1)
                .then(|| object.get("text")?.as_str())
                .flatten()
        })?;
        return Some(Box::new(
            pumpkin_data::data_component_impl::CustomNameImpl {
                name: pumpkin_util::text::TextComponent::text(text.to_owned()),
            },
        ));
    }
    if !pumpkin_util::loot_table::component_predicate_supported(id.to_name(), value) {
        return None;
    }
    read_data(id, &json_value_to_nbt(value))
}
pub(in crate::world::loot) fn item_predicate(predicate: &Value, stack: &ItemStack) -> Option<bool> {
    // ItemPredicate.test and DataComponentMatchers.test, without substring matching.
    if predicate.is_null() {
        return Some(true);
    }
    for (key, value) in predicate.as_object()? {
        let matched = match key.as_str() {
            "items" => registry_matches(
                value,
                if stack.is_empty() {
                    &pumpkin_data::item::Item::AIR
                } else {
                    stack.item
                },
            ),
            "count" => range_matches(
                value,
                f64::from(if stack.is_empty() {
                    0
                } else {
                    stack.item_count
                }),
            ),
            "components" => {
                for (name, value) in value.as_object()? {
                    let id = DataComponent::try_from_name(name)?;
                    if !pumpkin_util::loot_table::component_predicate_supported(name, value) {
                        return None;
                    }
                    let expected = read_loot_component(id, value)?;
                    if !item_component(stack, id)
                        .is_some_and(|actual| actual.equal(expected.as_ref()))
                    {
                        return Some(false);
                    }
                }
                true
            }
            "predicates" => {
                for (kind, rules) in value.as_object()? {
                    match kind.as_str() {
                        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
                            let id = if kind == "minecraft:stored_enchantments" {
                                DataComponent::StoredEnchantments
                            } else {
                                DataComponent::Enchantments
                            };
                            if item_component(stack, id).is_none() {
                                return Some(false);
                            }
                            let enchantments = if kind == "minecraft:stored_enchantments" {
                                stack.get_data_component::<pumpkin_data::data_component_impl::StoredEnchantmentsImpl>().map(|v| v.enchantment.as_ref())
                            } else {
                                stack
                                    .get_data_component::<EnchantmentsImpl>()
                                    .map(|v| v.enchantment.as_ref())
                            };
                            for rule in rules.as_array()? {
                                if !enchantments.is_some_and(|enchantments| {
                                    enchantments.iter().any(|(enchantment, level)| {
                                        rule.get("enchantments").is_none_or(|names| {
                                            registry_matches(names, *enchantment)
                                        }) && rule.get("levels").is_none_or(|range| {
                                            range_matches(range, f64::from(*level))
                                        })
                                    })
                                }) {
                                    return Some(false);
                                }
                            }
                        }
                        _ => return None,
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
