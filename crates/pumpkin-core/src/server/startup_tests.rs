use std::{sync::RwLock, time::Duration};

use pumpkin_config::{AdvancedConfiguration, BasicConfiguration, TelemetryConfig};

use super::Server;
use crate::data::VanillaData;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_bedrock_skips_authentication_initialization() {
    let directory = tempfile::tempdir().unwrap();
    let basic = BasicConfiguration {
        default_level_name: directory.path().to_string_lossy().into_owned(),
        allow_nether: false,
        allow_end: false,
        ..BasicConfiguration::default()
    };
    let mut advanced = AdvancedConfiguration::default();
    advanced.networking.bedrock.enabled = false;
    // An unused endpoint must not initialize keys, even with online auth left enabled.
    advanced.networking.bedrock.authentication.url = Some("invalid://unused".to_owned());
    let data = VanillaData {
        banned_ip_list: RwLock::default(),
        banned_player_list: RwLock::default(),
        operator_config: RwLock::default(),
        user_cache: RwLock::default(),
        whitelist_config: RwLock::default(),
    };
    let server = Server::new(
        basic,
        advanced,
        TelemetryConfig::default(),
        data,
        Vec::new(),
    )
    .await
    .unwrap();
    super::fixture_lifecycle::track_server(&server);
    server.tasks.close();
    tokio::time::timeout(Duration::from_secs(30), server.tasks.wait())
        .await
        .unwrap();

    assert!(server.bedrock_oidc_keys.get().is_none());
    super::fixture_lifecycle::finish().await;
}
