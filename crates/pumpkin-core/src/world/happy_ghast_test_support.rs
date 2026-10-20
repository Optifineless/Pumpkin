use super::{
    World,
    spawn_test_support::{proto, publish},
};
use crate::{
    command::CommandSender,
    data::VanillaData,
    entity::{EntityBase, passive::happy_ghast::HappyGhastEntity},
    item::{ItemBehaviour, items::spawn_egg::SpawnEggItem},
    net::java::combat_test_support::TestPlayer,
    server::Server,
};
use pumpkin_config::{BasicConfiguration, LoadConfiguration, PumpkinConfig};
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, dimension::Dimension, item::Item, item_stack::ItemStack,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::{
    path::Path,
    sync::{Arc, RwLock},
    time::Duration,
};

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: TestPlayer,
    pub directory: tempfile::TempDir,
}

impl Fixture {
    pub async fn new(enabled: bool) -> Self {
        Self::open(tempfile::tempdir().unwrap(), enabled).await
    }
    async fn server(path: &Path, enabled: bool) -> Arc<Server> {
        let basic = BasicConfiguration {
            default_level_name: path.to_string_lossy().into_owned(),
            allow_end: false,
            allow_nether: false,
            ..BasicConfiguration::default()
        };
        std::fs::write(
            path.join("pumpkin.toml"),
            format!("[happy_ghast]\nenabled = {enabled}"),
        )
        .unwrap();
        let mut advanced = PumpkinConfig::load(path).advanced;
        advanced.networking.bedrock.online_mode = false;
        let data = VanillaData {
            banned_ip_list: RwLock::default(),
            banned_player_list: RwLock::default(),
            operator_config: RwLock::default(),
            user_cache: RwLock::default(),
            whitelist_config: RwLock::default(),
        };
        tokio::time::timeout(
            Duration::from_secs(30),
            Server::new(
                basic,
                advanced,
                pumpkin_config::TelemetryConfig::default(),
                data,
                Vec::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap()
    }
    async fn open(directory: tempfile::TempDir, enabled: bool) -> Self {
        let server = Self::server(directory.path(), enabled).await;
        let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.spawn_mobs = false;
            info.game_rules.random_tick_speed = 0;
            info
        });
        for x in -2..=2 {
            for z in -2..=2 {
                let mut chunk = proto(&Biome::PLAINS, &Block::STONE);
                chunk.x = x;
                chunk.z = z;
                publish(&world, chunk);
            }
        }
        let watched: Vec<_> = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| Vector2::new(x, z)))
            .collect();
        tokio::time::timeout(
            Duration::from_secs(10),
            world.level.mark_chunks_as_newly_watched(&watched),
        )
        .await
        .unwrap();
        let player = TestPlayer::new(&world);
        player.client().set_player(player.player.clone());
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(32.0, 74.0, 32.0));
        Self {
            server,
            world,
            player,
            directory,
        }
    }
    pub fn summon(&self, coordinates: &str, nbt: &str) -> Arc<dyn EntityBase> {
        let before: Vec<_> = self
            .world
            .entities
            .load()
            .iter()
            .map(|e| e.get_entity().entity_uuid)
            .collect();
        let mut source = CommandSender::Console
            .into_source(&self.server)
            .with_world(self.world.clone());
        source.silent = true;
        assert_eq!(
            self.server
                .command_dispatcher
                .load()
                .execute_input(
                    &format!("summon minecraft:happy_ghast {coordinates} {nbt}"),
                    &source
                )
                .unwrap(),
            1
        );
        self.world
            .entities
            .load()
            .iter()
            .find(|e| !before.contains(&e.get_entity().entity_uuid))
            .unwrap()
            .clone()
    }
    pub fn egg(&self, pos: BlockPos) -> Arc<dyn EntityBase> {
        let before: Vec<_> = self
            .world
            .entities
            .load()
            .iter()
            .map(|e| e.get_entity().entity_uuid)
            .collect();
        let mut egg = ItemStack::new(1, &Item::HAPPY_GHAST_SPAWN_EGG);
        SpawnEggItem.use_on_block(
            &mut egg,
            self.player.player.as_ref(),
            pos,
            BlockDirection::Up,
            Vector3::new(0.5, 0.5, 0.5),
            &Block::AIR,
            &self.server,
        );
        assert!(egg.is_empty());
        self.world
            .entities
            .load()
            .iter()
            .find(|e| !before.contains(&e.get_entity().entity_uuid))
            .unwrap()
            .clone()
    }
    pub fn tick(&self, ticks: usize) {
        for _ in 0..ticks {
            self.world.tick(&self.server);
        }
    }
    pub async fn close(self) -> tempfile::TempDir {
        self.player.client().close();
        self.world.players.store(Arc::new(Vec::new()));
        tokio::time::timeout(Duration::from_secs(30), self.server.shutdown())
            .await
            .unwrap();
        self.directory
    }
    pub async fn restart(self, enabled: bool) -> Self {
        Self::open(self.close().await, enabled).await
    }
    pub async fn saved_record(&self, entity: &Arc<dyn EntityBase>) -> NbtCompound {
        tokio::time::timeout(Duration::from_secs(10), self.world.save())
            .await
            .unwrap();
        let chunk = tokio::time::timeout(
            Duration::from_secs(10),
            self.world
                .level
                .get_entity_chunk(entity.get_entity().chunk_pos.load()),
        )
        .await
        .unwrap()
        .unwrap();
        chunk
            .data
            .lock()
            .unwrap()
            .iter()
            .find(|nbt| nbt.get_uuid("UUID") == Some(entity.get_entity().entity_uuid))
            .unwrap()
            .clone()
    }
    pub async fn restore(&self, chunk_pos: Vector2<i32>, uuid: uuid::Uuid) -> Arc<dyn EntityBase> {
        if !self.world.level.is_chunk_watched(&chunk_pos) {
            tokio::time::timeout(
                Duration::from_secs(10),
                self.world.level.mark_chunks_as_newly_watched(&[chunk_pos]),
            )
            .await
            .unwrap();
        }
        let chunk = tokio::time::timeout(
            Duration::from_secs(10),
            self.world.level.get_entity_chunk(chunk_pos),
        )
        .await
        .unwrap()
        .unwrap();
        self.world.make_chunk_entities_live(&chunk, None);
        self.world.get_entity_by_uuid(uuid).unwrap()
    }
}

