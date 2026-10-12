use super::{
    world_info_save_tests::{set_clock, with_worlds},
    *,
};
use crate::plugin::{
    EventHandler, EventPriority, api::events::world::world_load::WorldUnloadEvent,
};
use futures::future::BoxFuture;
use pumpkin_nbt::nbt_compress::{read_gzip_compound_tag, write_gzip_compound_tag};
use pumpkin_world::world_info::data_files::{
    DimensionClock, WorldClocksData, read_world_clocks, write_world_clocks,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn clock(world: &World) -> DimensionClock {
    let time = world.level_time.lock().unwrap();
    DimensionClock {
        total_ticks: time.time_of_day,
        partial_tick: time.partial_tick,
        rate: time.rate,
        paused: time.paused,
    }
}

fn imported_clock() -> DimensionClock {
    DimensionClock {
        total_ticks: 900,
        partial_tick: 0.5,
        rate: 3.5,
        paused: false,
    }
}

fn changed_clock() -> DimensionClock {
    DimensionClock {
        total_ticks: 1507,
        partial_tick: 0.5,
        rate: 2.5,
        paused: true,
    }
}

fn prepare_named_end(path: &Path) {
    let mut data = WorldClocksData::default();
    data.clocks.insert(
        Dimension::THE_END.default_clock.unwrap().into(),
        imported_clock(),
    );
    data.clocks
        .insert("test:named_unloaded".into(), imported_clock());
    write_world_clocks(&path.join("other_end"), &data).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_save_all_persists_dispatcher_clock_before_restart() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[0], 100, 2.5, true);
        let main = clock(&worlds[0]);
        let named = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(
            clock(&named),
            imported_clock(),
            "actual own-root disk setup failed"
        );
        set_clock(&server, &named, 1500, 2.5, true);
        assert_eq!(
            clock(&named),
            changed_clock(),
            "actual named dispatcher commands failed"
        );
        server.save_all().await.unwrap();
        let root = server.basic_config.get_world_path();
        let main_saved = read_world_clocks(&root);
        assert_eq!(
            main_saved.clocks[Dimension::OVERWORLD.default_clock.unwrap()],
            main
        );
        let saved = read_world_clocks(&root.join("other_end"));
        assert_eq!(
            saved.clocks[Dimension::THE_END.default_clock.unwrap()],
            changed_clock(),
            "named root save-all lost the actual dispatcher clock"
        );
        assert_eq!(saved.clocks["test:named_unloaded"], imported_clock());
    })
    .await;
    with_worlds(directory.path(), None, |server, worlds| async move {
        let named = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(
            clock(&named),
            changed_clock(),
            "named root restart lost saved clock fields"
        );
        assert_eq!(clock(&worlds[0]).total_ticks, 107);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_shutdown_persists_dispatcher_clock_document() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, _| async move {
        let named = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(clock(&named), imported_clock());
        set_clock(&server, &named, 1500, 2.5, true);
        assert_eq!(clock(&named), changed_clock());
        // with_worlds performs the actual Server::shutdown and scheduler/cycle cleanup.
    })
    .await;
    let saved = read_world_clocks(&directory.path().join("other_end"));
    assert_eq!(
        saved.clocks[Dimension::THE_END.default_clock.unwrap()],
        changed_clock(),
        "named root shutdown lost the actual dispatcher clock"
    );
    with_worlds(directory.path(), None, |server, _| async move {
        let named = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(clock(&named), changed_clock());
    })
    .await;
}

struct FinalNamedClock(AtomicUsize);

impl EventHandler<WorldUnloadEvent> for FinalNamedClock {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut WorldUnloadEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.world.get_world_name() != "other_end" {
                return;
            }
            assert!(
                server.world_info_save_lock.try_lock().is_ok(),
                "named unload callback held save ownership"
            );
            assert!(
                event.world.level_time.try_lock().is_ok(),
                "named unload callback held time ownership"
            );
            set_clock(server, &event.world, 800, 0.75, true);
            self.0.fetch_add(1, Ordering::Relaxed);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_unload_retains_final_native_callback_clock_before_reload() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, _| async move {
        let retired = server.create_world("other_end".into(), Dimension::THE_END);
        let hook = Arc::new(FinalNamedClock(AtomicUsize::new(0)));
        server.plugin_manager.register::<WorldUnloadEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server.unload_world("other_end").await.unwrap();
        // The retired named world is outside the fixture's live-world collection.
        retired.level.world_portal.store(Arc::new(None));
        assert!(retired.level.cancel_token.is_cancelled());
        assert_eq!(
            hook.0.load(Ordering::Relaxed),
            1,
            "actual named unload callback did not finish"
        );
        let expected = clock(&retired);
        assert_eq!(
            expected,
            DimensionClock {
                total_ticks: 802,
                partial_tick: 0.25,
                rate: 0.75,
                paused: true
            }
        );
        assert!(
            !server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &retired))
        );
        let reloaded = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(
            clock(&reloaded),
            expected,
            "named root reload lost the final unload callback clock"
        );
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&server.basic_config.get_world_path().join("other_end")).clocks
                [Dimension::THE_END.default_clock.unwrap()],
            expected
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_fresh_overworld_does_not_inherit_main_clock_fallback() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[0], 100, 2.5, true);
        server.save_all().await.unwrap();
        let main = clock(&worlds[0]);
        assert_eq!(
            main,
            DimensionClock {
                total_ticks: 107,
                partial_tick: 0.5,
                rate: 2.5,
                paused: true
            }
        );
        let own = server.basic_config.get_world_path().join("fresh_overworld");
        assert!(!own.join("level.dat").exists());
        assert!(!own.join("data/minecraft/world_clocks.dat").exists());
        let named = server.create_world("fresh_overworld".into(), Dimension::OVERWORLD);
        assert_eq!(
            named.level_time.lock().unwrap().world_age,
            server.level_info.load().game_time
        );
        assert_eq!(
            clock(&named),
            DimensionClock {
                total_ticks: 0,
                partial_tick: 0.0,
                rate: 1.0,
                paused: false
            },
            "fresh named overworld inherited another root's clock"
        );
        assert_eq!(clock(&worlds[0]), main);
    })
    .await;
}

