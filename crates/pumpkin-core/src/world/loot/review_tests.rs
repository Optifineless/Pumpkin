use super::tests::{count, parse};
use super::*;
use pumpkin_data::{data_component::DataComponent, data_component_impl::EnchantmentsImpl};
use serde_json::json;
#[test]
fn placeholder_components_never_match_even_when_inverted() {
    let mut tool = ItemStack::new(1, &Item::IRON_SWORD);
    tool.patch.push((
        DataComponent::BreakSound,
        Some(Box::new(pumpkin_data::data_component_impl::BreakSoundImpl)),
    ));
    for inverted in [false, true] {
        let mut condition = json!({"type":"minecraft:match_tool","predicate":{"components":{"minecraft:break_sound":"minecraft:entity.cow.hurt"}}});
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
            .is_empty()
        );
    }
}
#[test]
fn wither_skeleton_coal_grows_from_empty_count() {
    // entities/wither_skeleton coal starts at -1; ItemStack.grow reads zero.
    let mut stack = new_stack("minecraft:coal").unwrap();
    stack.count = -1;
    let mut sword = ItemStack::new(1, &Item::IRON_SWORD);
    sword.patch.push((
        DataComponent::Enchantments,
        Some(Box::new(EnchantmentsImpl {
            enchantment: std::borrow::Cow::Owned(vec![(&pumpkin_data::Enchantment::LOOTING, 1)]),
        })),
    ));
    let params = LootContextParameters {
        attacking_entity_state: Some(EntityLootState {
            is_living: Some(true),
            equipment: [("mainhand".to_owned(), sword)].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let table = parse(
        &json!({"modifier":{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":1}}),
    );
    apply_functions(
        &table.functions,
        &mut stack,
        &params,
        &mut LootRandom::seeded(1),
    );
    assert_eq!(stack.count, 1);
}
#[test]
fn unsupported_count_providers_leave_the_stack_unchanged() {
    for provider in [
        json!({"type":"minecraft:score","target":"this","score":"test"}),
        json!("custom:amount"),
        json!({"type":"custom:unknown"}),
        json!({"type":"minecraft:uniform","min":1,"max":{"type":"minecraft:score"}}),
    ] {
        for modifier in [
            json!({"type":"minecraft:set_count","count":provider}),
            json!({"type":"minecraft:limit_count","limit":{"max":provider}}),
        ] {
            let table = parse(
                &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:coal","modifier":modifier}]}]}),
            );
            assert_eq!(
                count(
                    &generate_dynamic_loot_with_context(
                        &table,
                        1,
                        &LootContextParameters::default()
                    ),
                    &Item::COAL
                ),
                1
            );
        }
    }
}

#[test]
fn nested_random_modifiers_run_between_child_rolls() {
    // Java Random(1): nextInt(10) yields 5, 8, 7, 3; streamed counts are 6+9, 8+4.
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":{"pools":[{"rolls":2,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:set_count","count":{"min":1,"max":10}}}]}]},"modifier":{"type":"minecraft:set_count","count":{"min":1,"max":10},"add":true}}]}]}),
    );
    let drops = generate_dynamic_loot(&table, 1);
    assert_eq!(
        drops.iter().map(|s| s.item_count).collect::<Vec<_>>(),
        [15, 12]
    );
    let table = parse(
        &json!({"modifier":{"type":"minecraft:set_count","count":3},"pools":[{"rolls":1,"modifier":{"type":"minecraft:set_count","count":2,"add":true},"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:set_count","count":8}}]}]}),
    );
    assert_eq!(generate_dynamic_loot(&table, 1)[0].item_count, 3);
}
#[test]
fn ranged_properties_use_enum_boolean_and_integer_types() {
    use pumpkin_data::Block;
    for (block, props, ranges, expected) in [
        (
            &Block::OAK_SLAB,
            vec![("type", "double")],
            json!({"type":{"min":"double","max":"double"}}),
            true,
        ),
        (
            &Block::OAK_SLAB,
            vec![("waterlogged", "false")],
            json!({"waterlogged":{"min":"false","max":"true"}}),
            true,
        ),
        (
            &Block::WHEAT,
            vec![("age", "7")],
            json!({"age":{"min":"3","max":"7"}}),
            true,
        ),
        (
            &Block::WHEAT,
            vec![("age", "7")],
            json!({"age":{"min":"0","max":"8"}}),
            false,
        ),
        (
            &Block::OAK_SLAB,
            vec![("type", "double")],
            json!({"type":{"min":"invalid"}}),
            false,
        ),
        (
            &Block::WALL_TORCH,
            vec![("facing", "south")],
            json!({"facing":{"min":"north","max":"west"}}),
            true,
        ),
    ] {
        let state = block.from_properties(&props).to_state_id(block).to_state();
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","condition":{"type":"minecraft:match_block","state":ranges}}]}]}),
        );
        let params = LootContextParameters {
            block_state: Some(state),
            ..Default::default()
        };
        assert_eq!(
            !generate_dynamic_loot_with_context(&table, 1, &params).is_empty(),
            expected
        );
    }
}
#[test]
fn signed_weight_is_clamped_after_quality_and_luck() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","weight":-1,"quality":1}]}]}),
    );
    for (luck, expected) in [(1.0, 0), (2.0, 1)] {
        assert_eq!(
            count(
                &generate_dynamic_loot_with_context(
                    &table,
                    1,
                    &LootContextParameters {
                        luck,
                        ..Default::default()
                    }
                ),
                &Item::DIAMOND
            ),
            expected
        );
    }
}
#[test]
fn equipment_requires_living_entities_and_tests_empty_slots() {
    for (living, expected) in [(false, false), (true, true)] {
        let params = LootContextParameters {
            this_entity_state: Some(EntityLootState {
                is_living: Some(living),
                ..Default::default()
            }),
            ..Default::default()
        };
        for equipment in [json!({}), json!({"head":{"count":0}})] {
            let table = parse(
                &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","condition":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:equipment":equipment}}}]}]}),
            );
            assert_eq!(
                !generate_dynamic_loot_with_context(&table, 1, &params).is_empty(),
                expected
            );
        }
    }
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    helmet.add_enchantment(&pumpkin_data::Enchantment::RESPIRATION, 3);
    let mut params = LootContextParameters {
        attacking_entity_state: Some(EntityLootState {
            is_living: Some(true),
            equipment: [("head".to_owned(), helmet)].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        attacker_enchantment_level(&params, "minecraft:respiration"),
        3
    );
    assert_eq!(attacker_enchantment_level(&params, "minecraft:looting"), 0);
    params
        .attacking_entity_state
        .as_mut()
        .unwrap()
        .equipment
        .get_mut("head")
        .unwrap()
        .item_count = 0;
    assert_eq!(
        attacker_enchantment_level(&params, "minecraft:respiration"),
        0
    );
}
#[test]
fn holder_lists_preserve_expand() {
    for expand in [false, true] {
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:tag","items":["minecraft:apple","minecraft:diamond"],"expand":expand}]}]}),
        );
        assert_eq!(
            generate_dynamic_loot(&table, 1).len(),
            if expand { 1 } else { 2 }
        );
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","expand":expand,"value":[{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]},{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}]}]}]}),
        );
        assert_eq!(
            generate_dynamic_loot(&table, 1).len(),
            if expand { 1 } else { 2 }
        );
    }
}
#[test]
fn air_with_positive_count_is_still_empty() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:air","modifier":{"type":"minecraft:set_count","count":10}}]}]}),
    );
    assert!(generate_dynamic_loot(&table, 1).is_empty());
}
#[test]
fn vanilla_parent_resolves_overridden_child_and_cycles_stop() {
    let registry = std::sync::Arc::new(crate::data::datapack::DatapackManager::new());
    registry.insert_loot_table("minecraft:entities/sheep/white".to_owned(), std::sync::Arc::new(parse(&json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}))));
    let params = LootContextParameters {
        registry: Some(registry.clone()),
        this_entity_state: Some(EntityLootState {
            sheared: Some(false),
            components: [("minecraft:sheep/color".to_owned(), json!("white"))].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let table = pumpkin_data::loot_table::get_loot_table("entities/sheep").unwrap();
    assert_eq!(
        count(
            &generate_loot_with_context(table, 1, &params),
            &Item::DIAMOND
        ),
        1
    );
    registry.insert_loot_table("custom:cycle".to_owned(), std::sync::Arc::new(parse(&json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"custom:cycle"}]}]}))));
    let cycle = registry.get_loot_table("custom:cycle").unwrap();
    assert!(generate_loot_from_handle(&cycle, 1, &params).is_empty());
}

