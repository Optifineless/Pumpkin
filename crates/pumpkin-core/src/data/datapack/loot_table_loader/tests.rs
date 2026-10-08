use super::*;
use pumpkin_util::loot_table::LootEntryKind;
#[test]
fn loader_keeps_cross_pack_references_for_runtime_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let tables = temp.path().join("data/custom/loot_table");
    fs::create_dir_all(&tables).unwrap();
    fs::write(tables.join("parent.json"), r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"other:child"}]}]}"#).unwrap();
    let mut registry = HashMap::new();
    load_loot_tables_from_dir("custom", &tables, &mut registry);
    assert!(matches!(
        registry["custom:parent"].pools[0].entries[0].kind,
        LootEntryKind::Tables { .. }
    ));
}

#[test]
fn references_observe_final_pack_precedence_and_registry_reload() {
    let temp = tempfile::tempdir().unwrap();
    let world = temp.path().join("world");
    let low = world.join("datapacks/low");
    let high = world.join("datapacks/high");
    for pack in [&low, &high] {
        fs::create_dir_all(pack.join("data/custom/loot_table")).unwrap();
        fs::create_dir_all(pack.join("data/custom/predicate")).unwrap();
        fs::create_dir_all(pack.join("data/custom/item_modifier")).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"min_format":94,"max_format":94,"description":"test"}}"#,
        )
        .unwrap();
    }
    fs::write(low.join("data/custom/loot_table/parent.json"), r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"custom:child","condition":"custom:gate","modifier":"custom:amount"}]}]}"#).unwrap();
    fs::write(
        low.join("data/custom/loot_table/child.json"),
        r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]}"#,
    )
    .unwrap();
    fs::write(high.join("data/custom/loot_table/child.json"), r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#).unwrap();
    fs::write(
        low.join("data/custom/predicate/gate.json"),
        r#"{"type":"minecraft:random_chance","chance":0}"#,
    )
    .unwrap();
    fs::write(
        high.join("data/custom/predicate/gate.json"),
        r#"{"type":"minecraft:random_chance","chance":1}"#,
    )
    .unwrap();
    fs::write(
        low.join("data/custom/item_modifier/amount.json"),
        r#"{"type":"minecraft:set_count","count":1}"#,
    )
    .unwrap();
    fs::write(
        high.join("data/custom/item_modifier/amount.json"),
        r#"{"type":"minecraft:set_count","count":3}"#,
    )
    .unwrap();
    let registry = Arc::new(crate::data::datapack::DatapackManager::new());
    let recipes = crate::server::recipe::RecipeManager::new();
    registry.load_all(
        &world,
        &["file/low".to_owned(), "file/high".to_owned()],
        &recipes,
    );
    let table = registry.get_loot_table("custom:parent").unwrap();
    let params = crate::world::loot::LootContextParameters {
        registry: Some(registry.clone()),
        ..Default::default()
    };
    let items = crate::world::loot::generate_loot_from_handle(&table, 1, &params);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item, &pumpkin_data::item::Item::DIAMOND);
    assert_eq!(items[0].item_count, 3);
    fs::write(
        high.join("data/custom/item_modifier/amount.json"),
        r#"{"type":"minecraft:set_count","count":4}"#,
    )
    .unwrap();
    registry.load_all(
        &world,
        &["file/low".to_owned(), "file/high".to_owned()],
        &recipes,
    );
    assert_eq!(
        crate::world::loot::generate_loot_from_handle(&table, 1, &params)[0].item_count,
        4
    );
    fs::write(high.join("data/custom/predicate/gate.json"), "null").unwrap();
    registry.load_all(
        &world,
        &["file/low".to_owned(), "file/high".to_owned()],
        &recipes,
    );
    assert!(crate::world::loot::generate_loot_from_handle(&table, 1, &params).is_empty());
}

#[test]
fn loot_table_tags_merge_replace_and_reject_required_cycles() {
    use crate::world::loot::{LootContextParameters, generate_dynamic_loot_with_context};
    let temp = tempfile::tempdir().unwrap();
    let world = temp.path().join("world");
    let low = world.join("datapacks/low");
    let high = world.join("datapacks/high");
    for pack in [&low, &high] {
        fs::create_dir_all(pack.join("data/custom/tags/loot_table")).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"min_format":94,"max_format":94,"description":"test"}}"#,
        )
        .unwrap();
    }
    fs::write(
        low.join("data/custom/tags/loot_table/values.json"),
        r#"{"values":["minecraft:entities/sheep/white"]}"#,
    )
    .unwrap();
    fs::write(high.join("data/custom/tags/loot_table/values.json"), r#"{"values":[{"id":"minecraft:blocks/stone"},{"id":"custom:missing","required":false},"minecraft:entities/sheep/white"]}"#).unwrap();
    fs::write(
        high.join("data/custom/tags/loot_table/cycle.json"),
        r##"{"values":["#custom:cycle","#custom:cycle","minecraft:entities/sheep/white"]}"##,
    )
    .unwrap();
    let registry = Arc::new(crate::data::datapack::DatapackManager::new());
    let recipes = crate::server::recipe::RecipeManager::new();
    let enabled = ["file/low".to_owned(), "file/high".to_owned()];
    registry.load_all(&world, &enabled, &recipes);
    let params = LootContextParameters {
        registry: Some(registry.clone()),
        ..Default::default()
    };
    let table = parse_loot_table(r##"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"#custom:values"}]}]}"##).unwrap();
    let drops = generate_dynamic_loot_with_context(&table, 1, &params);
    assert_eq!(drops.len(), 2);
    assert_eq!(drops[0].item, &pumpkin_data::item::Item::WHITE_WOOL);
    assert_eq!(drops[1].item, &pumpkin_data::item::Item::COBBLESTONE);
    let cycle = parse_loot_table(r##"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"#custom:cycle"}]}]}"##).unwrap();
    assert!(generate_dynamic_loot_with_context(&cycle, 1, &params).is_empty());
    fs::write(
        high.join("data/custom/tags/loot_table/values.json"),
        r#"{"replace":true,"values":["minecraft:blocks/stone"]}"#,
    )
    .unwrap();
    registry.load_all(&world, &enabled, &recipes);
    let drops = generate_dynamic_loot_with_context(&table, 1, &params);
    assert_eq!(drops.len(), 1);
    assert_eq!(drops[0].item, &pumpkin_data::item::Item::COBBLESTONE);
}
