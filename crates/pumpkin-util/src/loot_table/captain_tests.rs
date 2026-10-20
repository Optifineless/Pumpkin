use super::*;

#[test]
fn b1020b_g01_generated_pillager_parser_supports_raider_and_amplifier() {
    // Read the retained generated table without a circular pumpkin-util -> pumpkin-data dependency.
    let generated = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pumpkin-data/src/generated/loot_table.rs"
    ));
    let (_, table) = generated
        .split_once("pub static ENTITIES_PILLAGER:")
        .unwrap();
    let (_, table) = table.split_once("LootTable::new(").unwrap();
    let literal = table
        .trim_start()
        .lines()
        .next()
        .unwrap()
        .trim()
        .trim_end_matches(',');
    let json = serde_json::from_str::<String>(literal).unwrap();
    let mut unsupported = BTreeSet::new();
    let table = parse_loot_table(&json, &mut unsupported).unwrap();
    assert!(
        unsupported.is_empty(),
        "generated pillager diagnostics: {unsupported:?}"
    );
    assert_eq!(
        table.random_sequence.as_deref(),
        Some("minecraft:entities/pillager")
    );
    assert!(matches!(
        table.pools[0].condition,
        LootCondition::EntityProperties { .. }
    ));
    assert_eq!(
        table.pools[0].entries[0].functions[1].kind,
        "set_ominous_bottle_amplifier"
    );
}
