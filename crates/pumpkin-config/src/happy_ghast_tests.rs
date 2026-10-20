use crate::PumpkinConfig;

#[test]
fn feat_happy_ghast_s1_config_defaults_and_round_trip() {
    for text in ["", "[happy_ghast]"] {
        let config: PumpkinConfig = toml::from_str(text).unwrap();
        assert!(
            config.advanced.happy_ghast.enabled,
            "missing config must enable flight"
        );
    }
    let disabled: PumpkinConfig = toml::from_str("[happy_ghast]\nenabled = false").unwrap();
    assert!(!disabled.advanced.happy_ghast.enabled);
    for original in [PumpkinConfig::default(), disabled] {
        let text = toml::to_string(&original).unwrap();
        assert!(text.contains("[happy_ghast]"));
        let restored: PumpkinConfig = toml::from_str(&text).unwrap();
        assert_eq!(
            restored.advanced.happy_ghast.enabled,
            original.advanced.happy_ghast.enabled
        );
    }
}
