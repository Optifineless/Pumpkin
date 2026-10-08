use super::{
    DynamicLootTable, LootCondition, LootEntry, LootEntryKind, LootFunction, LootPool,
    LootTableReference, SUPPORTED_ENTITY_COMPONENTS, component_predicate_supported,
};
use serde_json::Value;
use std::collections::BTreeSet;
/// Decode 26.3 and legacy loot codecs with bounded recursion.
///
/// Registry holders remain references until evaluation against the final datapack registry.
#[must_use]
pub fn parse_loot_table(
    json: &str,
    unsupported: &mut BTreeSet<String>,
) -> Option<DynamicLootTable> {
    // LootTable.DIRECT_CODEC, LootPool.CODEC and LootPoolEntries.CODEC keep this tree.
    const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
    if json.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    let root: Value = serde_json::from_str(json).ok()?;
    root.as_object()?;
    let mut parser = Parser {
        unsupported,
        remaining_nodes: 32768,
    };
    let table = parser.table(&root, 0);
    (parser.remaining_nodes > 0).then_some(table)
}
struct Parser<'a> {
    unsupported: &'a mut BTreeSet<String>,
    remaining_nodes: usize,
}
const MAX_DEPTH: usize = 64;
const MAX_CHILDREN: usize = 4096;
impl Parser<'_> {
    const fn take_node(&mut self) -> bool {
        self.remaining_nodes = self.remaining_nodes.saturating_sub(1);
        self.remaining_nodes > 0
    }
    fn unsupported(&mut self, kind: &str) -> LootCondition {
        self.unsupported.insert(kind.to_owned());
        LootCondition::Unsupported(kind.to_owned())
    }
    fn table(&mut self, value: &Value, depth: usize) -> DynamicLootTable {
        if depth >= MAX_DEPTH || !self.take_node() {
            return DynamicLootTable::default();
        }
        DynamicLootTable {
            random_sequence: value
                .get("random_sequence")
                .and_then(Value::as_str)
                .map(str::to_owned),
            pools: value
                .get("pools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_CHILDREN)
                .map(|pool| LootPool {
                    entries: pool
                        .get("entries")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_CHILDREN)
                        .map(|entry| self.entry(entry, depth + 1))
                        .collect(),
                    rolls: pool.get("rolls").cloned().unwrap_or(Value::from(1)),
                    bonus_rolls: pool.get("bonus_rolls").cloned().unwrap_or(Value::from(0)),
                    condition: self.condition_field(pool, depth + 1),
                    functions: self.functions_field(pool, depth + 1),
                })
                .collect(),
            functions: self.functions_field(value, depth + 1),
        }
    }
    fn condition_field(&mut self, value: &Value, depth: usize) -> LootCondition {
        value
            .get("condition")
            .or_else(|| value.get("conditions"))
            .map_or(LootCondition::None, |c| self.condition(c, depth))
    }
    fn condition(&mut self, value: &Value, depth: usize) -> LootCondition {
        if depth >= MAX_DEPTH || !self.take_node() {
            return self.unsupported("predicate recursion limit");
        }
        if let Some(name) = value.as_str() {
            return LootCondition::Reference(name.to_owned());
        }
        if let Some(conditions) = value.as_array() {
            if conditions.len() > MAX_CHILDREN {
                return self.unsupported("predicate length limit");
            }
            return LootCondition::AllOf(
                conditions
                    .iter()
                    .take(MAX_CHILDREN)
                    .map(|c| self.condition(c, depth + 1))
                    .collect(),
            );
        }
        let kind = value
            .get("type")
            .or_else(|| value.get("condition"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
        match kind {
            "survives_explosion" => LootCondition::SurvivesExplosion,
            "killed_by_player" => LootCondition::KilledByPlayer,
            "random_chance" | "random_chance_with_enchanted_bonus" => {
                self.random_condition(value, kind)
            }
            "table_bonus" => LootCondition::TableBonus {
                enchantment: string(value, "enchantment", "minecraft:fortune"),
                chances: value
                    .get("chances")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(MAX_CHILDREN)
                    .filter_map(Value::as_f64)
                    .map(|v| v as f32)
                    .collect(),
            },
            "match_block" | "block_state_property" => self.block_condition(value),
            "match_tool" => {
                let predicate = value.get("predicate").cloned().unwrap_or(Value::Null);
                if !self.report_item_parts(&predicate) {
                    return self.unsupported("item predicate parts");
                }
                LootCondition::MatchTool(predicate)
            }
            "entity_properties" => self.entity_condition(value),
            "inverted" => match value.get("term") {
                Some(term) => LootCondition::Inverted(Box::new(self.condition(term, depth + 1))),
                None => self.unsupported(kind),
            },
            "any_of" | "all_of" => match value.get("terms").and_then(Value::as_array) {
                Some(terms) => {
                    if terms.len() > MAX_CHILDREN {
                        return self.unsupported("predicate length limit");
                    }
                    let terms = terms
                        .iter()
                        .take(MAX_CHILDREN)
                        .map(|c| self.condition(c, depth + 1))
                        .collect();
                    if kind == "any_of" {
                        LootCondition::AnyOf(terms)
                    } else {
                        LootCondition::AllOf(terms)
                    }
                }
                None => self.unsupported(kind),
            },
            "weather_check" => LootCondition::WeatherCheck {
                raining: value.get("raining").and_then(Value::as_bool),
                thundering: value.get("thundering").and_then(Value::as_bool),
            },
            _ => self.unsupported(kind),
        }
    }
    fn entity_condition(&mut self, value: &Value) -> LootCondition {
        let entity = string(value, "entity", "this");
        if !matches!(
            entity.trim_start_matches("minecraft:"),
            "this"
                | "this_entity"
                | "attacker"
                | "killer"
                | "attacking_entity"
                | "direct_attacker"
                | "direct_killer"
                | "direct_attacking_entity"
                | "attacking_player"
                | "last_damage_player"
                | "killer_player"
        ) {
            return self.unsupported(&format!("entity target {entity}"));
        }
        let predicate = value.get("predicate").cloned().unwrap_or(Value::Null);
        if !self.report_entity_parts(&predicate) {
            return self.unsupported("entity predicate parts");
        }
        LootCondition::EntityProperties { entity, predicate }
    }
    fn block_condition(&mut self, value: &Value) -> LootCondition {
        if value.as_object().is_some_and(|fields| {
            fields.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "condition" | "blocks" | "block" | "state" | "properties"
                )
            })
        }) {
            return self.unsupported("match_block predicate parts");
        }
        LootCondition::BlockStateProperty {
            blocks: value
                .get("blocks")
                .or_else(|| value.get("block"))
                .cloned()
                .unwrap_or(Value::Null),
            properties: value
                .get("state")
                .or_else(|| value.get("properties"))
                .cloned()
                .unwrap_or(Value::Null),
        }
    }
    fn random_condition(&mut self, value: &Value, kind: &str) -> LootCondition {
        // RandomChance codecs require supported providers even if the current level is zero.
        if kind == "random_chance" {
            let chance = value.get("chance").cloned().unwrap_or(Value::Null);
            if !number_provider_supported(&chance, 0) {
                return self.unsupported("random chance number provider");
            }
            return LootCondition::RandomChance(chance);
        }
        let enchanted = value
            .get("enchanted_chance")
            .cloned()
            .unwrap_or(Value::Null);
        let supported = enchanted.is_number()
            || (enchanted
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.trim_start_matches("minecraft:") == "linear")
                && enchanted.get("base").is_some_and(Value::is_number)
                && enchanted
                    .get("per_level_above_first")
                    .is_some_and(Value::is_number));
        if !supported {
            return self.unsupported("enchanted chance level provider");
        }
        LootCondition::RandomChanceWithEnchantedBonus {
            enchantment: string(value, "enchantment", "minecraft:looting"),
            unenchanted: value
                .get("unenchanted_chance")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
            enchanted,
        }
    }
    fn report_entity_parts(&mut self, value: &Value) -> bool {
        let mut supported = true;
        if let Some(parts) = value.as_object() {
            for (key, part) in parts {
                let key = key.strip_prefix("minecraft:").unwrap_or(key);
                match key {
                    "vehicle" | "passenger" => supported &= self.report_entity_parts(part),
                    "equipment" => {
                        if let Some(slots) = part.as_object() {
                            for predicate in slots.values() {
                                supported &= self.report_item_parts(predicate);
                            }
                        }
                    }
                    "components" => {
                        if let Some(components) = part.as_object() {
                            for name in components.keys() {
                                if !SUPPORTED_ENTITY_COMPONENTS.contains(&name.as_str()) {
                                    self.unsupported.insert(format!("entity component {name}"));
                                    supported = false;
                                }
                            }
                        }
                    }
                    "type_specific/fishing_hook" => {
                        // FishingHookPredicate.CODEC reads one optional boolean, ignoring other keys.
                        if part.as_object().is_none_or(|fields| {
                            fields
                                .get("in_open_water")
                                .is_some_and(|value| !value.is_boolean())
                        }) {
                            self.unsupported
                                .insert("fishing hook predicate parts".to_owned());
                            supported = false;
                        }
                    }
                    "type_specific/sheep" => {
                        if part
                            .as_object()
                            .is_some_and(|fields| fields.keys().any(|key| key != "sheared"))
                        {
                            self.unsupported.insert("sheep predicate parts".to_owned());
                            supported = false;
                        }
                    }
                    "flags" => {
                        if let Some(flags) = part.as_object() {
                            for name in flags.keys() {
                                if !matches!(
                                    name.as_str(),
                                    "is_on_ground"
                                        | "is_on_fire"
                                        | "is_sneaking"
                                        | "is_sprinting"
                                        | "is_swimming"
                                        | "is_flying"
                                        | "is_baby"
                                        | "is_in_water"
                                        | "is_fall_flying"
                                ) {
                                    self.unsupported.insert(format!("entity flag {name}"));
                                    supported = false;
                                }
                            }
                        }
                    }
                    "entity_type" | "type" => {}
                    _ => {
                        self.unsupported.insert(format!("entity predicate {key}"));
                        supported = false;
                    }
                }
            }
        }
        supported
    }
    fn report_item_parts(&mut self, value: &Value) -> bool {
        let mut supported = true;
        if let Some(parts) = value.as_object() {
            for (key, part) in parts {
                match key.as_str() {
                    "items" | "count" => {}
                    "components" => {
                        if let Some(components) = part.as_object() {
                            for (name, value) in components {
                                if !component_predicate_supported(name, value) {
                                    self.unsupported.insert(format!("item component {name}"));
                                    supported = false;
                                }
                            }
                        }
                    }
                    "predicates" => {
                        if let Some(predicates) = part.as_object() {
                            for key in predicates.keys() {
                                if !matches!(
                                    key.as_str(),
                                    "minecraft:enchantments" | "minecraft:stored_enchantments"
                                ) {
                                    self.unsupported.insert(format!("item predicate {key}"));
                                    supported = false;
                                }
                            }
                        }
                    }
                    _ => {
                        self.unsupported.insert(format!("item predicate {key}"));
                        supported = false;
                    }
                }
            }
        }
        supported
    }
    fn functions_field(&mut self, value: &Value, depth: usize) -> Vec<LootFunction> {
        value
            .get("modifier")
            .or_else(|| value.get("functions"))
            .map_or_else(Vec::new, |f| self.functions(f, depth))
    }
    fn functions(&mut self, value: &Value, depth: usize) -> Vec<LootFunction> {
        if depth >= MAX_DEPTH || !self.take_node() {
            return Vec::new();
        }
        if let Some(name) = value.as_str() {
            return vec![LootFunction {
                kind: "reference".to_owned(),
                condition: LootCondition::None,
                parameters: Value::String(name.to_owned()),
                functions: Vec::new(),
            }];
        }
        if let Some(functions) = value.as_array() {
            return functions
                .iter()
                .take(MAX_CHILDREN)
                .flat_map(|f| self.functions(f, depth + 1))
                .collect();
        }
        let kind = value
            .get("type")
            .or_else(|| value.get("function"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
        if !matches!(
            kind,
            "sequence"
                | "set_count"
                | "apply_bonus"
                | "looting_enchant"
                | "enchanted_count_increase"
                | "explosion_decay"
                | "limit_count"
                | "set_components"
                | "copy_components"
                | "copy_state"
                | "furnace_smelt"
        ) {
            self.unsupported.insert(format!("modifier {kind} (no-op)"));
        }
        // SequenceFunction.MAP_CODEC keeps one condition around its ordered modifiers.
        let functions = if kind == "sequence" {
            value
                .get("functions")
                .map_or_else(Vec::new, |f| self.functions(f, depth + 1))
        } else {
            Vec::new()
        };
        vec![LootFunction {
            kind: kind.to_owned(),
            condition: self.condition_field(value, depth + 1),
            parameters: value.clone(),
            functions,
        }]
    }
    fn entry(&mut self, value: &Value, depth: usize) -> LootEntry {
        let kind = string(value, "type", "");
        let kind = kind.strip_prefix("minecraft:").unwrap_or(&kind);
        let children = |parser: &mut Self| {
            value
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_CHILDREN)
                .map(|v| parser.entry(v, depth + 1))
                .collect()
        };
        let entry_kind = if depth >= MAX_DEPTH || !self.take_node() {
            LootEntryKind::Unsupported
        } else {
            match kind {
                "item" => LootEntryKind::Item(string(value, "name", "")),
                "empty" => LootEntryKind::Empty,
                "tag" => LootEntryKind::Tag {
                    items: value.get("items").cloned().unwrap_or_else(|| {
                        value
                            .get("name")
                            .and_then(Value::as_str)
                            .map_or(Value::Null, |name| {
                                Value::String(if name.starts_with('#') {
                                    name.to_owned()
                                } else {
                                    format!("#{name}")
                                })
                            })
                    }),
                    expand: value
                        .get("expand")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
                "alternatives" => LootEntryKind::Alternatives(children(self)),
                "group" => LootEntryKind::Group(children(self)),
                "sequence" => LootEntryKind::Sequence(children(self)),
                "loot_table" => {
                    let table = value
                        .get("value")
                        .or_else(|| value.get("name"))
                        .unwrap_or(&Value::Null);
                    let tables: Vec<_> = table
                        .as_array()
                        .map_or_else(|| vec![table], |tables| tables.iter().collect());
                    if tables
                        .iter()
                        .any(|table| !table.is_string() && !table.is_object())
                    {
                        LootEntryKind::Unsupported
                    } else {
                        LootEntryKind::Tables {
                            tables: tables
                                .into_iter()
                                .take(MAX_CHILDREN)
                                .map(|table| {
                                    table.as_str().map_or_else(
                                        || {
                                            LootTableReference::Inline(Box::new(
                                                self.table(table, depth + 1),
                                            ))
                                        },
                                        |name| LootTableReference::Named(name.to_owned()),
                                    )
                                })
                                .collect(),
                            expand: value
                                .get("expand")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        }
                    }
                }
                _ => LootEntryKind::Unsupported,
            }
        };
        LootEntry {
            kind: entry_kind,
            weight: value
                .get("weight")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            quality: value
                .get("quality")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            condition: self.condition_field(value, depth + 1),
            functions: self.functions_field(value, depth + 1),
        }
    }
}
#[must_use]
pub fn number_provider_supported(value: &Value, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return false;
    }
    if value.is_number() {
        return true;
    }
    let supported = |name| {
        value
            .get(name)
            .is_some_and(|value| number_provider_supported(value, depth + 1))
    };
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("minecraft:uniform")
        .trim_start_matches("minecraft:")
    {
        "constant" => supported("value"),
        "uniform" => supported("min") && supported("max"),
        "binomial" => supported("n") && supported("p"),
        _ => false,
    }
}
fn string(value: &Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}

/// Decode a predicate holder after the runtime registry has selected its document.
#[must_use]
pub fn parse_loot_condition(value: &Value) -> LootCondition {
    Parser {
        unsupported: &mut BTreeSet::new(),
        remaining_nodes: 32768,
    }
    .condition(value, 0)
}
/// Decode an item modifier holder after the runtime registry has selected its document.
#[must_use]
pub fn parse_loot_functions(value: &Value) -> Vec<LootFunction> {
    Parser {
        unsupported: &mut BTreeSet::new(),
        remaining_nodes: 32768,
    }
    .functions(value, 0)
}