#[test]
fn random_sequences_advance_and_reset_using_vanilla_seed_hashing() {
    // Actual 26.3 RandomSequence(123, "minecraft:test/loot").random().nextInt(1000).
    let id = pumpkin_util::identifier::Identifier::parse("minecraft:test/loot").unwrap();
    let mut sequences = crate::world::random_sequences::RandomSequences::new();
    let expected = [823, 37, 847, 879, 647, 752];
    for value in expected {
        assert_eq!(
            sequences
                .get_or_create(&id, 123)
                .random_between_inclusive(0, 999),
            value
        );
    }
    sequences.reset(&id, 123);
    assert_eq!(
        sequences
            .get_or_create(&id, 123)
            .random_between_inclusive(0, 999),
        823
    );
}
#[test]
fn container_placement_continues_the_seeded_loot_random_source() {
    // Actual 26.3 LootTable.getAvailableSlots/shuffleAndSplitItems, following Random(37).nextInt(20).
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:set_count","count":{"min":1,"max":20}}}]}]}),
    );
    let handle = LootTableHandle::Dynamic(std::sync::Arc::new(table));
    let inventory: std::sync::Arc<dyn pumpkin_inventory::Inventory> =
        std::sync::Arc::new(pumpkin_inventory::SimpleInventory::new(9));
    fill_chest_inventory_handle(&inventory, &handle, 37);
    assert_eq!(
        (0..9)
            .map(|slot| inventory.get_stack(slot).item_count)
            .collect::<Vec<_>>(),
        [4, 0, 0, 0, 1, 1, 0, 0, 0]
    );
}
#[test]
fn empty_counts_are_read_as_zero_by_add_bonus_and_limits() {
    let params = LootContextParameters {
        tool: Some(ItemStack::EMPTY.clone()),
        ..Default::default()
    };
    for (modifier, expected) in [
        (
            json!({"type":"minecraft:set_count","count":1,"add":true}),
            1,
        ),
        (
            json!({"type":"minecraft:apply_bonus","formula":"minecraft:binomial_with_bonus_count","parameters":{"extra":1,"probability":1.0}}),
            1,
        ),
        (
            json!({"type":"minecraft:apply_bonus","formula":"minecraft:ore_drops"}),
            0,
        ),
        (json!({"type":"minecraft:limit_count","limit":{"max":2}}), 0),
    ] {
        let table = parse(&json!({"modifier":modifier}));
        let mut stack = new_stack("minecraft:coal").unwrap();
        stack.count = -1;
        apply_functions(
            &table.functions,
            &mut stack,
            &params,
            &mut LootRandom::seeded(1),
        );
        assert_eq!(stack.count, expected);
    }
}
#[test]
fn placeholder_component_writes_do_not_replace_existing_values() {
    let table = parse(
        &json!({"modifier":{"type":"minecraft:set_components","components":{"minecraft:attribute_modifiers":[],"minecraft:break_sound":"minecraft:entity.cow.hurt"}}}),
    );
    let mut stack = new_stack("minecraft:iron_sword").unwrap();
    apply_functions(
        &table.functions,
        &mut stack,
        &LootContextParameters::default(),
        &mut LootRandom::seeded(1),
    );
    assert!(stack.stack.patch.iter().all(|(id, _)| !matches!(
        id,
        DataComponent::AttributeModifiers | DataComponent::BreakSound
    )));
    let mut tool = ItemStack::new(1, &Item::IRON_SWORD);
    tool.patch.push((
        DataComponent::BreakSound,
        Some(Box::new(pumpkin_data::data_component_impl::BreakSoundImpl)),
    ));
    let table = parse(
        &json!({"modifier":{"type":"minecraft:copy_components","source":"tool","include":["minecraft:attribute_modifiers","minecraft:break_sound"]}}),
    );
    apply_functions(
        &table.functions,
        &mut stack,
        &LootContextParameters {
            tool: Some(tool),
            ..Default::default()
        },
        &mut LootRandom::seeded(1),
    );
    assert!(stack.stack.patch.iter().all(|(id, _)| !matches!(
        id,
        DataComponent::AttributeModifiers | DataComponent::BreakSound
    )));
}

#[test]
fn copy_state_copies_only_properties_declared_by_the_configured_block() {
    let block = &pumpkin_data::Block::BEEHIVE;
    let state = block
        .from_properties(&[("honey_level", "5")])
        .to_state_id(block)
        .to_state();
    for (configured, expected) in [
        ("minecraft:beehive", Some("5")),
        ("minecraft:stone", Some("0")),
    ] {
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:beehive","modifier":{"type":"minecraft:copy_state","block":configured,"properties":["honey_level"]}}]}]}),
        );
        let drops = generate_dynamic_loot_with_context(
            &table,
            1,
            &LootContextParameters {
                block_state: Some(state),
                ..Default::default()
            },
        );
        assert_eq!(
            drops[0]
                .get_data_component::<pumpkin_data::data_component_impl::BlockStateImpl>()
                .and_then(|props| props
                    .properties
                    .iter()
                    .find(|(name, _)| name == "honey_level")
                    .map(|(_, value)| value.as_ref())),
            expected
        );
    }
}
