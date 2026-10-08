use super::tests::{count, parse};
use super::*;
use pumpkin_data::{data_component::DataComponent, data_component_impl::*};
use serde_json::json;

#[test]
fn lossy_exact_predicates_are_unsupported_before_inversion_and_writes_are_skipped() {
    for (name, value) in [
        (
            "firework_explosion",
            json!({"shape":"small_ball","colors":[16711680]}),
        ),
        (
            "fireworks",
            json!({"explosions":[{"shape":"small_ball","colors":[16711680]}]}),
        ),
        ("tool", json!({"rules":[],"default_mining_speed":2.5})),
        ("item_name", json!({"text":"Name","bold":true})),
        ("container", json!([])),
        (
            "enchantments",
            json!({"minecraft:looting":1,"minecraft:fortune":2}),
        ),
    ] {
        let id = DataComponent::try_from_name(name).unwrap();
        let mut tool = ItemStack::new(1, &Item::IRON_SWORD);
        // Use the legacy decoder to reproduce the value which used to match lossy input.
        let component = read_data(
            id,
            &crate::data::datapack::context_provider_loader::json_value_to_nbt(&value),
        )
        .unwrap();
        tool.patch.push((id, Some(component)));
        for inverted in [false, true] {
            let mut condition = json!({"type":"minecraft:match_tool","predicate":{"components":{format!("minecraft:{name}"):value}}});
            if inverted {
                condition = json!({"type":"minecraft:inverted","term":condition});
            }
            let table = parse(
                &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","condition":condition}]}]}),
            );
            assert!(
                generate_dynamic_loot_with_context(
                    &table,
                    1,
                    &LootContextParameters {
                        tool: Some(tool.clone()),
                        ..Default::default()
                    }
                )
                .is_empty(),
                "{name}, inverted={inverted}"
            );
        }
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:set_components","components":{format!("minecraft:{name}"):value}}}]}]}),
        );
        assert!(
            generate_dynamic_loot(&table, 1)[0].patch.is_empty(),
            "{name}"
        );
    }
}

#[test]
fn empty_equipment_uses_air_and_requires_enchantment_components() {
    let mut zero = ItemStack::new(0, &Item::IRON_SWORD);
    zero.add_enchantment(&pumpkin_data::Enchantment::LOOTING, 3);
    for stack in [ItemStack::EMPTY.clone(), zero] {
        assert_eq!(
            item_predicate(&json!({"items":"minecraft:air"}), &stack),
            Some(true)
        );
        assert_eq!(
            item_predicate(&json!({"items":"minecraft:iron_sword"}), &stack),
            Some(false)
        );
        for kind in ["minecraft:enchantments", "minecraft:stored_enchantments"] {
            for rules in [json!([]), json!([{"enchantments":"minecraft:looting"}])] {
                assert_eq!(
                    item_predicate(&json!({"predicates":{kind: rules}}), &stack),
                    Some(false)
                );
            }
        }
    }
    let living = EntityLootState {
        is_living: Some(true),
        ..Default::default()
    };
    assert_eq!(
        entity_predicate(
            &json!({"minecraft:equipment":{"head":{"predicates":{"minecraft:enchantments":[]}}}}),
            &living
        ),
        Some(false)
    );
}

#[test]
fn item_tags_default_namespace_and_unresolved_alternatives_fall_through() {
    for expand in [false, true] {
        for name in ["#logs", "#minecraft:logs"] {
            let table = parse(
                &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:tag","items":name,"expand":expand}]}]}),
            );
            assert!(!generate_dynamic_loot(&table, 1).is_empty());
        }
        for name in [json!("#custom:missing"), json!([])] {
            let empty = name.is_array();
            let table = parse(
                &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:alternatives","children":[{"type":"minecraft:tag","items":name,"expand":expand},{"type":"minecraft:item","name":"minecraft:apple"}]}]}]}),
            );
            assert_eq!(
                count(&generate_dynamic_loot(&table, 1), &Item::APPLE),
                u32::from(!empty)
            );
        }
    }
}

