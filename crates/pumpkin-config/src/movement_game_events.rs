//! Accepted-movement game-event configuration.

use serde::{Deserialize, Serialize};

/// Notifications from accepted entity travel, independent of sculk forwarding.
#[derive(Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct MovementGameEventsConfig {
    /// Account accepted distance and emit STEP, SWIM and species FLAP notifications.
    pub enabled: bool,
}

impl Default for MovementGameEventsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use crate::AdvancedConfiguration;

    #[test]
    fn feat_warden_sculk_s3_configuration_defaults_and_roundtrip() {
        for text in [
            "",
            "[movement_game_events]",
            "[movement_game_events]\nenabled = true",
        ] {
            let config: AdvancedConfiguration = toml::from_str(text).unwrap();
            assert!(config.movement_game_events.enabled);
        }
        for enabled in [false, true] {
            let text = format!("[movement_game_events]\nenabled = {enabled}");
            let config: AdvancedConfiguration = toml::from_str(&text).unwrap();
            assert_eq!(config.movement_game_events.enabled, enabled);
            let saved = toml::to_string(&config).unwrap();
            assert!(saved.contains(&text));
            let restored: AdvancedConfiguration = toml::from_str(&saved).unwrap();
            assert_eq!(restored.movement_game_events.enabled, enabled);
        }
    }
}
