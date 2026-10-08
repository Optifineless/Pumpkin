use std::{collections::HashSet, fs, path::Path};

use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

// Holder sets in the stock SpawnContext selectors, including nested biome/structure tags.
fn holders(value: &Value, registry: &str) -> Vec<String> {
    fn visit(value: &Value, registry: &str, result: &mut Vec<String>) {
        match value {
            Value::Array(values) => values.iter().for_each(|v| visit(v, registry, result)),
            Value::String(name) => {
                if let Some(tag) = name.strip_prefix("#minecraft:") {
                    let path =
                        format!("../../assets/datapack/data/minecraft/tags/{registry}/{tag}.json");
                    let json: Value =
                        serde_json::from_str(&fs::read_to_string(path).expect("read variant tag"))
                            .expect("parse variant tag");
                    visit(&json["values"], registry, result);
                } else {
                    result.push(name.clone());
                }
            }
            _ => panic!("unsupported variant holder set: {value}"),
        }
    }
    let mut result = Vec::new();
    visit(value, registry, &mut result);
    let mut seen = HashSet::new();
    result.retain(|name| seen.insert(name.clone()));
    result
}

pub fn build() -> TokenStream {
    let mut arms = Vec::new();
    for species in [
        "cow",
        "pig",
        "chicken",
        "wolf",
        "frog",
        "cat",
        "zombie_nautilus",
    ] {
        let dir =
            Path::new("../../assets/datapack/data/minecraft").join(format!("{species}_variant"));
        let mut files = fs::read_dir(dir)
            .expect("variant directory")
            .map(|entry| entry.expect("variant file").path())
            .collect::<Vec<_>>();
        files.sort();
        let mut selectors = Vec::new();
        for file in files {
            if file.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let name = format!(
                "minecraft:{}",
                file.file_stem().expect("variant name").to_string_lossy()
            );
            let data: Value =
                serde_json::from_str(&fs::read_to_string(&file).expect("read variant"))
                    .expect("parse variant");
            for selector in data["spawn_conditions"]
                .as_array()
                .expect("variant selectors")
            {
                let priority = selector["priority"].as_i64().expect("selector priority") as i32;
                let condition = &selector["condition"];
                let check = match condition["type"].as_str() {
                    None if condition.is_null() => quote! { Condition::Always },
                    Some("minecraft:biome") => {
                        let names = holders(&condition["biomes"], "worldgen/biome");
                        quote! { Condition::Biomes(&[#(#names),*]) }
                    }
                    Some("minecraft:structure") => {
                        let names = holders(&condition["structures"], "worldgen/structure");
                        quote! { Condition::Structures(&[#(#names),*]) }
                    }
                    Some("minecraft:moon_brightness") => {
                        let min = condition["range"]["min"].as_f64().unwrap_or(f64::MIN);
                        let max = condition["range"]["max"].as_f64().unwrap_or(f64::MAX);
                        quote! { Condition::MoonBrightness { min: #min, max: #max } }
                    }
                    _ => panic!("unsupported spawn condition: {condition}"),
                };
                selectors.push(
                    quote! { Selector { name: #name, priority: #priority, condition: #check } },
                );
            }
        }
        arms.push(quote! { #species => &[#(#selectors),*] });
    }
    quote! {
        pub enum Condition {
            Always,
            Biomes(&'static [&'static str]),
            Structures(&'static [&'static str]),
            MoonBrightness { min: f64, max: f64 },
        }
        pub struct Selector {
            pub name: &'static str,
            pub priority: i32,
            pub condition: Condition,
        }
        pub fn selectors(species: &str) -> &'static [Selector] {
            match species { #(#arms,)* _ => &[] }
        }
    }
}
