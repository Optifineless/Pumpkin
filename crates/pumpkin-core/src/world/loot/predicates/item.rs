use super::{item_component, range_matches, registry_matches};
use crate::data::datapack::context_provider_loader::json_value_to_nbt;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{CustomDataImpl, DataComponentImpl, EnchantmentsImpl, read_data},
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
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
    item_predicate_with_custom_data(predicate, stack, None)
}

pub(in crate::world::loot) fn item_predicate_with_custom_data(
    predicate: &Value,
    stack: &ItemStack,
    raw_custom_data: Option<&NbtTag>,
) -> Option<bool> {
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
                        "minecraft:custom_data" => {
                            if !custom_data_matches(rules, stack, raw_custom_data)? {
                                return Some(false);
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

fn custom_data_matches(
    rules: &Value,
    stack: &ItemStack,
    raw_custom_data: Option<&NbtTag>,
) -> Option<bool> {
    let converted_expected = raw_custom_data.is_none().then(|| json_value_to_nbt(rules));
    let expected = converted_expected.as_ref().or(raw_custom_data)?;
    let NbtTag::Compound(expected) = expected else {
        return None;
    };
    Some(
        stack
            .get_data_component::<CustomDataImpl>()
            .is_some_and(|actual| partial_compound_matches(expected, &actual.data)),
    )
}

/*
Vanilla 26.3 `net.minecraft.nbt.NbtUtils.compareNbt(expected, actual, true)`:
if (expected == actual) {
   return true;
} else if (expected == null) {
   return true;
} else if (actual == null) {
   return false;
} else if (!expected.getClass().equals(actual.getClass())) {
   return false;
} else if (expected instanceof CompoundTag expectedCompound) {
   CompoundTag actualCompound = (CompoundTag)actual;
   if (actualCompound.size() < expectedCompound.size()) {
      return false;
   } else {
      for (Entry<String, Tag> entry : expectedCompound.entrySet()) {
         Tag tag = entry.getValue();
         if (!compareNbt(tag, actualCompound.get(entry.getKey()), partialListMatches)) {
            return false;
         }
      }

      return true;
   }
} else if (expected instanceof ListTag expectedList && partialListMatches) {
   ListTag actualList = (ListTag)actual;
   if (expectedList.isEmpty()) {
      return actualList.isEmpty();
   } else if (actualList.size() < expectedList.size()) {
      return false;
   } else {
      for (Tag tag : expectedList) {
         boolean found = false;

         for (Tag value : actualList) {
            if (compareNbt(tag, value, partialListMatches)) {
               found = true;
               break;
            }
         }

         if (!found) {
            return false;
         }
      }

      return true;
   }
} else {
   return expected.equals(actual);
}
*/
fn partial_compound_matches(expected: &NbtCompound, actual: &NbtCompound) -> bool {
    actual.child_tags.len() >= expected.child_tags.len()
        && expected
            .child_tags
            .iter()
            .all(|(key, expected)| partial_nbt_matches(Some(expected), actual.child_tags.get(key)))
}

fn partial_nbt_matches(expected: Option<&NbtTag>, actual: Option<&NbtTag>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };
    match (expected, actual) {
        (NbtTag::Compound(expected), NbtTag::Compound(actual)) => {
            partial_compound_matches(expected, actual)
        }
        (NbtTag::List(expected), NbtTag::List(actual)) => {
            if expected.is_empty() {
                actual.is_empty()
            } else {
                actual.len() >= expected.len()
                    && expected.iter().all(|expected| {
                        actual
                            .iter()
                            .any(|actual| partial_nbt_matches(Some(expected), Some(actual)))
                    })
            }
        }
        _ => expected == actual,
    }
}
