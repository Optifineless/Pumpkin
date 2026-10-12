use super::world_info_save_tests::{set_clock, with_worlds};
use super::*;
use crate::plugin::{
    EventHandler, EventPriority, api::events::world::world_load::WorldUnloadEvent,
};
use crate::world::time::LevelTime;
use futures::future::BoxFuture;
use pumpkin_world::world_info::data_files::{
    DimensionClock, read_world_clocks, write_world_clocks,
};
use std::{collections::HashSet, sync::atomic::AtomicUsize};

fn clock(world: &World) -> DimensionClock {
    let time = world.level_time.lock().unwrap();
    DimensionClock {
        total_ticks: time.time_of_day,
        partial_tick: time.partial_tick,
        rate: time.rate,
        paused: time.paused,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_clock_save_uses_actual_registered_clock_keys() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        for (world, ticks) in worlds.iter().zip([100, 200, 300]) {
            set_clock(&server, world, ticks, 0.75, true);
        }
        server.save_all().await.unwrap();
        let saved = read_world_clocks(&server.basic_config.get_world_path());
        let registered: HashSet<_> = pumpkin_data::registry::REGISTRY_V_26_3
            .iter()
            .find(|registry| registry.registry_id == "world_clock")
            .unwrap()
            .entries
            .iter()
            .map(|entry| format!("minecraft:{}", entry.name))
            .collect();
        assert!(
            saved.clocks.keys().all(|key| registered.contains(key)),
            "Canonical dimensions emitted an unregistered world-clock key: {:?}",
            saved.clocks.keys()
        );
        assert_eq!(saved.clocks["minecraft:overworld"], clock(&worlds[0]));
        assert_eq!(saved.clocks["minecraft:the_end"], clock(&worlds[2]));
        assert!(!saved.clocks.contains_key("minecraft:the_nether"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_clock_load_uses_declared_clock_when_dimension_id_differs() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[0], 1200, 0.25, true);
        server.save_all().await.unwrap();
        let path = server.basic_config.get_world_path();
        let saved = AnvilLevelInfo.read_world_info(&path).unwrap();
        let caves = LevelTime::from_world_data(&saved, &path, &Dimension::OVERWORLD_CAVES, true);
        assert_eq!(
            (
                caves.time_of_day,
                caves.partial_tick,
                caves.rate,
                caves.paused
            ),
            (1200, 0.75, 0.25, true),
            "Overworld-caves did not restore its declared overworld clock"
        );
        let nether = LevelTime::from_world_data(&saved, &path, &Dimension::THE_NETHER, true);
        assert_eq!(
            (
                nether.time_of_day,
                nether.partial_tick,
                nether.rate,
                nether.paused
            ),
            (0, 0.0, 1.0, false)
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_clock_save_removes_known_invalid_aliases_and_preserves_opaque_records() {
    let directory = tempfile::tempdir().unwrap();
    AnvilLevelInfo
        .write_world_info(
            &LevelData::default(pumpkin_util::world_seed::Seed(0)),
            directory.path(),
        )
        .unwrap();
    let mut imported = read_world_clocks(directory.path());
    let stale = DimensionClock {
        total_ticks: 9999,
        partial_tick: 0.5,
        rate: 3.5,
        paused: true,
    };
    for key in [
        "minecraft:the_nether",
        "minecraft:overworld_caves",
        "test:unloaded",
    ] {
        imported.clocks.insert(key.into(), stale.clone());
    }
    write_world_clocks(directory.path(), &imported).unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[0], 400, 1.25, true);
        set_clock(&server, &worlds[2], 500, 0.75, true);
        server.save_all().await.unwrap();
        let saved = read_world_clocks(&server.basic_config.get_world_path());
        assert!(
            !saved.clocks.contains_key("minecraft:the_nether")
                && !saved.clocks.contains_key("minecraft:overworld_caves"),
            "Known fork-created dimension aliases survived clock migration"
        );
        assert_eq!(saved.clocks["test:unloaded"], stale);
        assert_eq!(saved.clocks["minecraft:overworld"], clock(&worlds[0]));
        assert_eq!(saved.clocks["minecraft:the_end"], clock(&worlds[2]));
    })
    .await;
}

struct FinalClock(AtomicUsize);
impl EventHandler<WorldUnloadEvent> for FinalClock {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut WorldUnloadEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.world.dimension == Dimension::THE_END {
                set_clock(server, &event.world, 800, 0.75, true);
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        })
    }
}