pub(super) fn ghast(entity: &Arc<dyn EntityBase>) -> &HappyGhastEntity {
    entity.cast_any().downcast_ref().unwrap()
}
pub(super) fn draws(entity: &Arc<dyn EntityBase>, values: &[f64]) {
    *ghast(entity).random_draws.lock().unwrap() = values.iter().copied().collect();
}
pub(super) fn wanted(entity: &Arc<dyn EntityBase>, target: Vector3<f64>) {
    ghast(entity)
        .mob_entity
        .move_control
        .lock()
        .unwrap()
        .set_wanted_position(target.x, target.y, target.z, 1.0);
}
pub(super) fn target(entity: &Arc<dyn EntityBase>) -> Vector3<f64> {
    ghast(entity)
        .mob_entity
        .move_control
        .lock()
        .unwrap()
        .wanted_position()
        .0
}
pub(super) fn nbt(entity: &Arc<dyn EntityBase>) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    nbt
}
pub(super) fn assert_flying(entity: &Arc<dyn EntityBase>, start: Vector3<f64>) {
    let end = entity.get_entity().pos.load();
    let delta = end - start;
    assert!(
        delta.x > 0.01 && delta.y > 0.01 && delta.z > 0.01,
        "adult did not fly in 3D: {delta:?}; age={}, timeout={}, removed={}, velocity={:?}, target={:?}, loaded={}",
        ghast(entity)
            .get_entity()
            .age
            .load(std::sync::atomic::Ordering::Relaxed),
        ghast(entity)
            .server_still_timeout
            .load(std::sync::atomic::Ordering::Relaxed),
        entity.get_entity().is_removed(),
        entity.get_entity().velocity.load(),
        target(entity),
        entity
            .get_entity()
            .world
            .load()
            .level
            .is_chunk_loaded(&entity.get_entity().chunk_pos.load())
    );
    assert!(delta.length() < 30.0 && end.x.is_finite() && end.y.is_finite() && end.z.is_finite());
    let bounds = entity.get_entity().bounding_box.load();
    assert!((f64::midpoint(bounds.min.x, bounds.max.x) - end.x).abs() < 1.0e-9);
    assert_eq!(bounds.min.y, end.y);
    assert!(
        !ghast(entity)
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .is_in_progress()
    );
}
