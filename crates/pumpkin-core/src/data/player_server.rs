use crate::{
    entity::{NBTStorage, player::Player},
    server::Server,
};
use crossbeam::atomic::AtomicCell;
use pumpkin_inventory::screen_handler::ScreenHandler;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_world::data::player_data::{PlayerDataError, PlayerDataStorage};
use std::sync::Arc;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tracing::error;

/// Helper for managing player data in the server context.
///
/// This struct provides server-wide access to the `PlayerDataStorage` and
/// convenience methods for player handling.
pub struct ServerPlayerData {
    storage: Arc<PlayerDataStorage>,
    save_interval: Duration,
    last_save: AtomicCell<Instant>,
    jobs: tokio_util::task::TaskTracker,
    drain_gate: tokio::sync::Mutex<()>,
}

pub struct PlayerStorageSession {
    _gate: tokio::sync::OwnedMutexGuard<()>,
    retired: bool,
}

impl ServerPlayerData {
    /// Creates a new `ServerPlayerData` with specified configuration.
    pub fn new(data_path: impl Into<PathBuf>, save_interval: Duration, enabled: bool) -> Self {
        Self {
            storage: Arc::new(PlayerDataStorage::new(data_path, enabled)),
            save_interval,
            last_save: AtomicCell::new(Instant::now()),
            jobs: tokio_util::task::TaskTracker::new(),
            drain_gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Saves before PlayerList.remove makes the UUID available to another session.
    pub async fn handle_player_leave(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
    ) -> Result<(), PlayerDataError> {
        // PlayerList.remove runs on vanilla's server thread. Fence Pumpkin's entire
        // tick (including collisions/combat) through capture and simulation removal.
        // The connection owner has already joined this player's packet tasks.
        let tick = server.tick_gate.lock().await;
        // PlayerList.remove only retires its own session, never a rejoined UUID.
        if server
            .get_player_by_uuid(player.gameprofile.id)
            .is_some_and(|current| !Arc::ptr_eq(&current, player))
        {
            // PlayerList.remove retires the old entity before checking playersByUUID.
            player.unride_for_respawn();
            player
                .living_entity
                .entity
                .set_removed(crate::entity::RemovalReason::UnloadedWithPlayer);
            return Ok(());
        }
        // PlayerList.remove awards LEAVE_GAME before capturing the final save.
        player.increment_custom_stat(
            crate::entity::player::statistics::CustomStatistic::LeaveGame,
            1,
        );
        player
            .player_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .on_closed(player.as_ref());
        player.on_handled_screen_closed();
        let snapshot = {
            let mut session = player
                .storage_session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(session) = session.as_mut() else {
                return Ok(());
            };
            session.retired = true;
            self.storage.snapshot(&player.gameprofile.id, || {
                let mut nbt = NbtCompound::new();
                player.write_nbt(&mut nbt);
                nbt
            })
        };
        player.remove().await;
        server.remove_player(player);
        drop(tick);
        // The UUID session remains held while disk publication runs outside the tick fence.
        let storage = self.storage.clone();
        let saved = self
            .jobs
            .spawn_blocking(move || storage.save_snapshot(snapshot))
            .await
            .map_err(|error| PlayerDataError::Io(std::io::Error::other(error)))
            .and_then(std::convert::identity);
        // PlayerList.save includes advancements; finish them before releasing the UUID gate.
        let advancements = server
            .advancement_manager
            .save_player(player)
            .await
            .map_err(|error| PlayerDataError::Io(std::io::Error::other(error)));
        saved.and(advancements)
    }

    fn capture_player(&self, player: &Player) {
        let session = player
            .storage_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if session.as_ref().is_some_and(|session| !session.retired) {
            self.storage.snapshot(&player.gameprofile.id, || {
                let mut nbt = NbtCompound::new();
                player.write_nbt(&mut nbt);
                nbt
            });
        }
    }

    pub fn tick(&self, server: &Server) {
        let now = Instant::now();
        if now.duration_since(self.last_save.load()) >= self.save_interval
            && self.storage.is_save_enabled()
        {
            self.last_save.store(now);
            for world in server.worlds.load().iter() {
                for player in world.players.load().iter() {
                    self.capture_player(player);
                }
            }
            let storage = self.storage.clone();
            self.jobs.spawn_blocking(move || {
                if let Err(error) = storage.flush_all() {
                    error!("Periodic player save failed: {error}");
                }
            });
        }
    }

    /// Joins all player storage jobs and retries retained snapshots on the blocking pool.
    pub async fn drain(&self) -> Result<(), PlayerDataError> {
        // PlayerList.saveAll runs serially on vanilla's server thread; keep each barrier intact.
        let _drain = self.drain_gate.lock().await;
        self.jobs.close();
        self.jobs.wait().await;
        self.jobs.reopen();
        let storage = self.storage.clone();
        self.jobs
            .spawn_blocking(move || storage.flush_all())
            .await
            .map_err(|error| PlayerDataError::Io(std::io::Error::other(error)))?
    }

    pub async fn save_all_players(&self, server: &Server) -> Result<(), PlayerDataError> {
        for world in server.worlds.load().iter() {
            for player in world.players.load().iter() {
                self.capture_player(player);
            }
        }
        self.drain().await
    }

    pub(crate) async fn load_client_data(
        &self,
        uuid: &uuid::Uuid,
        client: &crate::net::ClientPlatform,
    ) -> Result<(PlayerStorageSession, Option<NbtCompound>), PlayerDataError> {
        self.load_data_cancellable(uuid, async {
            tokio::select! {
                () = crate::STOP_INTERRUPT.cancelled() => {},
                () = async {
                    match client {
                        crate::net::ClientPlatform::Java(client) => client.await_close_interrupt().await,
                        crate::net::ClientPlatform::Bedrock(client) => client.await_close_interrupt().await,
                    }
                } => {},
            }
        }).await
    }

    /// Cancels a waiting login without bypassing an active or failed session's saved data.
    pub(crate) async fn load_data_cancellable(
        &self,
        uuid: &uuid::Uuid,
        cancelled: impl Future<Output = ()>,
    ) -> Result<(PlayerStorageSession, Option<NbtCompound>), PlayerDataError> {
        tokio::select! {
            biased;
            () = cancelled => Err(PlayerDataError::Io(std::io::Error::new(
                std::io::ErrorKind::Interrupted, "Player login cancelled"))),
            result = self.load_data(uuid) => result,
        }
    }

    /// Holds the UUID gate from recovery until final save and player removal finish.
    pub async fn load_data(
        &self,
        uuid: &uuid::Uuid,
    ) -> Result<(PlayerStorageSession, Option<NbtCompound>), PlayerDataError> {
        let gate = self.storage.acquire_session(uuid).await?;
        let storage = self.storage.clone();
        let uuid = *uuid;
        let (gate, loaded) = self
            .jobs
            .spawn_blocking(move || {
                // IOWorker.close: cancellation must not release ownership or bypass the drain.
                let loaded = storage.load_player_data(&uuid);
                (gate, loaded)
            })
            .await
            .map_err(|error| PlayerDataError::Io(std::io::Error::other(error)))?;
        let (should_load, data) = loaded?;
        Ok((
            PlayerStorageSession {
                _gate: gate,
                retired: false,
            },
            should_load.then_some(data),
        ))
    }
}

#[cfg(test)]
mod test {
    use crate::data::player_server::ServerPlayerData;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_world::data::player_data::PlayerDataStorage;
    use std::time::Duration;
    use std::time::Instant;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disconnect_saves_the_single_plugin_approved_leave_statistic() {
        use crate::{
            entity::{
                death_test_world::DeathTestWorld,
                player::statistics::{CustomStatistic, StatisticCategory, Statistics},
            },
            plugin::{
                BoxFuture, EventHandler, EventPriority,
                api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent,
            },
            server::Server,
            world::scoreboard::{NoTarget, ScoreboardObjective},
        };
        use pumpkin_protocol::java::client::play::RenderType;
        use pumpkin_util::text::TextComponent;
        use std::sync::Arc;

        struct ChangeLeaveCount;
        impl EventHandler<PlayerStatisticIncrementEvent> for ChangeLeaveCount {
            fn handle_blocking<'a>(
                &'a self,
                _server: &'a Arc<Server>,
                event: &'a mut PlayerStatisticIncrementEvent,
            ) -> BoxFuture<'a, ()> {
                Box::pin(async move {
                    assert_eq!(
                        event.statistic_id,
                        format!("Custom:{}", CustomStatistic::LeaveGame as i32)
                    );
                    event.amount = 3;
                })
            }
        }

        let fixture = DeathTestWorld::new().await;
        let server = &fixture.server;
        let player = fixture.player("Disconnect");
        server
            .plugin_manager
            .register(Arc::new(ChangeLeaveCount), EventPriority::Normal, true);
        fixture.world().scoreboard.lock().unwrap().add_objective(
            &NoTarget,
            ScoreboardObjective::new(
                "quits",
                TextComponent::text("quits"),
                RenderType::Integer,
                None,
                "minecraft.custom:minecraft.leave_game",
            ),
        );
        let storage = &server.player_data_storage;
        let (session, _) = storage.load_data(&player.gameprofile.id).await.unwrap();
        *player.storage_session.lock().unwrap() = Some(session);
        storage.handle_player_leave(&player, server).await.unwrap();
        let (_, saved) = storage
            .storage
            .load_player_data(&player.gameprofile.id)
            .unwrap();
        let mut saved_stats = Statistics::default();
        saved_stats.read_nbt(&saved);
        player.storage_session.lock().unwrap().take();
        server.shutdown().await;

        assert_eq!(player.get_custom_stat(CustomStatistic::LeaveGame), 3);
        assert_eq!(
            saved_stats.get(StatisticCategory::Custom, CustomStatistic::LeaveGame as i32),
            3,
        );
        assert_eq!(
            fixture
                .world()
                .scoreboard
                .lock()
                .unwrap()
                .get_score_value("Disconnect", "quits"),
            Some(3),
        );
    }

