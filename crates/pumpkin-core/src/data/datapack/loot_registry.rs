use pumpkin_util::loot_table::{DynamicLootTable, LootCondition, LootFunction};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// One published reload generation; parsed holders cannot outlive their documents.
#[derive(Default)]
pub struct LootRegistry {
    pub tables: HashMap<String, Arc<DynamicLootTable>>,
    pub documents: HashMap<(String, String), Value>,
    predicates: Mutex<HashMap<String, Arc<LootCondition>>>,
    modifiers: Mutex<HashMap<String, Arc<[LootFunction]>>>,
}
impl LootRegistry {
    pub fn new(
        tables: HashMap<String, Arc<DynamicLootTable>>,
        documents: HashMap<(String, String), Value>,
    ) -> Self {
        Self {
            tables,
            documents,
            ..Self::default()
        }
    }
    pub fn document(&self, kind: &str, name: &str) -> Option<Value> {
        self.documents
            .get(&(kind.to_owned(), name.to_owned()))
            .cloned()
            .or_else(|| {
                pumpkin_data::loot_table::get_loot_registry_entry(kind, name)
                    .and_then(|json| serde_json::from_str(json).ok())
            })
    }
    pub fn predicate(&self, name: &str) -> Option<Arc<LootCondition>> {
        let mut cache = self
            .predicates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = cache.get(name) {
            return Some(value.clone());
        }
        let value = Arc::new(pumpkin_util::loot_table::parse_loot_condition(
            &self.document("predicate", name)?,
        ));
        cache.insert(name.to_owned(), value.clone());
        Some(value)
    }
    pub fn modifier(&self, name: &str) -> Option<Arc<[LootFunction]>> {
        let mut cache = self
            .modifiers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = cache.get(name) {
            return Some(value.clone());
        }
        let value: Arc<[LootFunction]> = Arc::from(pumpkin_util::loot_table::parse_loot_functions(
            &self.document("item_modifier", name)?,
        ));
        cache.insert(name.to_owned(), value.clone());
        Some(value)
    }
}
