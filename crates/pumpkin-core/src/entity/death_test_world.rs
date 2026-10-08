use std::sync::{Arc, RwLock};

use pumpkin_config::{AdvancedConfiguration, BasicConfiguration};
use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_util::{GameMode, math::vector3::Vector3};
use uuid::Uuid;

use super::{EntityBase, player::Player};
use crate::{
    data::VanillaData,
    net::{ClientPlatform, GameProfile, PlayerConfig, java::JavaClient},
    server::Server,
    world::World,
};

pub struct DeathTestWorld {
    pub(crate) server: Arc<Server>,
    _directory: tempfile::TempDir,
}

#[expect(
    clippy::unwrap_used,
    reason = "Test fixtures require successful construction"
)]
impl DeathTestWorld {
    pub(crate) async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let basic = BasicConfiguration {
            default_level_name: directory.path().to_string_lossy().into_owned(),
            allow_end: false,
            ..BasicConfiguration::default()
        };
        let mut advanced = AdvancedConfiguration::default();
        advanced.networking.bedrock.online_mode = false;
        let data = VanillaData {
            banned_ip_list: RwLock::default(),
            banned_player_list: RwLock::default(),
            operator_config: RwLock::default(),
            user_cache: RwLock::default(),
            whitelist_config: RwLock::default(),
        };
        // Construct real worlds and registries without starting networking or a tick loop.
        let server = Server::new(
            basic,
            advanced,
            pumpkin_config::TelemetryConfig::default(),
            data,
            Vec::new(),
        )
        .await
        .unwrap();
        Self {
            server,
            _directory: directory,
        }
    }

    pub(crate) fn world(&self) -> Arc<World> {
        self.server.get_world_from_dimension(&Dimension::OVERWORLD)
    }

    pub(crate) fn player(&self, name: &str) -> Arc<Player> {
        let world = self.world();
        let profile = GameProfile {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            properties: arc_swap::ArcSwap::from_pointee(Vec::new()),
            profile_actions: None,
        };
        let client = Arc::new(ClientPlatform::Java(JavaClient::without_connection(
            profile.clone(),
        )));
        let player = Arc::new(Player::new(
            client,
            profile,
            PlayerConfig::default(),
            &world,
            GameMode::Survival,
        ));
        player.set_client_loaded(true);
        world.players.rcu(|players| {
            let mut players = (**players).clone();
            players.push(player.clone());
            players
        });
        player
    }

    pub(crate) fn mob(&self, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let world = self.world();
        let mob =
            super::r#type::from_type(kind, Vector3::new(0.0, 64.0, 0.0), &world, Uuid::new_v4());
        world.add_entity_silent(mob.clone());
        mob
    }

    pub(crate) fn keep_inventory(&self, value: bool) {
        self.server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.keep_inventory = value;
            info
        });
    }
}
