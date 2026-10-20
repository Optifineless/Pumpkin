use serde::{Deserialize, Serialize};

/// Built-in game-event forwarding configuration.
#[derive(Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SculkGameEventsConfig {
    /// Forward game events with their source context to built-in consumers.
    pub enabled: bool,
}

impl Default for SculkGameEventsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use crate::AdvancedConfiguration;

    #[test]
    fn feat_warden_sculk_s1_config_defaults_and_off_preserves_plugins() {
        for old in ["", "[sculk_game_events]"] {
            let config: AdvancedConfiguration = toml::from_str(old).unwrap();
            assert!(config.sculk_game_events.enabled);
        }
        for enabled in [true, false] {
            let mut config = AdvancedConfiguration::default();
            config.sculk_game_events.enabled = enabled;
            let text = toml::to_string(&config).unwrap();
            assert!(text.contains(&format!("[sculk_game_events]\nenabled = {enabled}")));
            let loaded: AdvancedConfiguration = toml::from_str(&text).unwrap();
            assert_eq!(loaded.sculk_game_events.enabled, enabled);
        }
        let generated = toml::to_string(&AdvancedConfiguration::default()).unwrap();
        assert!(generated.contains("[sculk_game_events]\nenabled = true"));
        let mut old: toml::Table = toml::from_str(&generated).unwrap();
        old.remove("sculk_game_events");
        let old: AdvancedConfiguration = toml::from_str(&toml::to_string(&old).unwrap()).unwrap();
        assert!(old.sculk_game_events.enabled);
    }
}
