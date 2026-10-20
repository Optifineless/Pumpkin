use serde::{Deserialize, Serialize};

/// Controls repeated riding attempts for ordinary horses; saved horse data is always retained.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct HorseRidingTamingConfig {
    /// Enables ride-attempt temper, taming and bucking. Disable as an emergency gameplay switch.
    pub enabled: bool,
}

impl Default for HorseRidingTamingConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LoadConfiguration, PumpkinConfig};

    #[test]
    fn feat_horses_s2_old_toml_defaults_on_and_explicit_off_survives_merge()
    -> Result<(), toml::de::Error> {
        let (old, changed) = PumpkinConfig::merge_with_default_toml(toml::from_str(
            "[advancement]\nenabled = true",
        )?);
        assert!(changed);
        assert!(old.advanced.horse_riding_taming.enabled);
        let (off, _) = PumpkinConfig::merge_with_default_toml(toml::from_str(
            "[horse_riding_taming]\nenabled = false",
        )?);
        assert!(!off.advanced.horse_riding_taming.enabled);
        Ok(())
    }
}