fn reloaded_end(server: &Arc<Server>) -> Arc<World> {
    Arc::new(World::load(
        pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            server.basic_config.get_world_path(),
            0,
            Dimension::THE_END,
        ),
        server.level_info.clone(),
        Dimension::THE_END,
        server.block_registry.clone(),
        Arc::downgrade(server),
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_clock_unload_retains_callback_state_across_failed_save_and_reload() {
    let directory = tempfile::tempdir().unwrap();
    AnvilLevelInfo
        .write_world_info(
            &LevelData::default(pumpkin_util::world_seed::Seed(0)),
            directory.path(),
        )
        .unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[2], 300, 1.25, true);
        let hook = Arc::new(FinalClock(AtomicUsize::new(0)));
        server.plugin_manager.register::<WorldUnloadEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server
            .unload_world(Dimension::THE_END.minecraft_name)
            .await
            .unwrap();
        assert_eq!(hook.0.load(Ordering::Relaxed), 1);
        let expected = clock(&worlds[2]);
        assert_eq!(
            expected,
            DimensionClock {
                total_ticks: 802,
                partial_tick: 0.25,
                rate: 0.75,
                paused: true
            }
        );
        let path = server.basic_config.get_world_path();
        let destination = path.join("data/minecraft/world_clocks.dat");
        let backup = path.join("clock-before-unload-fault.dat");
        fs::rename(&destination, &backup).unwrap();
        fs::create_dir(&destination).unwrap();
        let result = server.save_all().await;
        fs::remove_dir(&destination).unwrap();
        fs::rename(&backup, &destination).unwrap();
        assert!(result.is_err());
        // A real canonical reload happens before the failed metadata save is retried.
        let restored = reloaded_end(&server);
        server.worlds.rcu(|current| {
            let mut next = (**current).clone();
            next.push(restored.clone());
            next
        });
        assert_eq!(
            clock(&restored),
            expected,
            "Canonical reload lost the final unloaded clock after a failed save"
        );
        let alias = server.create_world(".".into(), Dimension::THE_END);
        set_clock(&server, &alias, 9000, 3.5, false);
        set_clock(&server, &restored, 1000, 1.25, true);
        let newer = clock(&restored);
        server.save_all().await.unwrap();
        assert_eq!(read_world_clocks(&path).clocks["minecraft:the_end"], newer);
        let disk = AnvilLevelInfo.read_world_info(&path).unwrap();
        let reopened = LevelTime::from_world_data(&disk, &path, &Dimension::THE_END, true);
        assert_eq!(
            (
                reopened.time_of_day,
                reopened.partial_tick,
                reopened.rate,
                reopened.paused
            ),
            (
                newer.total_ticks,
                newer.partial_tick,
                newer.rate,
                newer.paused
            )
        );
    })
    .await;
}

