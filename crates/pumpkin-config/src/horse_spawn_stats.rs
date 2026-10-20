//! Fresh ordinary-horse attribute initialization.

use serde::{Deserialize, Serialize};

/// Emergency switch for vanilla fresh horse statistics.
#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct HorseSpawnStatsConfig {
    /// Initialize health, speed and jump strength during fresh finalization.
    /// Disabling this preserves generated defaults for new births only; saved
    /// horses keep their attributes and health under either setting.
    pub enabled: bool,
}

impl Default for HorseSpawnStatsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LoadConfiguration, PumpkinConfig};

    #[test]
    fn feat_horses_s1_config_old_toml_and_disabled_birth() {
        for (text, enabled) in [
            ("tps = 10.0", true),
            ("[horse_spawn_stats]", true),
            ("[horse_spawn_stats]\nenabled = false", false),
            ("[horse_spawn_stats]\nenabled = true", true),
        ] {
            let (config, changed) =
                PumpkinConfig::merge_with_default_toml(toml::from_str(text).unwrap());
            assert!(changed);
            assert_eq!(config.advanced.horse_spawn_stats.enabled, enabled);
            let saved = toml::Value::try_from(config).unwrap();
            let (reloaded, changed) = PumpkinConfig::merge_with_default_toml(saved);
            assert!(!changed);
            assert_eq!(reloaded.advanced.horse_spawn_stats.enabled, enabled);
        }
    }

    #[test]
    fn feat_horses_s1_real_config_load_adds_section_and_keeps_switch() {
        let directory = std::env::temp_dir().join(format!("horse-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        for (text, enabled) in [
            ("tps = 10.0", true),
            ("[horse_spawn_stats]", true),
            ("[horse_spawn_stats]\nenabled = false", false),
            ("[horse_spawn_stats]\nenabled = true", true),
        ] {
            std::fs::write(directory.join("pumpkin.toml"), text).unwrap();
            let config = PumpkinConfig::load(&directory);
            assert_eq!(config.advanced.horse_spawn_stats.enabled, enabled);
            let reloaded = PumpkinConfig::load(&directory);
            assert_eq!(reloaded.advanced.horse_spawn_stats.enabled, enabled);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
