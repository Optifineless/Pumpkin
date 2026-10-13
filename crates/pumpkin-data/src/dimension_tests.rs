use crate::dimension::{BedRule, BedRuleOption, Dimension};

fn assert_rule(
    actual: &BedRule,
    can_sleep: BedRuleOption,
    can_set_spawn: BedRuleOption,
    destroy_on_use: bool,
    destroy_on_leave: bool,
    error_message: Option<&str>,
) {
    assert_eq!(actual.can_sleep, can_sleep);
    assert_eq!(actual.can_set_spawn, can_set_spawn);
    assert_eq!(actual.destroy_on_use, destroy_on_use);
    assert_eq!(actual.destroy_on_leave, destroy_on_leave);
    assert_eq!(actual.error_message, error_message);
}

#[test]
fn generated_bed_and_anchor_rules_match_vanilla_dimensions() {
    // Mirrors vanilla 26.3 BedRule defaults and DimensionTypes.bootstrap.
    let no_sleep_message = Some(r#"{"translate":"block.minecraft.bed.no_sleep"}"#);
    let overworld = &Dimension::OVERWORLD;
    assert_rule(
        &overworld.bed_rule,
        BedRuleOption::WhenDark,
        BedRuleOption::Always,
        false,
        false,
        no_sleep_message,
    );
    assert_rule(
        &overworld.straw_bed_rule,
        BedRuleOption::WhenDark,
        BedRuleOption::Never,
        false,
        true,
        no_sleep_message,
    );
    assert!(!overworld.respawn_anchor_works);

    let nether = &Dimension::THE_NETHER;
    assert_rule(
        &nether.bed_rule,
        BedRuleOption::Never,
        BedRuleOption::Never,
        true,
        false,
        None,
    );
    assert_rule(
        &nether.straw_bed_rule,
        BedRuleOption::Never,
        BedRuleOption::Never,
        true,
        false,
        None,
    );
    assert!(nether.respawn_anchor_works);

    let end = &Dimension::THE_END;
    assert_rule(
        &end.bed_rule,
        BedRuleOption::Never,
        BedRuleOption::Never,
        true,
        false,
        None,
    );
    assert_rule(
        &end.straw_bed_rule,
        BedRuleOption::Never,
        BedRuleOption::Never,
        true,
        false,
        None,
    );
    assert!(!end.respawn_anchor_works);
}
