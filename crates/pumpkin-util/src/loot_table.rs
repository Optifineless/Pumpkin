use serde_json::Value;
use std::{collections::BTreeSet, sync::OnceLock};

/// Entity components supplied by the live loot snapshot.
pub const SUPPORTED_ENTITY_COMPONENTS: &[&str] = &[
    "minecraft:sheep/color",
    "minecraft:chicken/variant",
    "minecraft:mooshroom/variant",
];

/// A generated table retains vanilla JSON and decodes it once on first use.
#[derive(Debug)]
pub struct LootTable {
    json: &'static str,
    parsed: OnceLock<DynamicLootTable>,
}
impl LootTable {
    #[must_use]
    pub const fn new(json: &'static str) -> Self {
        Self {
            json,
            parsed: OnceLock::new(),
        }
    }
    #[must_use]
    pub const fn json(&self) -> &'static str {
        self.json
    }
    #[must_use]
    pub fn parsed(&self) -> &DynamicLootTable {
        // Codegen validates these documents with this same decoder.
        self.parsed
            .get_or_init(|| parse_loot_table(self.json, &mut BTreeSet::new()).unwrap_or_default())
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum LootCondition {
    #[default]
    None,
    Unsupported(String),
    Reference(String),
    SurvivesExplosion,
    KilledByPlayer,
    RandomChance(Value),
    RandomChanceWithEnchantedBonus {
        enchantment: String,
        unenchanted: f32,
        enchanted: Value,
    },
    TableBonus {
        enchantment: String,
        chances: Vec<f32>,
    },
    BlockStateProperty {
        blocks: Value,
        properties: Value,
    },
    MatchTool(Value),
    EntityProperties {
        entity: String,
        predicate: Value,
    },
    AllOf(Vec<Self>),
    AnyOf(Vec<Self>),
    Inverted(Box<Self>),
    WeatherCheck {
        raining: Option<bool>,
        thundering: Option<bool>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LootBonusFormula {
    OreDrops,
    UniformBonusCount(i32),
    BinomialWithBonusCount { extra: i32, probability: f32 },
}
/// Conditional modifiers remain ordered at entry, pool and table scope.
#[derive(Clone, Debug)]
pub struct LootFunction {
    pub condition: LootCondition,
    pub kind: String,
    pub parameters: Value,
    pub functions: Vec<Self>,
}
#[derive(Clone, Debug)]
pub enum LootEntryKind {
    Item(String),
    Empty,
    Tag {
        items: Value,
        expand: bool,
    },
    Tables {
        tables: Vec<LootTableReference>,
        expand: bool,
    },
    Alternatives(Vec<LootEntry>),
    Group(Vec<LootEntry>),
    Sequence(Vec<LootEntry>),
    Unsupported,
}
#[derive(Clone, Debug)]
pub enum LootTableReference {
    Named(String),
    Inline(Box<DynamicLootTable>),
}
#[derive(Clone, Debug)]
pub struct LootEntry {
    pub kind: LootEntryKind,
    pub weight: i32,
    pub quality: i32,
    pub condition: LootCondition,
    pub functions: Vec<LootFunction>,
}
#[derive(Clone, Debug)]
pub struct LootPool {
    pub entries: Vec<LootEntry>,
    pub rolls: Value,
    pub bonus_rolls: Value,
    pub condition: LootCondition,
    pub functions: Vec<LootFunction>,
}
#[derive(Clone, Debug, Default)]
pub struct DynamicLootTable {
    pub random_sequence: Option<String>,
    pub pools: Vec<LootPool>,
    pub functions: Vec<LootFunction>,
}
pub type DynamicLootCondition = LootCondition;
pub type DynamicLootEntry = LootEntry;
pub type DynamicLootPool = LootPool;
pub type ChestLootEntry = LootEntry;
pub type ChestLootPool = LootPool;
pub type ChestLootTable = LootTable;
impl From<&LootTable> for DynamicLootTable {
    fn from(table: &LootTable) -> Self {
        table.parsed().clone()
    }
}

mod parse;
pub use parse::{
    number_provider_supported, parse_loot_condition, parse_loot_functions, parse_loot_table,
};

/// JSON codecs audited through conversion, decoding and value equality.
#[must_use]
pub fn component_codec_supported(name: &str) -> bool {
    // DataComponentExactPredicate.test compares complete values. Compound/float/NBT,
    // text, registry-holder and collection codecs are excluded until lossless; in
    // particular enchantment vectors have order-sensitive equality and containers
    // always compare unequal. Typed CopyComponentsFunction does not use this list.
    matches!(
        name.strip_prefix("minecraft:").unwrap_or(name),
        "max_stack_size"
            | "max_damage"
            | "damage"
            | "repair_cost"
            | "map_id"
            | "base_color"
            | "rarity"
    )
}

/// Typed copying bypasses JSON decoding and equality, but not placeholder storage.
#[must_use]
pub fn component_copy_supported(name: &str) -> bool {
    matches!(
        name.strip_prefix("minecraft:").unwrap_or(name),
        "max_stack_size"
            | "custom_data"
            | "enchantments"
            | "damage"
            | "max_damage"
            | "food"
            | "tool"
            | "enchantable"
            | "damage_resistant"
            | "potion_contents"
            | "fireworks"
            | "firework_explosion"
            | "custom_name"
            | "lore"
            | "item_name"
            | "item_model"
            | "consumable"
            | "equippable"
            | "attack_range"
            | "kinetic_weapon"
            | "piercing_weapon"
            | "damage_type"
            | "use_cooldown"
            | "repair_cost"
            | "repairable"
            | "map_id"
            | "map_post_processing"
            | "block_entity_data"
            | "bundle_contents"
            // CopyComponentsFunction.run copies BeehiveBlockEntity's typed occupants unchanged.
            | "bees"
            | "container"
            | "block_state"
            | "profile"
            | "chicken/variant"
            | "villager/variant"
            | "wolf/variant"
            | "wolf/sound_variant"
            | "wolf/collar"
            | "fox/variant"
            | "salmon/size"
            | "parrot/variant"
            | "mooshroom/variant"
            | "rabbit/variant"
            | "pig/variant"
            | "pig/sound_variant"
            | "cow/variant"
            | "cow/sound_variant"
            | "frog/variant"
            | "horse/variant"
            | "painting/variant"
            | "llama/variant"
            | "axolotl/variant"
            | "cat/variant"
            | "cat/sound_variant"
            | "cat/collar"
            | "sheep/color"
            | "shulker/color"
            | "dyed_color"
            | "base_color"
            | "note_block_sound"
            | "tooltip_style"
            | "lock"
            | "container_loot"
            | "custom_model_data"
            | "lodestone_tracker"
            | "trim"
            | "can_place_on"
            | "can_break"
            | "attack_animation"
            | "rarity"
            | "banner_patterns"
            | "weapon"
            | "entity_data"
            | "jukebox_playable"
            | "cooking_fuel"
            | "compostable"
    )
}

/// Reject unsupported exact values before a surrounding condition can invert them.
#[must_use]
pub fn component_predicate_supported(name: &str, value: &Value) -> bool {
    if !component_codec_supported(name) {
        return false;
    }
    // DataComponents' integer codecs; the current JSON-to-NBT bridge must not narrow values.
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "base_color" | "rarity" => value.is_string(),
        "max_stack_size" => value.as_i64().is_some_and(|n| (1..=99).contains(&n)), // DataComponents:118
        "max_damage" => value
            .as_i64()
            .is_some_and(|n| n > 0 && i32::try_from(n).is_ok()),
        "damage" | "repair_cost" => value
            .as_i64()
            .is_some_and(|n| n >= 0 && i32::try_from(n).is_ok()),
        "map_id" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fishing_hook_predicate_accepts_optional_boolean_and_unknown_keys() {
        for (parts, supported) in [
            (serde_json::json!({}), true),
            (serde_json::json!({"in_open_water":true}), true),
            (serde_json::json!({"in_open_water":false}), true),
            (serde_json::json!({"in_open_water":"true"}), false),
            (serde_json::json!({"unknown":true}), true),
            (serde_json::json!({"in_open_water":true,"unknown":{}}), true),
            (serde_json::json!(false), false),
        ] {
            let mut unsupported = BTreeSet::new();
            let condition = serde_json::json!({"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:type_specific/fishing_hook":parts}});
            let table = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:cod","condition":condition}]}]});
            parse_loot_table(&table.to_string(), &mut unsupported).unwrap();
            assert_eq!(unsupported.is_empty(), supported);
        }
    }
    #[test]
    fn unknown_conditions_and_missing_predicates_are_never_unconditional() {
        let mut unsupported = BTreeSet::new();
        for condition in [
            serde_json::json!({"type":"minecraft:entity_scores"}),
            serde_json::json!("custom:missing"),
        ] {
            let json = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","condition":condition}]}]});
            let table = parse_loot_table(&json.to_string(), &mut unsupported).unwrap();
            assert!(matches!(
                table.pools[0].entries[0].condition,
                LootCondition::Unsupported(_) | LootCondition::Reference(_)
            ));
        }
        assert_eq!(unsupported.len(), 1);
    }
}
