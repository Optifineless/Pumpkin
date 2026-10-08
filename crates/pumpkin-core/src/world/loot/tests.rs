use super::*;
use pumpkin_data::Block;
use pumpkin_data::{data_component::DataComponent, entity::EntityType};
use serde_json::Value;
use serde_json::json;

#[test]
fn non_living_entities_ignore_only_living_entity_flags() {
    // EntityFlagsPredicate.matches ignores baby/fall-flying only on non-living entities.
    let mut entity = EntityLootState {
        is_living: Some(false),
        flags: [
            ("is_baby".to_owned(), false),
            ("is_fall_flying".to_owned(), false),
            ("is_on_fire".to_owned(), false),
        ]
        .into(),
        ..Default::default()
    };
    let predicate =
        json!({"minecraft:flags":{"is_baby":true,"is_fall_flying":true,"is_on_fire":false}});
    assert_eq!(entity_predicate(&predicate, &entity), Some(true));
    entity.is_living = Some(true);
    assert_eq!(entity_predicate(&predicate, &entity), Some(false));
    entity.is_living = Some(false);
    entity.flags.insert("is_on_fire".to_owned(), true);
    assert_eq!(entity_predicate(&predicate, &entity), Some(false));
}

fn drops(key: &str, seed: i64, params: &LootContextParameters) -> Vec<ItemStack> {
    let table = pumpkin_data::loot_table::get_loot_table(key).expect("generated vanilla table");
    generate_loot_with_context(table, seed, params)
}
pub(super) fn count(items: &[ItemStack], item: &Item) -> u32 {
    items
        .iter()
        .filter(|stack| stack.item == item)
        .map(|stack| u32::from(stack.item_count))
        .sum()
}
fn block_context(block: &Block, properties: &[(&str, &str)]) -> LootContextParameters {
    let state = block
        .from_properties(properties)
        .to_state_id(block)
        .to_state();
    LootContextParameters {
        block_state: Some(state),
        tool: Some(ItemStack::new(1, &Item::IRON_PICKAXE)),
        ..Default::default()
    }
}
fn entity_context(entity: EntityLootState) -> LootContextParameters {
    LootContextParameters {
        this_entity: entity.entity_type,
        this_entity_state: Some(entity),
        killed_by_player: Some(true),
        last_damage_player_state: Some(EntityLootState {
            entity_type: Some(&EntityType::PLAYER),
            ..Default::default()
        }),
        ..Default::default()
    }
}
#[test]
fn attacking_player_target_uses_last_damage_player_and_absent_predicate_is_optional() {
    // LootContext.EntityTarget.ATTACKING_PLAYER uses LAST_DAMAGE_PLAYER, not ATTACKING_ENTITY.
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","condition":{"type":"minecraft:entity_properties","entity":"attacking_player","predicate":{"minecraft:entity_type":"minecraft:player"}}}]}]}),
    );
    let mut params = LootContextParameters {
        attacking_entity_state: Some(EntityLootState {
            entity_type: Some(&EntityType::ZOMBIE),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(generate_dynamic_loot_with_context(&table, 0, &params).is_empty());
    params.last_damage_player_state = Some(EntityLootState {
        entity_type: Some(&EntityType::PLAYER),
        ..Default::default()
    });
    assert_eq!(
        count(
            &generate_dynamic_loot_with_context(&table, 0, &params),
            &Item::APPLE
        ),
        1
    );
    // LootItemEntityPropertyCondition.test allows an absent predicate even with no entity.
    for (predicate, expected) in [
        (None, 1),
        (Some(json!({"minecraft:entity_type":"minecraft:player"})), 0),
    ] {
        let mut condition =
            json!({"type":"minecraft:entity_properties","entity":"direct_attacker"});
        if let Some(predicate) = predicate {
            condition["predicate"] = predicate;
        }
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","condition":condition}]}]}),
        );
        assert_eq!(
            count(
                &generate_dynamic_loot_with_context(&table, 0, &params),
                &Item::APPLE
            ),
            expected
        );
    }
}
#[test]
fn killed_by_player_requires_last_damage_player() {
    // LootItemKilledByPlayerCondition.test checks the presence of LAST_DAMAGE_PLAYER.
    let table = parse(
        &json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:killed_by_player"},"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]}),
    );
    let mut params = LootContextParameters {
        killed_by_player: Some(true),
        ..Default::default()
    };
    assert!(generate_dynamic_loot_with_context(&table, 0, &params).is_empty());
    params.last_damage_player_state = Some(EntityLootState {
        entity_type: Some(&EntityType::PLAYER),
        ..Default::default()
    });
    assert_eq!(
        count(
            &generate_dynamic_loot_with_context(&table, 0, &params),
            &Item::APPLE
        ),
        1
    );
}
#[test]
fn entity_predicates_follow_vehicle_passenger_back_references() {
    // VehiclePredicate.matches and PassengerPredicate.matches can return to THIS_ENTITY.
    let entity = EntityLootState {
        entity_id: Some(1),
        entity_type: Some(&EntityType::ZOMBIE),
        vehicle: Some(Box::new(EntityLootState {
            entity_id: Some(2),
            entity_type: Some(&EntityType::CHICKEN),
            passengers: vec![EntityLootState {
                entity_id: Some(1),
                entity_type: Some(&EntityType::ZOMBIE),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let predicate = json!({"minecraft:vehicle":{"minecraft:passenger":{"minecraft:entity_type":"minecraft:zombie","minecraft:vehicle":{"minecraft:entity_type":"minecraft:chicken"}}}});
    assert_eq!(entity_predicate(&predicate, &entity), Some(true));
}
#[test]
fn generated_sheep_loot_matches_colour_and_sheared_state() {
    // entities/sheep.json: each wool table requires exact colour and sheared=false.
    for color in ["white", "cyan", "black"] {
        let sheep = EntityLootState {
            entity_type: Some(&EntityType::SHEEP),
            components: [("minecraft:sheep/color".to_owned(), json!(color))].into(),
            flags: [("is_on_fire".to_owned(), false)].into(),
            sheared: Some(false),
            ..Default::default()
        };
        let mut params = entity_context(sheep);
        for seed in 0..64 {
            let items = drops("entities/sheep", seed, &params);
            let wool = items
                .iter()
                .filter(|stack| stack.item.registry_key.ends_with("_wool"))
                .collect::<Vec<_>>();
            assert_eq!(wool.len(), 1);
            assert_eq!(wool[0].item.registry_key, format!("{color}_wool"));
            assert_eq!(wool[0].item_count, 1);
        }
        params.this_entity_state.as_mut().expect("sheep").sheared = Some(true);
        for seed in 0..16 {
            assert!(
                drops("entities/sheep", seed, &params)
                    .iter()
                    .all(|stack| !stack.item.registry_key.ends_with("_wool"))
            );
        }
    }
}
#[test]
fn generated_zombie_disc_requires_player_baby_and_chicken_vehicle() {
    // entities/zombie.json: the last pool is all_of(killed_by_player, baby + chicken).
    let zombie = EntityLootState {
        entity_type: Some(&EntityType::ZOMBIE),
        flags: [
            ("is_baby".to_owned(), false),
            ("is_on_fire".to_owned(), false),
        ]
        .into(),
        ..Default::default()
    };
    let mut params = entity_context(zombie);
    let mut flesh_seen = false;
    for seed in 0..128 {
        let items = drops("entities/zombie", seed, &params);
        assert_eq!(count(&items, &Item::MUSIC_DISC_LAVA_CHICKEN), 0);
        assert!(count(&items, &Item::ROTTEN_FLESH) <= 2);
        flesh_seen |= count(&items, &Item::ROTTEN_FLESH) > 0;
    }
    assert!(flesh_seen);
    let zombie = params.this_entity_state.as_mut().expect("zombie");
    zombie.flags.insert("is_baby".to_owned(), true);
    zombie.vehicle = Some(Box::new(EntityLootState {
        entity_type: Some(&EntityType::CHICKEN),
        ..Default::default()
    }));
    for seed in 0..16 {
        assert_eq!(
            count(
                &drops("entities/zombie", seed, &params),
                &Item::MUSIC_DISC_LAVA_CHICKEN
            ),
            1
        );
    }
    params.killed_by_player = Some(false);
    let player = params.last_damage_player_state.take();
    assert_eq!(
        count(
            &drops("entities/zombie", 0, &params),
            &Item::MUSIC_DISC_LAVA_CHICKEN
        ),
        0
    );
    params.killed_by_player = Some(true);
    params.last_damage_player_state = player;
    params.this_entity_state.as_mut().expect("zombie").vehicle = None;
    assert_eq!(
        count(
            &drops("entities/zombie", 0, &params),
            &Item::MUSIC_DISC_LAVA_CHICKEN
        ),
        0
    );
}
#[test]
fn generated_slab_count_modifier_only_runs_for_double_slabs() {
    // blocks/oak_slab.json supplies count=2 only for type=double.
    for (kind, expected) in [("bottom", 1), ("top", 1), ("double", 2)] {
        let params = block_context(&Block::OAK_SLAB, &[("type", kind)]);
        assert_eq!(
            count(&drops("blocks/oak_slab", 0, &params), &Item::OAK_SLAB),
            expected
        );
    }
}
#[test]
fn generated_wheat_uses_first_matching_alternative_and_mature_seed_pool() {
    // blocks/wheat.json: age=7 selects wheat, otherwise seeds; mature seeds start at 1.
    for seed in 0..64 {
        let young = drops(
            "blocks/wheat",
            seed,
            &block_context(&Block::WHEAT, &[("age", "3")]),
        );
        assert_eq!(count(&young, &Item::WHEAT), 0);
        assert_eq!(count(&young, &Item::WHEAT_SEEDS), 1);
        let mature = drops(
            "blocks/wheat",
            seed,
            &block_context(&Block::WHEAT, &[("age", "7")]),
        );
        assert_eq!(count(&mature, &Item::WHEAT), 1);
        assert!((1..=4).contains(&count(&mature, &Item::WHEAT_SEEDS)));
    }
    // Block.getDrops supplies an empty TOOL when harvested by hand.
    let mut by_hand = block_context(&Block::WHEAT, &[("age", "7")]);
    by_hand.tool = None;
    assert!(
        (0..32)
            .any(|seed| { count(&drops("blocks/wheat", seed, &by_hand), &Item::WHEAT_SEEDS) > 1 })
    );
}
#[test]
fn generated_composter_extra_pool_requires_level_eight() {
    for (level, expected) in [("0", 0), ("7", 0), ("8", 1)] {
        let items = drops(
            "blocks/composter",
            0,
            &block_context(&Block::COMPOSTER, &[("level", level)]),
        );
        assert_eq!(count(&items, &Item::COMPOSTER), 1);
        assert_eq!(count(&items, &Item::BONE_MEAL), expected);
    }
}
pub(super) fn parse(value: &Value) -> DynamicLootTable {
    crate::data::datapack::loot_table_loader::parse_loot_table(&value.to_string())
        .expect("valid loot JSON")
}
#[test]
fn unsupported_predicates_fail_closed_even_when_inverted() {
    for condition in [
        json!({"type":"minecraft:entity_scores"}),
        json!({"type":"minecraft:inverted","term":{"type":"minecraft:damage_source_properties"}}),
        json!({"type":"minecraft:inverted","term":{"type":"minecraft:all_of","terms":[{"type":"minecraft:random_chance","chance":0},{"type":"minecraft:entity_scores"}]}}),
        json!({"type":"minecraft:inverted","term":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:components":{"custom:unknown":true}}}}),
        json!({"type":"minecraft:inverted","term":{"type":"minecraft:random_chance","chance":{"type":"custom:missing_number"}}}),
        json!({"type":"minecraft:inverted","term":{"type":"minecraft:random_chance_with_enchanted_bonus","unenchanted_chance":0,"enchanted_chance":{"type":"custom:missing_number"}}}),
        json!({"type":"minecraft:entity_properties","entity":"custom:unsupported_target"}),
        json!({"type":"minecraft:inverted","term":{"type":"minecraft:entity_properties","entity":"custom:unsupported_target","predicate":{"minecraft:entity_type":"minecraft:player"}}}),
    ] {
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","condition":condition}]}]}),
        );
        assert!(generate_dynamic_loot(&table, 0).is_empty(), "{condition}");
    }
}
#[test]
fn alternatives_stop_at_empty_and_sequence_stops_without_undoing_prior_expansion() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:alternatives","children":[{"type":"minecraft:empty"},{"type":"minecraft:item","name":"minecraft:diamond"}]}]}]}),
    );
    for seed in 0..16 {
        assert!(generate_dynamic_loot(&table, seed).is_empty());
    }
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:sequence","children":[{"type":"minecraft:item","name":"minecraft:apple"},{"type":"minecraft:item","name":"minecraft:diamond","condition":{"type":"minecraft:random_chance","chance":0}},{"type":"minecraft:item","name":"minecraft:emerald"}]}]}]}),
    );
    let items = generate_dynamic_loot(&table, 0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item, &Item::APPLE);
}
#[test]
fn nested_tables_preserve_rolls_and_modifier_order() {
    let table = parse(
        &json!({"modifier":{"type":"minecraft:set_count","count":3,"add":true},"pools":[{"rolls":1,"modifier":{"type":"minecraft:set_count","count":2,"add":true},"entries":[{"type":"minecraft:loot_table","modifier":{"type":"minecraft:set_count","count":1,"add":true},"value":{"pools":[{"rolls":2,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:set_count","count":4}}]}]}}]}]}),
    );
    let items = generate_dynamic_loot(&table, 0);
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|stack| stack.item_count == 10));
}
#[test]
fn conditional_add_and_binomial_counts_do_not_become_uniform_counts() {
    let functions = json!([
        {"type":"minecraft:set_count","count":{"type":"minecraft:binomial","n":5,"p":0.0}},
        {"type":"minecraft:set_count","count":2,"add":true},
        {"type":"minecraft:set_count","count":4,"add":true,"condition":{"type":"minecraft:random_chance","chance":0}}
    ]);
    // SequenceFunction.run preserves the same modifier order as its inline list codec.
    for modifier in [
        functions.clone(),
        json!({"type":"minecraft:sequence","functions":functions}),
    ] {
        let table = parse(
            &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":modifier}]}]}),
        );
        for seed in 0..32 {
            assert_eq!(generate_dynamic_loot(&table, seed)[0].item_count, 2);
        }
    }
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:sequence","condition":{"type":"minecraft:random_chance","chance":0},"functions":{"type":"minecraft:set_count","count":9}}}]}]}),
    );
    assert_eq!(generate_dynamic_loot(&table, 0)[0].item_count, 1);
}
#[test]
fn looting_uses_attacker_instead_of_tool() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":[{"type":"minecraft:set_count","count":0},{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":1}]}]}]}),
    );
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&pumpkin_data::Enchantment::LOOTING, 3);
    let mut params = LootContextParameters {
        tool: Some(sword.clone()),
        ..Default::default()
    };
    assert!(generate_dynamic_loot_with_context(&table, 0, &params).is_empty());
    params.attacking_entity_state = Some(EntityLootState {
        is_living: Some(true),
        equipment: [("mainhand".to_owned(), sword)].into(),
        ..Default::default()
    });
    assert_eq!(
        generate_dynamic_loot_with_context(&table, 0, &params)[0].item_count,
        3
    );
}
#[test]
fn conditional_smelt_and_components_survive_stack_splitting() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:beef","modifier":[{"type":"minecraft:set_count","count":2},{"type":"minecraft:furnace_smelt","condition":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:flags":{"is_on_fire":true}}}},{"type":"minecraft:set_count","count":130},{"type":"minecraft:set_components","components":{"minecraft:custom_name":{"text":"Dinner"}}}]}]}]}),
    );
    let mut params = entity_context(EntityLootState {
        flags: [("is_on_fire".to_owned(), true)].into(),
        ..Default::default()
    });
    let items = generate_dynamic_loot_with_context(&table, 0, &params);
    assert_eq!(items[0].item, &Item::COOKED_BEEF);
    assert_eq!(items.len(), 3);
    assert_eq!(count(&items, &Item::COOKED_BEEF), 130);
    assert!(
        items
            .iter()
            .all(|stack| stack.has_data_component(DataComponent::CustomName))
    );
    params
        .this_entity_state
        .as_mut()
        .expect("entity")
        .flags
        .insert("is_on_fire".to_owned(), false);
    assert_eq!(
        generate_dynamic_loot_with_context(&table, 0, &params)[0].item,
        &Item::BEEF
    );
}
#[test]
fn copy_components_obeys_include_and_exclude() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:copy_components","source":"tool","include":["minecraft:custom_name","minecraft:damage"],"exclude":["minecraft:damage"]}}]}]}),
    );
    let mut tool = ItemStack::new(1, &Item::DIAMOND_SWORD);
    tool.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
        name: pumpkin_util::text::TextComponent::text("Named sword"),
    });
    tool.set_data_component(pumpkin_data::data_component_impl::DamageImpl { damage: 12 });
    let params = LootContextParameters {
        tool: Some(tool),
        ..Default::default()
    };
    let items = generate_dynamic_loot_with_context(&table, 0, &params);
    assert_eq!(items.len(), 1);
    assert!(items[0].has_data_component(DataComponent::CustomName));
    assert!(!items[0].has_data_component(DataComponent::Damage));
    assert_eq!(items[0].get_max_stack_size(), 64);
}
#[test]
fn explosion_decay_rolls_per_item_before_limit_count() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":[{"type":"minecraft:set_count","count":8},{"type":"minecraft:explosion_decay"},{"type":"minecraft:limit_count","limit":{"max":6}}]}]}]}),
    );
    let mut params = LootContextParameters::default();
    assert_eq!(
        generate_dynamic_loot_with_context(&table, 0, &params)[0].item_count,
        6
    );
    params.explosion_radius = Some(f32::INFINITY);
    assert!(generate_dynamic_loot_with_context(&table, 0, &params).is_empty());
    params.explosion_radius = Some(2.0);
    let counts: Vec<_> = (0..32)
        .map(|seed| {
            count(
                &generate_dynamic_loot_with_context(&table, seed, &params),
                &Item::APPLE,
            )
        })
        .collect();
    assert!(counts.iter().all(|count| *count <= 6));
    assert!(counts.iter().any(|count| (1..6).contains(count)));
}
#[test]
fn fortune_bonus_does_not_use_looting_or_attacker_equipment() {
    let table = parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:uniform_bonus_count","parameters":{"bonusMultiplier":1}}}]}]}),
    );
    let mut tool = ItemStack::new(1, &Item::DIAMOND_PICKAXE);
    tool.add_enchantment(&pumpkin_data::Enchantment::LOOTING, 3);
    let mut params = LootContextParameters {
        tool: Some(tool),
        ..Default::default()
    };
    for seed in 0..16 {
        assert_eq!(
            generate_dynamic_loot_with_context(&table, seed, &params)[0].item_count,
            1
        );
    }
    params
        .tool
        .as_mut()
        .expect("tool")
        .add_enchantment(&pumpkin_data::Enchantment::FORTUNE, 3);
    assert!((0..16).any(|seed| {
        generate_dynamic_loot_with_context(&table, seed, &params)[0].item_count > 1
    }));
    params.attacking_entity_state = Some(EntityLootState {
        is_living: Some(true),
        equipment: [("mainhand".to_owned(), params.tool.take().expect("tool"))].into(),
        ..Default::default()
    });
    assert_eq!(
        generate_dynamic_loot_with_context(&table, 0, &params)[0].item_count,
        1
    );
}
