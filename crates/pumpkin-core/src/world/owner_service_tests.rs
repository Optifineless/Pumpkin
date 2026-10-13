#![expect(clippy::unwrap_used, reason = "controlled service regression fixtures")]

use super::*;
use crate::{
    entity::player::Player,
    net::{ClientPlatform, GameProfile, PlayerConfig, java::JavaClient},
};
use crate::{server::combat_test_support, world::spawn_test_support};
use pumpkin_data::{Block, biome::Biome, dimension::Dimension};
use pumpkin_nbt::{compound::NbtCompound, nbt_compress::write_gzip_compound_tag};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2},
};
use pumpkin_world::{
    level::Level,
    tick::TickPriority,
    world_info::{MAXIMUM_SUPPORTED_LEVEL_VERSION, MAXIMUM_SUPPORTED_WORLD_DATA_VERSION},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Barrier, atomic::AtomicUsize},
};

const RESTORED_AGE: i64 = 7341;

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Vanilla PrimaryLevelData.parse reads Time; older fork saves use the same long tag.
        let mut generation = NbtCompound::new();
        generation.put_long("seed", 0);
        let mut data = NbtCompound::new();
        data.put_int("DataVersion", MAXIMUM_SUPPORTED_WORLD_DATA_VERSION);
        data.put_int("version", MAXIMUM_SUPPORTED_LEVEL_VERSION);
        data.put_string("LevelName", "M1 restored age".into());
        data.put_long("Time", RESTORED_AGE);
        data.put_long("DayTime", 5000);
        data.put_compound("WorldGenSettings", generation);
        let mut root = NbtCompound::new();
        root.put_compound("Data", data);
        write_gzip_compound_tag(
            root,
            std::fs::File::create(dir.path().join("level.dat")).unwrap(),
        )
        .unwrap();
        let server = combat_test_support::server(dir.path());
        let level = Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            dir.path().to_path_buf(),
            0,
            Dimension::OVERWORLD,
        );
        let world = Arc::new(World::load(
            level,
            server.level_info.clone(),
            Dimension::OVERWORLD,
            server.block_registry.clone(),
            Arc::downgrade(&server),
        ));
        server.worlds.store(Arc::new(vec![world.clone()]));
        Self {
            server,
            world,
            _dir: dir,
        }
    }

    async fn check(self, body: impl FnOnce(&Self)) {
        let result = catch_unwind(AssertUnwindSafe(|| body(&self)));
        self.server.shutdown().await;
        assert!(self.world.level.cancel_token.is_cancelled());
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }

    fn player(&self) -> Arc<Player> {
        let profile = GameProfile {
            id: uuid::Uuid::new_v4(),
            name: "M1FrozenService".into(),
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
            &self.world,
            GameMode::Survival,
        ));
        player.set_client_loaded(true);
        self.world.players.store(Arc::new(vec![player.clone()]));
        player
    }
}

#[tokio::test]
async fn overlapping_normal_and_frozen_real_adapters_reject_before_effects() {
    let first = Fixture::new();
    let other = Fixture::new();
    first
        .check(|fixture| {
            let world = &fixture.world;
            let entered = Barrier::new(2);
            let release = Barrier::new(2);
            let bodies = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                let task = scope.spawn(|| {
                    world.with_normal_service(|| {
                        entered.wait();
                        release.wait();
                    })
                });
                entered.wait();
                let held_epoch = world.owner_service.epoch.load(Ordering::Acquire);
                let frozen = world.with_frozen_service(|| {
                    bodies.fetch_add(1, Ordering::Relaxed);
                    world.tick_environment();
                });
                let normal = world.with_normal_service(|| {
                    bodies.fetch_add(1, Ordering::Relaxed);
                    world.tick_environment();
                });
                let independent = other.world.with_normal_service(|| 17);
                let after_rejection = world.owner_service.epoch.load(Ordering::Acquire);
                let observed_age = world.get_world_age();
                // Always release before assertions, including under the negative control.
                release.wait();
                let finished = task.join();
                assert_eq!(frozen, Err(ServiceEntryError::ServiceAlreadyActive));
                assert_eq!(normal, Err(ServiceEntryError::ServiceAlreadyActive));
                assert_eq!(bodies.load(Ordering::Relaxed), 0);
                assert_eq!(observed_age, RESTORED_AGE);
                assert_eq!(after_rejection, held_epoch);
                assert_eq!(independent, Ok(17));
                assert_eq!(finished.unwrap(), Ok(()));
            });
            assert_eq!(world.with_frozen_service(|| 23), Ok(23));
            assert_eq!(world.owner_service.epoch.load(Ordering::Acquire), 2);
        })
        .await;
    other.check(|_| {}).await;
}

