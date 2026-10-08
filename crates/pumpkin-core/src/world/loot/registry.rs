use super::{
    LootContextParameters, LootTableHandle, MAX_LOOT_DEPTH, MAX_LOOT_STACKS, get_loot_table,
};
use serde_json::Value;
use std::collections::BTreeSet;
pub(super) fn registry_key(name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    }
}
pub(super) fn registry_document(
    params: &LootContextParameters,
    kind: &str,
    name: &str,
) -> Option<Value> {
    params
        .registry
        .as_ref()
        .and_then(|registry| registry.get_loot_registry_document(kind, name))
        .or_else(|| {
            pumpkin_data::loot_table::get_loot_registry_entry(kind, name)
                .and_then(|json| serde_json::from_str(json).ok())
        })
}
pub(super) fn resolve_table(params: &LootContextParameters, name: &str) -> Option<LootTableHandle> {
    // NestedLootTable.createItemStack resolves the current registry holder.
    params
        .registry
        .as_ref()
        .and_then(|registry| registry.get_loot_table(name))
        .or_else(|| get_loot_table(name))
}
// Generated documents are immutable. Runtime caches live in the registry reload generation.
fn generated_registry() -> &'static crate::data::datapack::loot_registry::LootRegistry {
    static REGISTRY: std::sync::OnceLock<crate::data::datapack::loot_registry::LootRegistry> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
pub(super) fn predicate(
    params: &LootContextParameters,
    name: &str,
) -> Option<std::sync::Arc<super::LootCondition>> {
    let name = registry_key(name);
    params.registry.as_ref().map_or_else(
        || generated_registry().predicate(&name),
        |registry| registry.get_loot_predicate(&name),
    )
}
pub(super) fn modifier(
    params: &LootContextParameters,
    name: &str,
) -> Option<std::sync::Arc<[super::LootFunction]>> {
    let name = registry_key(name);
    params.registry.as_ref().map_or_else(
        || generated_registry().modifier(&name),
        |registry| registry.get_loot_modifier(&name),
    )
}
pub(super) fn item_holders(value: &Value, rng: &mut super::LootRandom<'_>) -> Option<Vec<String>> {
    use pumpkin_data::{item::Item, tag::Taggable};
    if !rng.charge_work(1) {
        return None;
    }
    match value {
        Value::String(name) if name.starts_with('#') => {
            let items = Item::get_tag_values(name)?;
            if items.len() > MAX_LOOT_STACKS || !rng.charge_work(items.len()) {
                return None;
            }
            Some(items.iter().map(|item| (*item).to_owned()).collect())
        }
        Value::String(name) => {
            Item::from_registry_key(name.strip_prefix("minecraft:").unwrap_or(name))
                .map(|_| vec![registry_key(name)])
        }
        Value::Array(names) => {
            let mut result = Vec::new();
            // HolderSet's direct form is a list of identifiers, never nested lists/tags.
            for name in names {
                let name = name.as_str()?;
                if name.starts_with('#') {
                    return None;
                }
                result.extend(item_holders(&Value::String(name.to_owned()), rng)?);
                if result.len() > MAX_LOOT_STACKS {
                    return None;
                }
            }
            Some(result)
        }
        _ => None,
    }
}
pub(super) fn table_holders(
    name: &str,
    params: &LootContextParameters,
    rng: &mut super::LootRandom<'_>,
) -> Option<Vec<String>> {
    // Memoization is local to this resolution; no result survives a registry reload.
    TableHolders {
        params,
        visiting: BTreeSet::new(),
        cache: std::collections::HashMap::default(),
    }
    .resolve(name, rng, 0)
}
struct TableHolders<'a> {
    params: &'a LootContextParameters,
    visiting: BTreeSet<String>,
    cache: std::collections::HashMap<String, Option<Vec<String>>>,
}
impl TableHolders<'_> {
    fn resolve(
        &mut self,
        name: &str,
        rng: &mut super::LootRandom<'_>,
        depth: usize,
    ) -> Option<Vec<String>> {
        if depth >= MAX_LOOT_DEPTH || !rng.charge_work(1) {
            return None;
        }
        let Some(tag) = name.strip_prefix('#') else {
            return resolve_table(self.params, name).map(|_| vec![registry_key(name)]);
        };
        let tag = registry_key(tag);
        if let Some(cached) = self.cache.get(&tag) {
            if !rng.charge_work(cached.as_ref().map_or(0, Vec::len)) {
                return None;
            }
            return cached.clone();
        }
        if !self.visiting.insert(tag.clone()) {
            return None;
        }
        let result = self.resolve_tag(&tag, rng, depth);
        self.visiting.remove(&tag);
        self.cache.insert(tag, result.clone());
        result
    }
    fn resolve_tag(
        &mut self,
        tag: &str,
        rng: &mut super::LootRandom<'_>,
        depth: usize,
    ) -> Option<Vec<String>> {
        let document = registry_document(self.params, "loot_table_tag", tag)?;
        let mut names = Vec::new();
        let mut seen = BTreeSet::new();
        // TagLoader.tryBuildTag preserves order and distinguishes unresolved from empty.
        for entry in document.get("values")?.as_array()? {
            let name = entry
                .as_str()
                .or_else(|| entry.get("id").and_then(Value::as_str))?;
            let Some(values) = self.resolve(name, rng, depth + 1) else {
                if !rng.charge_work(1) {
                    return None;
                }
                if entry.get("required").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                return None;
            };
            for value in values {
                if !rng.charge_work(1) {
                    return None;
                }
                if seen.insert(value.clone()) {
                    names.push(value);
                }
                if names.len() > MAX_LOOT_STACKS {
                    return None;
                }
            }
        }
        Some(names)
    }
}
