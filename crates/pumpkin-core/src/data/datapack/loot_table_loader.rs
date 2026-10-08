use pumpkin_util::loot_table::DynamicLootTable;
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::Path,
    sync::Arc,
};

/// Parse a loot table with the shared 26.3/legacy decoder.
#[must_use]
pub fn parse_loot_table(json_content: &str) -> Option<DynamicLootTable> {
    let json = json_content;
    let mut unsupported = BTreeSet::new();
    let table = pumpkin_util::loot_table::parse_loot_table(json, &mut unsupported);
    for kind in unsupported {
        tracing::warn!("Unsupported loot feature: {kind}");
    }
    table
}
/// Load tables without inlining references, so the final registry retains pack precedence.
pub fn load_loot_tables_from_dir<S: std::hash::BuildHasher>(
    namespace: &str,
    dir: &Path,
    registry: &mut HashMap<String, Arc<DynamicLootTable>, S>,
) -> usize {
    let before = registry.len();
    load_recursive(namespace, dir, dir, registry);
    registry.len() - before
}
fn read_document(path: &Path) -> Option<String> {
    // Bound disk-supplied JSON before allocating or resolving recursive references.
    const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
    (fs::metadata(path).ok()?.len() <= MAX_DOCUMENT_BYTES)
        .then(|| fs::read_to_string(path).ok())
        .flatten()
}
fn load_recursive<S: std::hash::BuildHasher>(
    namespace: &str,
    base: &Path,
    dir: &Path,
    registry: &mut HashMap<String, Arc<DynamicLootTable>, S>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_recursive(namespace, base, &path, registry);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            && let Ok(relative) = path.strip_prefix(base)
        {
            let key = format!(
                "{namespace}:{}",
                relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            if let Some(json) = read_document(&path)
                && let Some(table) = parse_loot_table(&json)
            {
                registry.insert(key, Arc::new(table));
            }
        }
    }
}

/// Load predicate and modifier documents into the merged reloadable loot registry.
pub fn load_loot_registry_documents<S: std::hash::BuildHasher>(
    namespace: &str,
    namespace_dir: &Path,
    registry: &mut HashMap<(String, String), Value, S>,
) {
    load_documents_recursive(
        namespace,
        "loot_table_tag",
        &namespace_dir.join("tags/loot_table"),
        &namespace_dir.join("tags/loot_table"),
        registry,
    );
    for kind in ["predicate", "item_modifier"] {
        load_documents_recursive(
            namespace,
            kind,
            &namespace_dir.join(kind),
            &namespace_dir.join(kind),
            registry,
        );
    }
}
fn load_documents_recursive<S: std::hash::BuildHasher>(
    namespace: &str,
    kind: &str,
    base: &Path,
    dir: &Path,
    registry: &mut HashMap<(String, String), Value, S>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_documents_recursive(namespace, kind, base, &path, registry);
        } else if path.extension().is_some_and(|ext| ext == "json")
            && let Ok(relative) = path.strip_prefix(base)
            && let Some(json) = read_document(&path)
            && let Ok(mut document) = serde_json::from_str::<Value>(&json)
        {
            let name = format!(
                "{namespace}:{}",
                relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            let key = (kind.to_owned(), name);
            // TagLoader.load appends tag entries unless the higher pack declares replace.
            if kind == "loot_table_tag"
                && document.get("replace").and_then(Value::as_bool) != Some(true)
                && let Some(previous) = registry.get(&key)
                && let Some(previous) = previous.get("values").and_then(Value::as_array)
                && let Some(values) = document.get_mut("values").and_then(Value::as_array_mut)
            {
                let mut merged = previous.clone();
                merged.append(values);
                *values = merged;
            }
            registry.insert(key, document);
        }
    }
}
#[cfg(test)]
mod tests;