#[test]
fn repeated_reference_graphs_are_bounded_and_caches_follow_reload() {
    use std::{fs, sync::Arc};
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("datapacks/test");
    fs::create_dir_all(pack.join("data/test/predicate")).unwrap();
    fs::create_dir_all(pack.join("data/test/tags/loot_table")).unwrap();
    fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":94,"max_format":94,"description":"test"}}"#,
    )
    .unwrap();
    for index in 0..17 {
        let next = format!("test:p{}", index + 1);
        fs::write(
            pack.join(format!("data/test/predicate/p{index}.json")),
            json!({"type":"minecraft:all_of","terms":[next,next]}).to_string(),
        )
        .unwrap();
        fs::write(
            pack.join(format!("data/test/tags/loot_table/p{index}.json")),
            json!({"values":[format!("#test:p{}",index+1), format!("#test:p{}",index+1)]})
                .to_string(),
        )
        .unwrap();
    }
    let leaf = pack.join("data/test/predicate/p17.json");
    fs::write(
        &leaf,
        json!({"type":"minecraft:random_chance","chance":1}).to_string(),
    )
    .unwrap();
    fs::write(
        pack.join("data/test/tags/loot_table/p17.json"),
        r#"{"values":["minecraft:entities/sheep"]}"#,
    )
    .unwrap();
    let manager = Arc::new(crate::data::datapack::DatapackManager::new());
    let recipes = crate::server::recipe::RecipeManager::new();
    let packs = ["file/test".to_owned()];
    manager.load_all(dir.path(), &packs, &recipes);
    let params = LootContextParameters {
        registry: Some(manager.clone()),
        ..Default::default()
    };
    assert!(!check_condition(
        &LootCondition::Reference("test:p0".to_owned()),
        &params,
        &mut LootRandom::seeded(1)
    ));
    let mut rng = LootRandom::seeded(1);
    assert_eq!(
        table_holders("#test:p0", &params, &mut rng).unwrap(),
        ["minecraft:entities/sheep"]
    );
    assert!(rng.charge_work(65000)); // shared tag subgraphs are visited once, not exponentially.
    assert!(check_condition(
        &LootCondition::Reference("test:p17".to_owned()),
        &params,
        &mut LootRandom::seeded(1)
    ));
    fs::write(
        leaf,
        json!({"type":"minecraft:random_chance","chance":0}).to_string(),
    )
    .unwrap();
    manager.load_all(dir.path(), &packs, &recipes);
    assert!(!check_condition(
        &LootCondition::Reference("test:p17".to_owned()),
        &params,
        &mut LootRandom::seeded(1)
    ));
}

#[test]
fn death_loot_uses_attacker_looting_independently_of_tool_fortune() {
    use pumpkin_data::Enchantment;
    // Port of the merged death regression: separate attacker equipment from the block TOOL.
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:rotten_flesh","modifier":{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":{"type":"minecraft:uniform","min":0,"max":1}}}]}]}),
    );
    let mut tool = ItemStack::new(1, &Item::DIAMOND_PICKAXE);
    tool.add_enchantment(&Enchantment::FORTUNE, 3);
    let mut params = LootContextParameters {
        tool: Some(tool),
        ..Default::default()
    };
    for seed in 1..33 {
        assert_eq!(
            count(
                &generate_dynamic_loot_with_context(&table, seed, &params),
                &Item::ROTTEN_FLESH
            ),
            1
        );
    }
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&Enchantment::LOOTING, 3);
    params.tool = None;
    params.attacking_entity_state = Some(EntityLootState {
        is_living: Some(true),
        equipment: [("mainhand".to_owned(), sword)].into(),
        ..Default::default()
    });
    assert!((1..33).any(|seed| count(
        &generate_dynamic_loot_with_context(&table, seed, &params),
        &Item::ROTTEN_FLESH
    ) > 1));
}