struct ReplaceCanonical(AtomicUsize);
impl EventHandler<WorldUnloadEvent> for ReplaceCanonical {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut WorldUnloadEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.world.dimension == Dimension::THE_END
                && self.0.fetch_add(1, Ordering::Relaxed) == 0
            {
                let newer = reloaded_end(server);
                set_clock(server, &newer, 2000, 0.25, true);
                server.worlds.rcu(|current| {
                    let mut next = (**current).clone();
                    next.retain(|world| !Arc::ptr_eq(world, &event.world));
                    next.push(newer.clone());
                    next
                });
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_clock_old_unload_completion_preserves_newer_canonical_owner() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[2], 300, 1.25, true);
        let hook = Arc::new(ReplaceCanonical(AtomicUsize::new(0)));
        server.plugin_manager.register::<WorldUnloadEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server
            .unload_world(Dimension::THE_END.minecraft_name)
            .await
            .unwrap();
        assert_eq!(hook.0.load(Ordering::Relaxed), 1);
        let current = server.worlds.load_full();
        let newer = current
            .iter()
            .find(|world| world.dimension == Dimension::THE_END)
            .unwrap();
        assert!(!Arc::ptr_eq(newer, &worlds[2]));
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&server.basic_config.get_world_path()).clocks["minecraft:the_end"],
            clock(newer)
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_review_custom_root_loads_its_own_saved_clock() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[2], 300, 1.25, true);
        server.save_all().await.unwrap();
        let canonical = clock(&worlds[2]);
        let other_root = server.basic_config.get_world_path().join("other_end");
        AnvilLevelInfo
            .write_world_info(
                &LevelData::default(pumpkin_util::world_seed::Seed(0)),
                &other_root,
            )
            .unwrap();
        let expected = DimensionClock {
            total_ticks: 900,
            partial_tick: 0.5,
            rate: 3.5,
            paused: false,
        };
        let mut imported = read_world_clocks(&other_root);
        imported
            .clocks
            .insert("minecraft:the_end".into(), expected.clone());
        write_world_clocks(&other_root, &imported).unwrap();
        assert_eq!(
            read_world_clocks(&other_root).clocks["minecraft:the_end"],
            expected
        );
        let other = server.create_world("other_end".into(), Dimension::THE_END);
        assert!(!other.saves_default_clock);
        assert_eq!(clock(&worlds[2]), canonical);
        assert_eq!(
            clock(&other),
            expected,
            "Custom root loaded the canonical retained clock instead of its own disk"
        );
    })
    .await;
}

struct UnloadEndOnSpawn(AtomicUsize);

impl EventHandler<crate::plugin::world::spawn_change::SpawnChangeEvent> for UnloadEndOnSpawn {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _event: &'a mut crate::plugin::world::spawn_change::SpawnChangeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            server
                .unload_world(Dimension::THE_END.minecraft_name)
                .await
                .unwrap();
            self.0.fetch_add(1, Ordering::Relaxed);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_review_spawn_callback_preserves_final_unloaded_clock() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[2], 300, 1.25, true);
        server.save_all().await.unwrap();
        let unload = Arc::new(FinalClock(AtomicUsize::new(0)));
        server.plugin_manager.register::<WorldUnloadEvent, _>(
            unload.clone(),
            EventPriority::Normal,
            true,
        );
        let spawn = Arc::new(UnloadEndOnSpawn(AtomicUsize::new(0)));
        server
            .plugin_manager
            .register::<crate::plugin::world::spawn_change::SpawnChangeEvent, _>(
                spawn.clone(),
                EventPriority::Normal,
                true,
            );
        let source = crate::command::CommandSender::Console
            .into_source(&server)
            .with_world(worlds[0].clone());
        assert_eq!(
            server
                .command_dispatcher
                .load()
                .execute_input("setworldspawn 8 64 9", &source)
                .unwrap(),
            1
        );
        assert_eq!(spawn.0.load(Ordering::Relaxed), 1);
        assert_eq!(unload.0.load(Ordering::Relaxed), 1);
        let info = server.level_info.load();
        assert_eq!((info.spawn_x, info.spawn_y, info.spawn_z), (8, 64, 9));
        drop(info);
        let expected = clock(&worlds[2]);
        assert_eq!(expected.total_ticks, 802);
        server.save_all().await.unwrap();
        let saved = read_world_clocks(&server.basic_config.get_world_path());
        let restored = reloaded_end(&server);
        server.worlds.rcu(|current| {
            let mut next = (**current).clone();
            next.push(restored.clone());
            next
        });
        assert_eq!(
            saved.clocks.get("minecraft:the_end"),
            Some(&expected),
            "Spawn callback publication erased the final unloaded clock before save"
        );
        assert_eq!(clock(&restored), expected);
    })
    .await;
}