#[tokio::test]
async fn startup_and_restored_clock_readiness_use_world_load_and_the_real_adapter() {
    Fixture::new()
        .check(|fixture| {
            let loaded = &fixture.world;
            assert_eq!(loaded.owner_service.restored_age, RESTORED_AGE);
            assert_eq!(loaded.get_world_age(), RESTORED_AGE);
            assert_eq!(loaded.level.get_game_time(), RESTORED_AGE);
            assert_eq!(
                loaded.with_normal_service(|| loaded.get_world_age()),
                Ok(RESTORED_AGE)
            );
            let mut uninitialized = World::load(
                loaded.level.clone(),
                loaded.level_info.clone(),
                loaded.dimension.clone(),
                loaded.block_registry.clone(),
                Arc::downgrade(&fixture.server),
            );
            // Explicit state only on this fixture; no bypass or production pause hook.
            uninitialized.owner_service = OwnerServiceState::new();
            let entered = AtomicUsize::new(0);
            let rejected = uninitialized.with_normal_service(|| {
                entered.fetch_add(1, Ordering::Relaxed);
            });
            assert_eq!(rejected, Err(ServiceEntryError::NotInitialized));
            assert_eq!(entered.load(Ordering::Relaxed), 0);
            assert_eq!(uninitialized.owner_service.epoch.load(Ordering::Acquire), 0);
            assert!(uninitialized.owner_service.clock_ready(RESTORED_AGE));
            assert!(!uninitialized.owner_service.clock_ready(RESTORED_AGE));
            assert!(!uninitialized.owner_service.public_ingress(RESTORED_AGE + 1));
            assert_eq!(
                uninitialized.with_frozen_service(|| ()),
                Err(ServiceEntryError::NotInitialized)
            );
            assert!(uninitialized.owner_service.public_ingress(RESTORED_AGE));
            assert!(!uninitialized.owner_service.public_ingress(RESTORED_AGE));
            assert_eq!(
                uninitialized.with_normal_service(|| uninitialized.get_world_age()),
                Ok(RESTORED_AGE)
            );
        })
        .await;
}

#[tokio::test]
async fn mismatched_level_clock_rejects_one_shot_initialization_before_service_effects() {
    Fixture::new()
        .check(|fixture| {
            let loaded = &fixture.world;
            let mut mismatched = World::load(
                loaded.level.clone(),
                loaded.level_info.clone(),
                loaded.dimension.clone(),
                loaded.block_registry.clone(),
                Arc::downgrade(&fixture.server),
            );
            // Exercise the actual initializer against M0's published clock, without a load hook.
            mismatched.owner_service = OwnerServiceState::new();
            mismatched.level.set_game_time(RESTORED_AGE + 1);
            assert_eq!(mismatched.get_world_age(), RESTORED_AGE);
            mismatched.initialize_owner_service(RESTORED_AGE);
            let entered = AtomicUsize::new(0);
            assert_eq!(
                mismatched.with_normal_service(|| entered.fetch_add(1, Ordering::Relaxed)),
                Err(ServiceEntryError::NotInitialized)
            );
            assert_eq!(
                mismatched.with_frozen_service(|| entered.fetch_add(1, Ordering::Relaxed)),
                Err(ServiceEntryError::NotInitialized)
            );
            // Correcting the clock cannot reopen the rejected one-shot initialization.
            mismatched.level.set_game_time(RESTORED_AGE);
            mismatched.initialize_owner_service(RESTORED_AGE);
            assert_eq!(
                mismatched.with_normal_service(|| entered.fetch_add(1, Ordering::Relaxed)),
                Err(ServiceEntryError::NotInitialized)
            );
            assert_eq!(entered.load(Ordering::Relaxed), 0);
            assert_eq!(mismatched.owner_service.epoch.load(Ordering::Acquire), 0);
        })
        .await;
}