    #[tokio::test]
    async fn login_recovery_waits_for_previous_session_final_save() {
        let directory = tempdir().unwrap();
        let data = ServerPlayerData::new(directory.path(), Duration::from_secs(60), true);
        let uuid = Uuid::new_v4();
        let (session, _) = data.load_data(&uuid).await.unwrap();
        // The session returned by the real login path must own storage's UUID gate.
        let mut same_uuid = Box::pin(data.storage.acquire_session(&uuid));
        assert!(
            std::future::Future::poll(
                same_uuid.as_mut(),
                &mut std::task::Context::from_waker(std::task::Waker::noop())
            )
            .is_pending()
        );
        drop(same_uuid);
        let mut reconnect = Box::pin(data.load_data(&uuid));
        assert!(
            std::future::Future::poll(
                reconnect.as_mut(),
                &mut std::task::Context::from_waker(std::task::Waker::noop())
            )
            .is_pending()
        );
        let mut nbt = NbtCompound::new();
        nbt.put_int("InventoryRevision", 2);
        data.storage.snapshot(&uuid, || nbt);
        data.drain().await.unwrap();
        drop(session);
        let (_session, loaded) = reconnect.await.unwrap();
        assert_eq!(loaded.unwrap().get_int("InventoryRevision"), Some(2));
    }

