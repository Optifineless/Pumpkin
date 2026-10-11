use super::*;
use crate::command::CommandSender;
use futures::FutureExt;
use pumpkin_world::world_info::data_files::{
    DimensionClock, read_world_clocks, write_world_clocks,
};
use std::{panic::AssertUnwindSafe, path::Path, sync::mpsc};

struct SaveSignal(mpsc::Sender<bool>);

impl WorldInfoWriter for SaveSignal {
    fn write_world_info(&self, info: &LevelData, path: &Path) -> Result<(), WorldInfoError> {
        let result = AnvilLevelInfo.write_world_info(info, path);
        let _ = self.0.send(result.is_ok());
        result
    }
}

async fn with_worlds<F: Future<Output = ()>>(
    path: &Path,
    writer: Option<Arc<dyn WorldInfoWriter>>,
    check: impl FnOnce(Arc<Server>, Vec<Arc<World>>) -> F,
) {
    let mut server = combat_test_support::server(path);
    if let Some(writer) = writer {
        Arc::get_mut(&mut server).unwrap().world_info_writer = writer;
    }
    let worlds: Vec<_> = [
        Dimension::OVERWORLD,
        Dimension::THE_NETHER,
        Dimension::THE_END,
    ]
    .into_iter()
    .map(|dimension| {
        Arc::new(World::load(
            pumpkin_world::level::Level::from_root_folder(
                &pumpkin_config::world::LevelConfig::default(),
                path.to_path_buf(),
                0,
                dimension.clone(),
            ),
            server.level_info.clone(),
            dimension,
            server.block_registry.clone(),
            Arc::downgrade(&server),
        ))
    })
    .collect();
    server.worlds.store(Arc::new(worlds.clone()));
    let result = AssertUnwindSafe(check(server.clone(), worlds.clone()))
        .catch_unwind()
        .await;
    let mut owned_worlds = worlds.clone();
    for world in server.worlds.load().iter() {
        if !owned_worlds.iter().any(|owned| Arc::ptr_eq(owned, world)) {
            owned_worlds.push(world.clone());
        }
    }
    let levels: Vec<_> = owned_worlds
        .iter()
        .map(|world| Arc::downgrade(&world.level))
        .collect();
    server.shutdown().await;
    assert!(
        owned_worlds
            .iter()
            .all(|world| world.level.cancel_token.is_cancelled())
    );
    // Release fixture-owned portal links after their schedulers have stopped.
    for world in &owned_worlds {
        world.level.world_portal.store(Arc::new(None));
    }
    drop(owned_worlds);
    drop(worlds);
    drop(server);
    assert!(levels.iter().all(|level| level.upgrade().is_none()));
    result.unwrap();
}

fn set_clock(server: &Arc<Server>, world: &Arc<World>, ticks: i64, rate: f32, paused: bool) {
    let source = CommandSender::Console
        .into_source(server)
        .with_world(world.clone());
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input(&format!("time set {ticks}"), &source)
        .unwrap();
    dispatcher
        .execute_input(&format!("time rate {rate}"), &source)
        .unwrap();
    dispatcher.execute_input("time resume", &source).unwrap();
    for _ in 0..3 {
        world.tick_environment();
    }
    if paused {
        dispatcher.execute_input("time pause", &source).unwrap();
    }
}

const AUTOSAVED_CLOCKS: [(i64, f32, f32, bool); 3] = [
    (107, 0.5, 2.5, true),
    (201, 0.5, 0.5, false),
    (303, 0.75, 1.25, true),
];
const SAVE_ALL_CLOCKS: [(i64, f32, f32, bool); 3] = [
    (107, 0.5, 2.5, true),
    (902, 0.25, 0.75, true),
    (1200, 0.75, 0.25, false),
];