#[tokio::test]
async fn receipt_unwind_releases_its_claim_and_epochs_remain_monotonic() {
    Fixture::new()
        .check(|fixture| {
            let world = &fixture.world;
            let unwound = catch_unwind(AssertUnwindSafe(|| {
                let _ = world.with_normal_service(|| std::panic::resume_unwind(Box::new("probe")));
            }));
            assert!(unwound.is_err());
            assert_eq!(world.owner_service.active.load(Ordering::Acquire), IDLE);
            assert_eq!(world.owner_service.epoch.load(Ordering::Acquire), 1);
            assert_eq!(world.with_frozen_service(|| 41), Ok(41));
            assert_eq!(world.owner_service.epoch.load(Ordering::Acquire), 2);
            world.tick(&fixture.server);
            assert_eq!(world.owner_service.epoch.load(Ordering::Acquire), 3);
        })
        .await;
}

#[tokio::test]
async fn frozen_server_entry_preserves_age_and_pending_scheduled_work() {
    Fixture::new()
        .check(|fixture| {
            let world = &fixture.world;
            let chunk = spawn_test_support::publish(
                world,
                spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
            );
            let position = BlockPos::new(8, 63, 8);
            world
                .active_chunks
                .write()
                .unwrap()
                .insert(Vector2::new(0, 0));
            world
                .level
                .schedule_block_tick(&Block::STONE, position, 1, TickPriority::Normal);
            assert!(
                world
                    .level
                    .is_block_tick_scheduled(&position, &Block::STONE)
            );
            let pending = chunk.block_ticks.to_vec();
            let age = world.get_world_age();
            let player = fixture.player();
            fixture
                .server
                .tick_rate_manager
                .set_frozen(&fixture.server, true);
            assert_eq!(player.tick_counter.load(Ordering::Relaxed), 0);
            fixture.server.tick_players_and_network();
            assert_eq!(player.tick_counter.load(Ordering::Relaxed), 1);
            assert_eq!(world.get_world_age(), age);
            assert!(
                world
                    .level
                    .is_block_tick_scheduled(&position, &Block::STONE)
            );
            let after = chunk.block_ticks.to_vec();
            assert_eq!(pending.len(), 1);
            assert_eq!(after.len(), pending.len());
            assert_eq!(after[0].position, pending[0].position);
            assert_eq!(after[0].delay, pending[0].delay);
            assert_eq!(world.owner_service.epoch.load(Ordering::Acquire), 1);
            assert_eq!(world.owner_service.active.load(Ordering::Acquire), IDLE);
        })
        .await;
}

#[tokio::test]
async fn instrumented_guard_is_rejected_at_the_body_boundary_and_releases_receipt() {
    Fixture::new()
        .check(|fixture| {
            let world = &fixture.world;
            let called = AtomicUsize::new(0);
            let failed = catch_unwind(AssertUnwindSafe(|| {
                let _guard = world.weather.lock().unwrap();
                let _ = world.with_normal_service(|| called.fetch_add(1, Ordering::Relaxed));
            }));
            assert!(failed.is_err());
            assert_eq!(called.load(Ordering::Relaxed), 0);
            assert_eq!(world.with_normal_service(|| 31), Ok(31));
        })
        .await;
}