    #[tokio::test]
    async fn waiting_login_cancels_without_releasing_the_previous_session() {
        let directory = tempdir().unwrap();
        let data = ServerPlayerData::new(directory.path(), Duration::from_secs(60), true);
        let uuid = Uuid::new_v4();
        let (session, _) = data.load_data(&uuid).await.unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut reconnect = Box::pin(data.load_data_cancellable(&uuid, cancel.cancelled()));
        assert!(
            std::future::Future::poll(
                reconnect.as_mut(),
                &mut std::task::Context::from_waker(std::task::Waker::noop())
            )
            .is_pending()
        );
        cancel.cancel();
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(1), reconnect).await.unwrap(), Err(pumpkin_world::data::player_data::PlayerDataError::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted)
        );
        let mut another = Box::pin(data.load_data(&uuid));
        assert!(
            std::future::Future::poll(
                another.as_mut(),
                &mut std::task::Context::from_waker(std::task::Waker::noop())
            )
            .is_pending()
        );
        drop(session);
        assert!(another.await.is_ok());
    }

    #[test]
    fn cancelled_recovery_remains_in_the_shutdown_barrier() {
        let directory = tempdir().unwrap();
        let data = ServerPlayerData::new(directory.path(), Duration::from_secs(60), true);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (release, held) = std::sync::mpsc::channel();
        runtime.spawn_blocking(move || held.recv().unwrap());
        runtime.block_on(async {
            let uuid = Uuid::new_v4();
            let mut login = Box::pin(data.load_data(&uuid));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(login.as_mut(), &mut context).is_pending());
            drop(login);
            // Exercise drain's producer barrier while recovery is still queued on the pool.
            data.jobs.close();
            let mut barrier = Box::pin(data.jobs.wait());
            let finished_early =
                std::future::Future::poll(barrier.as_mut(), &mut context).is_ready();
            release.send(()).unwrap();
            barrier.await;
            data.drain().await.unwrap();
            assert!(
                !finished_early,
                "shutdown abandoned cancelled login recovery"
            );
        });
    }

    #[test]
    fn storage_review_concurrent_drains_do_not_interleave_tracker_cycles() {
        let directory = tempdir().unwrap();
        let data = ServerPlayerData::new(directory.path(), Duration::from_secs(60), true);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (release, held) = std::sync::mpsc::channel();
        runtime.spawn_blocking(move || held.recv().unwrap());
        runtime.block_on(async {
            let mut first = Box::pin(data.drain());
            let mut second = Box::pin(data.drain());
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(first.as_mut(), &mut context).is_pending());
            assert!(!data.jobs.is_closed());
            assert!(std::future::Future::poll(second.as_mut(), &mut context).is_pending());
            // The first drain is still publishing; the second cannot start another close/wait.
            let interleaved = data.jobs.is_closed();
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                first.await.unwrap();
                second.await.unwrap();
            })
            .await
            .unwrap();
            assert!(!interleaved);
        });
    }

    #[tokio::test]
    async fn player_data_storage_new() {
        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        let storage = PlayerDataStorage::new(path.clone(), true);

        assert_eq!(storage.get_data_path().as_path(), path.as_path());
        // Note: save_enabled might be configured differently in your actual code
    }

    #[tokio::test]
    async fn player_data_storage_get_player_data_path() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        let storage = PlayerDataStorage::new(path.clone(), true);

        let uuid = Uuid::new_v4();
        let expected_path = path.join(format!("{uuid}.dat"));

        assert_eq!(storage.get_player_data_path(&uuid), expected_path);
    }

    #[tokio::test]
    async fn player_data_storage_save_and_load() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        let storage = PlayerDataStorage::new(path, true); // Ensure saving is enabled for this test

        let uuid = Uuid::new_v4();

        // Create test data
        let mut nbt = NbtCompound::new();
        nbt.put_string("TestKey", "TestValue".to_string());
        nbt.put_int("TestInt", 42);

        // Save the data
        storage.save_player_data(&uuid, nbt).unwrap();

        // Load the data
        let (load_success, loaded_nbt) = storage.load_player_data(&uuid).unwrap();

        assert!(load_success);
        assert_eq!(loaded_nbt.get_string("TestKey").unwrap(), "TestValue");
        assert_eq!(loaded_nbt.get_int("TestInt").unwrap(), 42);
    }

    #[tokio::test]
    async fn player_data_storage_load_nonexistent() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        let storage = PlayerDataStorage::new(path, true); // Ensure saving is enabled for this test

        let uuid = Uuid::new_v4();

        // Try to load non-existent data
        let (load_success, empty_nbt) = storage.load_player_data(&uuid).unwrap();

        assert!(!load_success);
        assert_eq!(empty_nbt.child_tags.len(), 0);
    }

    #[tokio::test]
    async fn player_data_storage_disabled() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        let storage = PlayerDataStorage::new(path, false);

        let uuid = Uuid::new_v4();
        let mut nbt = NbtCompound::new();
        nbt.put_string("TestKey", "TestValue".to_string());

        // Save should succeed but do nothing
        let save_result = storage.save_player_data(&uuid, nbt);
        assert!(save_result.is_ok());

        // Load should return empty data
        let (load_success, empty_nbt) = storage.load_player_data(&uuid).unwrap();
        assert!(!load_success);
        assert_eq!(empty_nbt.child_tags.len(), 0);
    }

    #[tokio::test]
    async fn server_player_data_new() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();
        let save_interval = Duration::from_mins(5);

        let player_data = ServerPlayerData::new(path, save_interval, true);

        assert_eq!(player_data.save_interval, save_interval);
        assert!(
            Instant::now().duration_since(player_data.last_save.load()) < Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn player_data_file_structure() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        let uuid = Uuid::new_v4();
        let storage = PlayerDataStorage::new(path, true);

        // Create and save player data
        let mut nbt = NbtCompound::new();
        nbt.put_string("name", "TestPlayer".to_string());
        nbt.put_int("level", 42);
        storage.save_player_data(&uuid, nbt).unwrap();

        // Verify the file exists
        let player_data_path = storage.get_player_data_path(&uuid);
        assert!(player_data_path.exists());

        // Load it again and verify content
        let (success, loaded_data) = storage.load_player_data(&uuid).unwrap();
        assert!(success);
        assert_eq!(loaded_data.get_string("name").unwrap(), "TestPlayer");
        assert_eq!(loaded_data.get_int("level").unwrap(), 42);
    }
}
