use super::{EntityLootState, LootContextParameters};
use pumpkin_data::{
    data_component::DataComponent, data_component_impl::DataComponentImpl, item_stack::ItemStack,
    tag::Taggable,
};
use serde_json::Value;
mod block;
mod entity;
mod item;
pub(super) use block::match_block;
pub(super) use entity::{attacker_enchantment_level, enchantment_level, entity_predicate};
pub(super) use item::{item_predicate, read_loot_component};

pub(in crate::world::loot) fn entity_target<'a>(
    target: &str,
    params: &'a LootContextParameters,
) -> Option<&'a EntityLootState> {
    match target.trim_start_matches("minecraft:") {
        "this" | "this_entity" => params.this_entity_state.as_ref(),
        "attacker" | "killer" | "attacking_entity" => params.attacking_entity_state.as_ref(),
        "direct_attacker" | "direct_killer" | "direct_attacking_entity" => {
            params.direct_attacking_entity_state.as_ref()
        }
        "attacking_player" | "last_damage_player" | "killer_player" => {
            params.last_damage_player_state.as_ref()
        }
        _ => None,
    }
}
pub(in crate::world::loot) fn registry_matches<T: Taggable>(names: &Value, object: &T) -> bool {
    match names {
        Value::String(name) => {
            if name.starts_with('#') {
                object.is_tagged_with(name).unwrap_or(false)
            } else {
                object.registry_key().trim_start_matches("minecraft:")
                    == name.trim_start_matches("minecraft:")
            }
        }
        Value::Array(names) => names.iter().any(|name| registry_matches(name, object)),
        _ => false,
    }
}
pub(in crate::world::loot) fn range_matches(range: &Value, actual: f64) -> bool {
    if let Some(exact) = range
        .as_f64()
        .or_else(|| range.as_str().and_then(|v| v.parse().ok()))
    {
        return actual == exact;
    }
    range.as_object().is_some_and(|range| {
        let number = |key| {
            range.get(key).and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|v| v.parse().ok()))
            })
        };
        number("min").is_none_or(|min| actual >= min)
            && number("max").is_none_or(|max| actual <= max)
    })
}
pub(in crate::world::loot) fn item_component(
    stack: &ItemStack,
    id: DataComponent,
) -> Option<&dyn DataComponentImpl> {
    // ItemStack.getComponents returns an empty map for empty stacks.
    if stack.is_empty() {
        return None;
    }
    if let Some((_, component)) = stack.patch.iter().find(|(key, _)| *key == id) {
        return component.as_deref();
    }
    stack
        .item
        .components
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, component)| *component)
}