fn assert_clocks(path: &Path, expected: &[(i64, f32, f32, bool)]) {
    let saved = read_world_clocks(path);
    for (dimension, &(ticks, fraction, rate, paused)) in [
        Dimension::OVERWORLD,
        Dimension::THE_NETHER,
        Dimension::THE_END,
    ]
    .iter()
    .zip(expected)
    {
        assert!(
            saved.clocks.contains_key(dimension.minecraft_name),
            "Save omitted live {} clock",
            dimension.minecraft_name
        );
        let clock = &saved.clocks[dimension.minecraft_name];
        assert_eq!(
            (
                clock.total_ticks,
                clock.partial_tick,
                clock.rate,
                clock.paused
            ),
            (ticks, fraction, rate, paused)
        );
    }
    assert_eq!(saved.clocks["test:unloaded"].total_ticks, 9999);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn autosave_and_save_all_persist_live_dimension_clocks_before_restart() {
    let directory = tempfile::tempdir().unwrap();
    AnvilLevelInfo
        .write_world_info(
            &LevelData::default(pumpkin_util::world_seed::Seed(42)),
            directory.path(),
        )
        .unwrap();
    let mut imported = read_world_clocks(directory.path());
    imported.clocks.insert(
        "test:unloaded".into(),
        DimensionClock {
            total_ticks: 9999,
            partial_tick: 0.75,
            rate: 3.0,
            paused: true,
        },
    );
    write_world_clocks(directory.path(), &imported).unwrap();
    let (signal, completed) = mpsc::channel();
    with_worlds(
        directory.path(),
        Some(Arc::new(SaveSignal(signal))),
        |server, worlds| async move {
            for ((world, ticks), (rate, paused)) in
                worlds
                    .iter()
                    .zip([100, 200, 300])
                    .zip([(2.5, true), (0.5, false), (1.25, true)])
            {
                set_clock(&server, world, ticks, rate, paused);
            }
            worlds[0].level_time.lock().unwrap().world_age =
                i64::try_from(worlds[0].level.autosave_ticks).unwrap() - 1;
            worlds[0].tick_environment();
            let saved = tokio::task::spawn_blocking(move || {
                completed.recv_timeout(Duration::from_secs(30)).unwrap()
            })
            .await
            .unwrap();
            assert!(saved, "Production autosave failed");
            // Inspect before shutdown; its final save cannot hide a broken autosave.
            let path = server.basic_config.get_world_path();
            assert_clocks(&path, &AUTOSAVED_CLOCKS);
            set_clock(&server, &worlds[1], 900, 0.75, true);
            set_clock(&server, &worlds[2], 1200, 0.25, false);
            server.save_all().await.unwrap();
            assert_clocks(&path, &SAVE_ALL_CLOCKS);
        },
    )
    .await;
    with_worlds(directory.path(), None, |_, worlds| async move {
        for (world, expected) in worlds.iter().zip(SAVE_ALL_CLOCKS) {
            let clock = world.level_time.lock().unwrap();
            assert_eq!(
                (
                    clock.time_of_day,
                    clock.partial_tick,
                    clock.rate,
                    clock.paused
                ),
                expected
            );
        }
    })
    .await;
    assert_eq!(
        read_world_clocks(directory.path()).clocks["test:unloaded"],
        imported.clocks["test:unloaded"]
    );
}

#[tokio::test]
async fn save_all_reports_authoritative_clock_write_failure_and_retry_recovers() {
    let directory = tempfile::tempdir().unwrap();
    AnvilLevelInfo
        .write_world_info(
            &LevelData::default(pumpkin_util::world_seed::Seed(42)),
            directory.path(),
        )
        .unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[1], 900, 0.75, true);
        let path = server.basic_config.get_world_path();
        let clock_path = path.join("data/minecraft/world_clocks.dat");
        let backup = path.join("clock-before-fault.dat");
        let previous = fs::read(&clock_path).unwrap();
        fs::rename(&clock_path, &backup).unwrap();
        fs::create_dir(&clock_path).unwrap();
        // The real writer cannot replace a directory. Restore the old bytes before asserting
        // so failed negative controls still shut down without a broken fixture path.
        let result = server.save_all().await;
        fs::remove_dir(&clock_path).unwrap();
        fs::rename(&backup, &clock_path).unwrap();
        assert_eq!(fs::read(&clock_path).unwrap(), previous);
        assert!(
            result.is_err(),
            "save_all reported success after its authoritative clock write failed"
        );
        assert!(result.unwrap_err().contains("Failed to save world info"));
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&path).clocks["minecraft:the_nether"].total_ticks,
            902
        );
    })
    .await;
    with_worlds(directory.path(), None, |_, worlds| async move {
        let clock = worlds[1].level_time.lock().unwrap();
        assert_eq!(
            (
                clock.time_of_day,
                clock.partial_tick,
                clock.rate,
                clock.paused
            ),
            (902, 0.25, 0.75, true)
        );
    })
    .await;
}

#[tokio::test]
async fn custom_dimension_worlds_do_not_overwrite_default_clock_saved_data() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        for (world, ticks) in worlds.iter().zip([100, 200, 300]) {
            set_clock(&server, world, ticks, 1.0, true);
        }
        let extra_overworld = server.create_world("other_overworld".into(), Dimension::OVERWORLD);
        let extra_nether = server.create_world("other_nether".into(), Dimension::THE_NETHER);
        set_clock(&server, &extra_overworld, 9000, 2.0, false);
        set_clock(&server, &extra_nether, 19000, 3.0, false);
        let dot_alias = server.create_world(".".into(), Dimension::OVERWORLD);
        let absolute_alias = server.create_world(
            server
                .basic_config
                .get_world_path()
                .to_string_lossy()
                .into_owned(),
            Dimension::THE_NETHER,
        );
        assert_eq!(
            dot_alias.level.level_folder.root_folder,
            worlds[0].level.level_folder.root_folder
        );
        assert_eq!(
            absolute_alias.level.level_folder.root_folder,
            worlds[1].level.level_folder.root_folder
        );
        set_clock(&server, &dot_alias, 29000, 4.0, false);
        set_clock(&server, &absolute_alias, 39000, 5.0, false);
        assert_ne!(
            extra_overworld.level.level_folder.root_folder,
            worlds[0].level.level_folder.root_folder
        );
        server.save_all().await.unwrap();
        let saved = read_world_clocks(&server.basic_config.get_world_path());
        for (world, expected) in worlds.iter().zip([103, 203, 303]) {
            assert_eq!(
                saved.clocks[world.dimension.minecraft_name].total_ticks, expected,
                "Custom world replaced the canonical clock"
            );
        }
        server
            .unload_world(Dimension::THE_NETHER.minecraft_name)
            .await
            .unwrap();
        assert!(
            !server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &worlds[1]))
        );
        assert!(
            server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &absolute_alias))
        );
        set_clock(&server, &absolute_alias, 49000, 6.0, false);
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&server.basic_config.get_world_path()).clocks
                [Dimension::THE_NETHER.minecraft_name]
                .total_ticks,
            203,
            "Created alias replaced an unloaded canonical clock"
        );
    })
    .await;
    with_worlds(directory.path(), None, |_, worlds| async move {
        for (world, ticks) in worlds.iter().zip([103, 203, 303]) {
            assert_eq!(world.level_time.lock().unwrap().time_of_day, ticks);
        }
    })
    .await;
}
