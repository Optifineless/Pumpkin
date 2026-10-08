use heck::ToShoutySnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};

pub fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        Many(Vec<T>),
        One(T),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::Many(values) => values,
        OneOrMany::One(value) => vec![value],
    })
}
fn collect(base: &Path, dir: &Path, files: &mut Vec<(String, Value)>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, files);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            files.push((
                relative,
                serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap(),
            ));
        }
    }
}
fn registry_holders(unsupported: &mut BTreeSet<String>) -> TokenStream {
    let mut rows = Vec::new();
    for registry in ["predicate", "item_modifier"] {
        let base = Path::new("../../assets/datapack/data/minecraft").join(registry);
        if !base.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect(&base, &base, &mut files);
        for (name, value) in files {
            let wrapper = if registry == "predicate" {
                serde_json::json!({"pools":[{"rolls":1,"condition":value}]})
            } else {
                serde_json::json!({"modifier":value})
            };
            assert!(
                pumpkin_util::loot_table::parse_loot_table(&wrapper.to_string(), unsupported)
                    .is_some()
            );
            rows.push((registry, name, serde_json::to_string(&value).unwrap()));
        }
    }
    rows.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let len = rows.len();
    let rows = rows
        .iter()
        .map(|(registry, name, json)| quote! { (#registry, #name, #json) });
    quote! {
        static LOOT_REGISTRY_HOLDERS: [(&str, &str, &str); #len] = [#(#rows),*];
        /// Return vanilla JSON for predicate and item modifier registry holders.
        #[must_use]
        pub fn get_loot_registry_entry(registry: &str, key: &str) -> Option<&'static str> {
            let key = key.strip_prefix("minecraft:").unwrap_or(key);
            LOOT_REGISTRY_HOLDERS.binary_search_by_key(&(registry, key), |&(r, k, _)| (r, k))
                .ok().map(|i| LOOT_REGISTRY_HOLDERS[i].2)
        }
    }
}

// StatePropertiesPredicate.RangedMatcher uses the property's typed ordering.
fn property_types() -> TokenStream {
    let properties: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("../../assets/properties.json").unwrap()).unwrap();
    let blocks: Value =
        serde_json::from_str(&fs::read_to_string("../../assets/blocks.json").unwrap()).unwrap();
    let mut rows = Vec::new();
    for block in blocks["blocks"].as_array().unwrap() {
        let block_name = block["name"].as_str().unwrap();
        for key in block["properties"].as_array().unwrap() {
            let property = properties.iter().find(|p| p["hash_key"] == *key).unwrap();
            let name = property["serialized_name"].as_str().unwrap();
            let kind = match property["type"].as_str().unwrap() {
                "boolean" => quote! { LootPropertyType::Boolean },
                "int" => {
                    let min = property["min"].as_i64().unwrap() as i32;
                    let max = property["max"].as_i64().unwrap() as i32;
                    quote! { LootPropertyType::Integer(#min, #max) }
                }
                "enum" => {
                    let mut values: Vec<_> = property["values"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect();
                    let direction = |name: &str| {
                        use pumpkin_util::BlockDirection;
                        match name {
                            "down" => Some(BlockDirection::Down),
                            "up" => Some(BlockDirection::Up),
                            "north" => Some(BlockDirection::North),
                            "south" => Some(BlockDirection::South),
                            "west" => Some(BlockDirection::West),
                            "east" => Some(BlockDirection::East),
                            _ => None,
                        }
                    };
                    // Direction enum order differs from Facing's extracted possible-value order.
                    if values.iter().all(|v| direction(v).is_some()) {
                        values.sort_by_key(|v| direction(v).unwrap() as u8);
                    }
                    quote! { LootPropertyType::Enum(&[#(#values),*]) }
                }
                _ => unreachable!(),
            };
            rows.push((block_name, name, kind));
        }
    }
    rows.sort_by_key(|&(block, name, _)| (block, name));
    let len = rows.len();
    let rows = rows
        .iter()
        .map(|(block, name, kind)| quote! { ((#block, #name), #kind) });
    quote! {
        pub enum LootPropertyType { Boolean, Integer(i32, i32), Enum(&'static [&'static str]) }
        impl LootPropertyType {
            #[must_use]
            pub fn value(&self, value: &str) -> Option<i32> {
                match self {
                    Self::Boolean => match value { "false" => Some(0), "true" => Some(1), _ => None },
                    Self::Integer(min, max) => value.parse::<i32>().ok().filter(|value| value >= min && value <= max),
                    Self::Enum(values) => values.iter().position(|name| *name == value).map(|index| index as i32),
                }
            }
        }
        static LOOT_PROPERTY_TYPES: [((&str, &str), LootPropertyType); #len] = [#(#rows),*];
        #[must_use]
        pub fn get_loot_property_type(block: &str, name: &str) -> Option<&'static LootPropertyType> {
            LOOT_PROPERTY_TYPES.binary_search_by_key(&(block, name), |(key, _)| *key).ok().map(|index| &LOOT_PROPERTY_TYPES[index].1)
        }
    }
}

pub fn build() -> TokenStream {
    let base = Path::new("../../assets/datapack/data/minecraft/loot_table");
    let mut files = Vec::new();
    collect(base, base, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut tokens = TokenStream::new();
    let mut rows = Vec::new();
    let mut unsupported = BTreeSet::new();
    for (relative, value) in files {
        let json = serde_json::to_string(&value).unwrap();
        assert!(pumpkin_util::loot_table::parse_loot_table(&json, &mut unsupported).is_some());
        let ident = format_ident!("{}", relative.replace('/', "_").to_shouty_snake_case());
        tokens.extend(quote! { pub static #ident: LootTable = LootTable::new(#json); });
        rows.push((format!("minecraft:{relative}"), ident.clone()));
        rows.push((relative, ident));
    }
    let holders = registry_holders(&mut unsupported);
    for kind in unsupported {
        if kind.starts_with("modifier ") {
            eprintln!("Unsupported loot {kind}");
        } else {
            eprintln!("Unsupported loot condition: {kind} (fails closed)");
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let len = rows.len();
    let rows = rows.iter().map(|(key, ident)| quote! { (#key, &#ident) });
    let property_types = property_types();
    quote! {
        pub use pumpkin_util::loot_table::*;
        #holders
        #property_types
        #tokens
        static LOOT_TABLES_BY_KEY: [(&str, &LootTable); #len] = [#(#rows),*];
        #[must_use]
        pub fn get_loot_table(key: &str) -> Option<&'static LootTable> {
            LOOT_TABLES_BY_KEY.binary_search_by_key(&key, |&(k, _)| k).ok().map(|i| LOOT_TABLES_BY_KEY[i].1)
        }
        #[must_use]
        pub fn get_chest_loot_table(key: &str) -> Option<&'static LootTable> { get_loot_table(key) }
    }
}