fn prepare_valid_legacy_root(path: &Path) {
    let mut data = LevelData::default(pumpkin_util::world_seed::Seed(777));
    data.game_time = 3333;
    AnvilLevelInfo.write_world_info(&data, path).unwrap();
    // Start from a complete supported file, retaining version/seed/generation fields.
    let level_path = path.join("level.dat");
    let mut root = read_gzip_compound_tag(fs::File::open(&level_path).unwrap()).unwrap();
    let mut data = root.get_compound("Data").unwrap().clone();
    data.put_long("DayTime", 8888);
    root.put_compound("Data", data);
    write_gzip_compound_tag(root, fs::File::create(&level_path).unwrap()).unwrap();
    fs::remove_file(path.join("data/minecraft/world_clocks.dat")).unwrap();
    let valid = AnvilLevelInfo.read_world_info(path).unwrap();
    assert_eq!(
        valid.day_time, 8888,
        "own legacy fixture was not a supported readable level.dat"
    );
    assert_eq!(valid.world_gen_settings.seed, 777);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_imported_overworld_migrates_its_own_valid_legacy_clock() {
    let directory = tempfile::tempdir().unwrap();
    prepare_valid_legacy_root(&directory.path().join("legacy_overworld"));
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[0], 100, 2.5, true);
        server.save_all().await.unwrap();
        let main = clock(&worlds[0]);
        let named = server.create_world("legacy_overworld".into(), Dimension::OVERWORLD);
        assert_eq!(
            named.level_time.lock().unwrap().world_age,
            server.level_info.load().game_time
        );
        assert_eq!(
            clock(&named),
            DimensionClock {
                total_ticks: 8888,
                partial_tick: 0.0,
                rate: 1.0,
                paused: false
            },
            "imported named overworld ignored its own valid legacy clock"
        );
        assert_eq!(clock(&worlds[0]), main);
        server.save_all().await.unwrap();
        let own = AnvilLevelInfo
            .read_world_info(
                &server
                    .basic_config
                    .get_world_path()
                    .join("legacy_overworld"),
            )
            .unwrap();
        assert_eq!(
            own.world_gen_settings.seed, 777,
            "clock save overwrote named generation metadata"
        );
    })
    .await;
}

struct CountFullMetadata(Mutex<Vec<(PathBuf, bool)>>);

impl WorldInfoWriter for CountFullMetadata {
    fn write_world_info(&self, info: &LevelData, path: &Path) -> Result<(), WorldInfoError> {
        let result = AnvilLevelInfo.write_world_info(info, path);
        self.0
            .lock()
            .unwrap()
            .push((path.to_path_buf(), result.is_ok()));
        result
    }
}

fn autosave_boundary(world: &Arc<World>) {
    assert!(world.level.autosave_ticks > 0 && world.level.save_enabled.load(Ordering::Relaxed));
    world.level_time.lock().unwrap().world_age =
        i64::try_from(world.level.autosave_ticks).unwrap() - 1;
    world.tick_environment();
}

async fn check_autosave_targets(path: &Path, include_main: bool) {
    let counter = Arc::new(CountFullMetadata(Mutex::new(Vec::new())));
    with_worlds(path, Some(counter.clone()), |server, worlds| async move {
        set_clock(&server, &worlds[0], 100, 2.5, true);
        let named = server.create_world("autosave_overworld".into(), Dimension::OVERWORLD);
        set_clock(&server, &named, 1500, 2.5, true);
        let expected = clock(&named);
        if include_main {
            autosave_boundary(&worlds[0]);
        }
        autosave_boundary(&named);
        // schedule_world_info_save and RNG saves use this existing real task tracker.
        // Await completed scheduled writers; shutdown's final metadata write occurs later.
        server.tasks.close();
        server.tasks.wait().await;
        let root = server.basic_config.get_world_path();
        let writes = counter.0.lock().unwrap().clone();
        assert!(
            writes.iter().all(|(_, succeeded)| *succeeded),
            "actual autosave metadata writer failed"
        );
        assert!(
            writes.iter().all(|(path, _)| path == &root),
            "autosave used a full metadata writer for another root"
        );
        assert_eq!(
            writes.len(),
            usize::from(include_main),
            "named overworld autosave queued redundant canonical full metadata writes"
        );
        let saved = read_world_clocks(&root.join("autosave_overworld"));
        assert_eq!(
            saved
                .clocks
                .get(Dimension::OVERWORLD.default_clock.unwrap()),
            Some(&expected),
            "completed named autosave omitted its own clock document"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_mixed_autosave_writes_main_full_metadata_once() {
    let directory = tempfile::tempdir().unwrap();
    check_autosave_targets(directory.path(), true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_root_only_autosave_avoids_main_full_metadata() {
    let directory = tempfile::tempdir().unwrap();
    check_autosave_targets(directory.path(), false).await;
}

mod retention;
