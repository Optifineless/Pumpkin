#![expect(
    clippy::unwrap_used,
    reason = "World restart regression uses a valid temporary world fixture"
)]

use crate::{
    command::CommandSender,
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::dimension::Dimension;
use pumpkin_nbt::{compound::NbtCompound, nbt_compress::write_gzip_compound_tag};
use pumpkin_world::world_info::{
    MAXIMUM_SUPPORTED_LEVEL_VERSION, MAXIMUM_SUPPORTED_WORLD_DATA_VERSION,
    data_files::{DimensionClock, read_world_clocks, write_world_clocks},
};
use std::{
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::Arc,
};

async fn with_world<T>(
    path: &Path,
    check: impl FnOnce(&Arc<Server>, &Arc<World>) -> T,
) -> std::thread::Result<T> {
    with_worlds(path, &[Dimension::OVERWORLD], |server, worlds| {
        check(server, &worlds[0])
    })
    .await
}

async fn with_worlds<T>(
    path: &Path,
    dimensions: &[Dimension],
    check: impl FnOnce(&Arc<Server>, &[Arc<World>]) -> T,
) -> std::thread::Result<T> {
    let server = combat_test_support::server(path);
    let worlds: Vec<_> = dimensions
        .iter()
        .map(|dimension| {
            Arc::new(World::load(
                pumpkin_world::level::Level::from_root_folder(
                    &pumpkin_config::world::LevelConfig::default(),
                    path.to_path_buf(),
                    0,
                    dimension.clone(),
                ),
                server.level_info.clone(),
                dimension.clone(),
                server.block_registry.clone(),
                Arc::downgrade(&server),
            ))
        })
        .collect();
    server.worlds.store(Arc::new(worlds.clone()));
    let levels: Vec<_> = worlds
        .iter()
        .map(|world| Arc::downgrade(&world.level))
        .collect();
    let result = catch_unwind(AssertUnwindSafe(|| check(&server, &worlds)));
    server.shutdown().await;
    let scheduler_stopped = worlds
        .iter()
        .all(|world| world.level.cancel_token.is_cancelled());
    // A failing cleanup negative control must also release its scheduler before deleting files.
    if !scheduler_stopped {
        for world in &worlds {
            world.shutdown().await;
        }
    }
    drop(worlds);
    drop(server);
    assert!(
        scheduler_stopped,
        "Server::shutdown missed the fixture world"
    );
    assert!(
        levels.iter().all(|level| level.upgrade().is_none()),
        "Fixture scheduler retained its level"
    );
    result
}

fn write_old_fork_world_data(path: &Path) {
    let mut world_gen_settings = NbtCompound::new();
    world_gen_settings.put_long("seed", 0);
    let mut data = NbtCompound::new();
    data.put_int("DataVersion", MAXIMUM_SUPPORTED_WORLD_DATA_VERSION);
    data.put_int("version", MAXIMUM_SUPPORTED_LEVEL_VERSION);
    data.put_string("LevelName", "legacy clock world".to_string());
    data.put_long("DayTime", 5000);
    data.put_compound("WorldGenSettings", world_gen_settings);
    let mut root = NbtCompound::new();
    root.put_compound("Data", data);
    write_gzip_compound_tag(root, fs::File::create(path.join("level.dat")).unwrap()).unwrap();

    let mut overworld = NbtCompound::new();
    overworld.put_int("total_ticks", 5000);
    let mut clocks = NbtCompound::new();
    clocks.put_compound("minecraft:overworld", overworld);
    clocks.put_int("DataVersion", MAXIMUM_SUPPORTED_WORLD_DATA_VERSION);
    let mut root = NbtCompound::new();
    root.put_compound("data", clocks);
    let data_dir = path.join("data/minecraft");
    fs::create_dir_all(&data_dir).unwrap();
    write_gzip_compound_tag(
        root,
        fs::File::create(data_dir.join("world_clocks.dat")).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn baseline_regression_old_fork_clock_save_loads_through_world_creation() {
    let directory = tempfile::tempdir().unwrap();
    write_old_fork_world_data(directory.path());
    with_world(directory.path(), |_, world| {
        let time = world.level_time.lock().unwrap();
        assert_eq!(time.world_age, 0);
        assert_eq!(time.time_of_day, 5000);
        assert_eq!(time.partial_tick, 0.0);
        assert_eq!(time.rate, 1.0);
        assert!(!time.paused);
    })
    .await
    .unwrap();
    let saved = pumpkin_nbt::nbt_compress::read_gzip_compound_tag(
        fs::File::open(directory.path().join("data/minecraft/world_clocks.dat")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        saved
            .get_compound("data")
            .unwrap()
            .get_compound("minecraft:overworld")
            .unwrap()
            .get_long("total_ticks"),
        Some(5000)
    );
    with_world(directory.path(), |_, world| {
        assert_eq!(world.level_time.lock().unwrap().time_of_day, 5000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn malformed_fraction_recovers_through_world_load_tick_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    write_old_fork_world_data(directory.path());
    let mut clocks = read_world_clocks(directory.path());
    clocks
        .clocks
        .get_mut("minecraft:overworld")
        .unwrap()
        .partial_tick = f32::NAN;
    write_world_clocks(directory.path(), &clocks).unwrap();
    with_world(directory.path(), |_, world| {
        world.tick_environment();
        let time = world.level_time.lock().unwrap();
        assert_eq!(time.world_age, 1);
        assert_eq!(time.time_of_day, 5001);
        assert_eq!(time.partial_tick, 0.0);
    })
    .await
    .unwrap();
    with_world(directory.path(), |_, world| {
        assert_eq!(world.level_time.lock().unwrap().time_of_day, 5001);
        world.tick_environment();
        assert_eq!(world.level_time.lock().unwrap().time_of_day, 5002);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dimension_clocks_restart_independently_of_global_game_time() {
    let directory = tempfile::tempdir().unwrap();
    write_old_fork_world_data(directory.path());
    let mut clocks = read_world_clocks(directory.path());
    clocks.clocks.insert(
        "minecraft:the_nether".into(),
        DimensionClock {
            total_ticks: 19000,
            partial_tick: 0.25,
            rate: 0.5,
            paused: false,
        },
    );
    clocks.clocks.insert(
        "test:unloaded".into(),
        DimensionClock {
            total_ticks: 9000,
            partial_tick: 0.75,
            rate: 3.0,
            paused: true,
        },
    );
    write_world_clocks(directory.path(), &clocks).unwrap();
    let dimensions = [
        Dimension::OVERWORLD,
        Dimension::THE_NETHER,
        Dimension::THE_END,
    ];
    with_worlds(directory.path(), &dimensions, |server, worlds| {
        let source = CommandSender::Console.into_source(server);
        let dispatcher = server.command_dispatcher.load();
        dispatcher.execute_input("time set 6000", &source).unwrap();
        dispatcher.execute_input("time rate 2.5", &source).unwrap();
        for _ in 0..3 {
            worlds[0].tick_environment();
        }
        dispatcher.execute_input("time pause", &source).unwrap();
        let nether = worlds[1].level_time.lock().unwrap();
        assert_eq!(
            (
                nether.time_of_day,
                nether.partial_tick,
                nether.rate,
                nether.paused
            ),
            (19000, 0.25, 0.5, false)
        );
        let end = worlds[2].level_time.lock().unwrap();
        assert_eq!(
            (end.time_of_day, end.partial_tick, end.rate, end.paused),
            (0, 0.0, 1.0, false)
        );
    })
    .await
    .unwrap();
    with_worlds(directory.path(), &dimensions, |_, worlds| {
        for (world, expected) in worlds.iter().zip([
            (6007, 0.5, 2.5, true),
            (19000, 0.25, 0.5, false),
            (0, 0.0, 1.0, false),
        ]) {
            let time = world.level_time.lock().unwrap();
            assert_eq!(time.world_age, 3);
            assert_eq!(
                (time.time_of_day, time.partial_tick, time.rate, time.paused),
                expected
            );
        }
    })
    .await
    .unwrap();
    assert_eq!(
        read_world_clocks(directory.path()).clocks["test:unloaded"],
        clocks.clocks["test:unloaded"]
    );
}

#[tokio::test]
async fn baseline_regression_shutdown_reloads_game_time_and_full_overworld_clock_state()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    with_world(directory.path(), |server, world| -> Result<(), String> {
        let source = CommandSender::Console.into_source(server);
        let dispatcher = server.command_dispatcher.load();
        dispatcher
            .execute_input("time set 6000", &source)
            .map_err(|error| format!("{error:?}"))?;
        dispatcher
            .execute_input("time rate 2.5", &source)
            .map_err(|error| format!("{error:?}"))?;
        for _ in 0..3 {
            world.tick_environment();
        }
        dispatcher
            .execute_input("time pause", &source)
            .map_err(|error| format!("{error:?}"))?;

        Ok(())
    })
    .await
    .unwrap()?;

    with_world(directory.path(), |_, world| {
        let time = world.level_time.lock().unwrap();
        assert_eq!(time.world_age, 3);
        assert_eq!(time.time_of_day, 6007);
        assert_eq!(time.partial_tick, 0.5);
        assert_eq!(time.rate, 2.5);
        assert!(time.paused);
    })
    .await
    .unwrap();

    Ok(())
}

#[tokio::test]
async fn failed_restart_fixture_stops_scheduler_before_next_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let failed_assertion = with_world(directory.path(), |_, world| {
        assert!(
            world.level.cancel_token.is_cancelled(),
            "Injected fixture assertion"
        );
    })
    .await;
    assert!(failed_assertion.is_err());
    let failed_command = with_world(directory.path(), |server, _| {
        let source = CommandSender::Console.into_source(server);
        server
            .command_dispatcher
            .load()
            .execute_input("time set invalid", &source)
    })
    .await
    .unwrap();
    assert!(failed_command.is_err());
    with_world(directory.path(), |_, world| {
        assert!(!world.level.cancel_token.is_cancelled());
        world.tick_environment();
        assert_eq!(world.level_time.lock().unwrap().world_age, 1);
    })
    .await
    .unwrap();
    directory.close().unwrap();
}
