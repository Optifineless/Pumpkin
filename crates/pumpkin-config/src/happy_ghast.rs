use serde::{Deserialize, Serialize};

/// Adult Happy Ghast autonomous flight configuration.
#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct HappyGhastConfig {
    /// Enable careful adult flight and roaming homes; false restores legacy adult AI.
    pub enabled: bool,
}

impl Default for HappyGhastConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
