use super::*;
#[cfg(unix)]
use crate::plugin::api::events::world::world_init::WorldInitEvent;
use crate::plugin::api::events::world::world_save::WorldSaveEvent;

fn clock_destination(root: &Path) -> PathBuf {
    root.join("data/minecraft/world_clocks.dat")
}

fn block_clock(root: &Path) -> (PathBuf, PathBuf) {
    let destination = clock_destination(root);
    let backup = root.join("clock-before-test-fault.dat");
    fs::rename(&destination, &backup).unwrap();
    fs::create_dir(&destination).unwrap();
    (destination, backup)
}

fn restore_clock((destination, backup): (PathBuf, PathBuf)) {
    fs::remove_dir(&destination).unwrap();
    fs::rename(backup, destination).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_failed_unload_retains_final_callback_for_reload_and_retry() {
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
        let root = server.basic_config.get_world_path().join("other_end");
        let blocked = block_clock(&root);
        let result = server.unload_world("other_end").await;
        retired.level.world_portal.store(Arc::new(None));
        restore_clock(blocked);
        let error = result.unwrap_err();
        assert!(error.contains("clock root") && error.contains("other_end"));
        assert_eq!(hook.0.load(Ordering::Relaxed), 1);
        assert!(retired.level.cancel_token.is_cancelled());
        assert!(
            !server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &retired))
        );
        let expected = clock(&retired);
        assert_eq!(expected.total_ticks, 802);
        assert_eq!(
            read_world_clocks(&root).clocks[Dimension::THE_END.default_clock.unwrap()],
            imported_clock()
        );
        let restored = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(
            clock(&restored),
            expected,
            "named reload ignored the retained failed-write clock"
        );
        server.save_all().await.unwrap();
        let saved = read_world_clocks(&root);
        assert_eq!(
            saved.clocks[Dimension::THE_END.default_clock.unwrap()],
            expected
        );
        assert_eq!(saved.clocks["test:named_unloaded"], imported_clock());
    })
    .await;
}

struct SaveCallbacks(AtomicUsize);

impl EventHandler<WorldSaveEvent> for SaveCallbacks {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _event: &'a mut WorldSaveEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert!(
                server.world_info_save_lock.try_lock().is_ok(),
                "world save callback held clock-save ownership"
            );
            self.0.fetch_add(1, Ordering::Relaxed);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_save_all_failure_attempts_other_roots_and_world_callbacks() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, worlds| async move {
        let broken = server.create_world("other_end".into(), Dimension::THE_END);
        let healthy = server.create_world("healthy_overworld".into(), Dimension::OVERWORLD);
        server.save_all().await.unwrap();
        set_clock(&server, &worlds[0], 400, 1.25, true);
        set_clock(&server, &broken, 500, 0.75, true);
        set_clock(&server, &healthy, 700, 1.25, true);
        let expected = clock(&healthy);
        let hook = Arc::new(SaveCallbacks(AtomicUsize::new(0)));
        server.plugin_manager.register::<WorldSaveEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        let root = server.basic_config.get_world_path();
        let main_fault = block_clock(&root);
        let named_fault = block_clock(&root.join("other_end"));
        let result = server.save_all().await;
        restore_clock(main_fault);
        restore_clock(named_fault);
        let error = result.unwrap_err();
        assert!(
            error.contains("world info")
                && error.contains("clock root")
                && error.contains("other_end"),
            "missing contextual root failure: {error}"
        );
        assert_eq!(
            hook.0.load(Ordering::Relaxed),
            5,
            "a failed clock root skipped later live-world save callbacks"
        );
        assert_eq!(
            read_world_clocks(&root.join("healthy_overworld")).clocks
                [Dimension::OVERWORLD.default_clock.unwrap()],
            expected
        );
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&root).clocks[Dimension::OVERWORLD.default_clock.unwrap()],
            clock(&worlds[0])
        );
        assert_eq!(
            read_world_clocks(&root.join("other_end")).clocks
                [Dimension::THE_END.default_clock.unwrap()],
            clock(&broken)
        );
    })
    .await;
}

struct ReplaceNamedClock {
    callbacks: AtomicUsize,
    replacement: Mutex<Option<Arc<World>>>,
}

struct NestedNamedUnload {
    callbacks: AtomicUsize,
    replacement: Mutex<Option<Arc<World>>>,
    newest_retired: Mutex<Option<DimensionClock>>,
}

impl EventHandler<WorldUnloadEvent> for NestedNamedUnload {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut WorldUnloadEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.world.get_world_name() != "other_end" {
                return;
            }
            assert!(server.world_info_save_lock.try_lock().is_ok());
            assert!(event.world.level_time.try_lock().is_ok());
            match self.callbacks.fetch_add(1, Ordering::Relaxed) {
                0 => {
                    set_clock(server, &event.world, 800, 0.75, true);
                    let middle = server.create_world("other_end".into(), Dimension::THE_END);
                    assert!(!Arc::ptr_eq(&middle, &event.world));
                    set_clock(server, &middle, 1200, 1.25, true);
                    server.unload_world("other_end").await.unwrap();
                    middle.level.world_portal.store(Arc::new(None));
                    assert!(
                        !server
                            .worlds
                            .load()
                            .iter()
                            .any(|world| Arc::ptr_eq(world, &middle))
                    );
                    let expected = self.newest_retired.lock().unwrap().clone().unwrap();
                    assert_eq!(expected.total_ticks, 1602);
                    let newest = server.create_world("other_end".into(), Dimension::THE_END);
                    assert_eq!(
                        clock(&newest),
                        expected,
                        "older pending unload overwrote the newer retired root clock"
                    );
                    set_clock(server, &newest, 1800, 0.5, true);
                    *self.replacement.lock().unwrap() = Some(newest);
                }
                1 => {
                    set_clock(server, &event.world, 1600, 0.75, true);
                    *self.newest_retired.lock().unwrap() = Some(clock(&event.world));
                }
                other => panic!("unexpected nested native unload callback {other}"),
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_nested_newer_retirement_survives_older_pending_callback() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, _| async move {
        let oldest = server.create_world("other_end".into(), Dimension::THE_END);
        let hook = Arc::new(NestedNamedUnload {
            callbacks: AtomicUsize::new(0),
            replacement: Mutex::new(None),
            newest_retired: Mutex::new(None),
        });
        server.plugin_manager.register::<WorldUnloadEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server.unload_world("other_end").await.unwrap();
        oldest.level.world_portal.store(Arc::new(None));
        assert_eq!(hook.callbacks.load(Ordering::Relaxed), 2);
        let newest = hook.replacement.lock().unwrap().clone().unwrap();
        assert!(
            !server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &oldest))
        );
        assert!(
            server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &newest))
        );
        let expected = clock(&newest);
        assert_eq!(
            expected,
            DimensionClock {
                total_ticks: 1801,
                partial_tick: 0.5,
                rate: 0.5,
                paused: true
            }
        );
        server.save_all().await.unwrap();
        let saved = read_world_clocks(&server.basic_config.get_world_path().join("other_end"));
        assert_eq!(
            saved.clocks[Dimension::THE_END.default_clock.unwrap()],
            expected
        );
        assert_eq!(saved.clocks["test:named_unloaded"], imported_clock());
    })
    .await;
}

impl EventHandler<WorldUnloadEvent> for ReplaceNamedClock {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut WorldUnloadEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.world.get_world_name() != "other_end" {
                return;
            }
            assert!(server.world_info_save_lock.try_lock().is_ok());
            assert!(event.world.level_time.try_lock().is_ok());
            set_clock(server, &event.world, 800, 0.75, true);
            let replacement = server.create_world("other_end".into(), Dimension::THE_END);
            assert!(
                !Arc::ptr_eq(&replacement, &event.world),
                "admission reused the cancelled retiring world"
            );
            assert_eq!(
                clock(&replacement),
                clock(&event.world),
                "replacement admission ignored the latest callback clock"
            );
            set_clock(server, &replacement, 1200, 1.25, true);
            *self.replacement.lock().unwrap() = Some(replacement);
            self.callbacks.fetch_add(1, Ordering::Relaxed);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_new_owner_wins_old_unload_completion_and_path_alias() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, _| async move {
        let retired = server.create_world("other_end".into(), Dimension::THE_END);
        let hook = Arc::new(ReplaceNamedClock {
            callbacks: AtomicUsize::new(0),
            replacement: Mutex::new(None),
        });
        server.plugin_manager.register::<WorldUnloadEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server.unload_world("other_end").await.unwrap();
        retired.level.world_portal.store(Arc::new(None));
        assert_eq!(hook.callbacks.load(Ordering::Relaxed), 1);
        let replacement = hook.replacement.lock().unwrap().clone().unwrap();
        assert!(
            server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &replacement))
        );
        assert!(
            !server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, &retired))
        );
        let expected = clock(&replacement);
        assert_eq!(
            expected,
            DimensionClock {
                total_ticks: 1203,
                partial_tick: 0.75,
                rate: 1.25,
                paused: true
            }
        );
        let root = server.basic_config.get_world_path().join("other_end");
        assert_eq!(
            read_world_clocks(&root).clocks[Dimension::THE_END.default_clock.unwrap()],
            expected
        );
        let relative_alias = server.create_world("aliases/../other_end".into(), Dimension::THE_END);
        let absolute_alias =
            server.create_world(root.to_string_lossy().into_owned(), Dimension::THE_END);
        assert!(
            Arc::ptr_eq(&relative_alias, &replacement)
                && Arc::ptr_eq(&absolute_alias, &replacement)
        );
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&root).clocks[Dimension::THE_END.default_clock.unwrap()],
            expected
        );
        assert_eq!(
            read_world_clocks(&root).clocks["test:named_unloaded"],
            imported_clock()
        );
    })
    .await;
}

#[cfg(unix)]
struct InitCallbacks(AtomicUsize);

#[cfg(unix)]
impl EventHandler<WorldInitEvent> for InitCallbacks {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut WorldInitEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert!(server.world_info_save_lock.try_lock().is_ok());
            assert!(event.world.level_time.try_lock().is_ok());
            self.0.fetch_add(1, Ordering::Relaxed);
        })
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_physical_symlink_alias_reuses_one_published_owner() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    with_worlds(directory.path(), None, |server, worlds| async move {
        let hook = Arc::new(InitCallbacks(AtomicUsize::new(0)));
        server.plugin_manager.register::<WorldInitEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        let named = server.create_world("other_end".into(), Dimension::THE_END);
        let root = server.basic_config.get_world_path();
        std::os::unix::fs::symlink(root.join("other_end"), root.join("end_alias")).unwrap();
        let alias = server.create_world("end_alias".into(), Dimension::THE_END);
        assert!(Arc::ptr_eq(&named, &alias));
        assert_eq!(
            hook.0.load(Ordering::Relaxed),
            1,
            "physical alias published a second named owner"
        );
        std::os::unix::fs::symlink(&root, root.join("main_alias")).unwrap();
        let main_alias = server.create_world("main_alias".into(), Dimension::OVERWORLD);
        assert!(!main_alias.saves_default_clock);
        set_clock(&server, &main_alias, 9000, 3.5, false);
        set_clock(&server, &worlds[0], 100, 2.5, true);
        set_clock(&server, &named, 700, 0.75, true);
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&root).clocks[Dimension::OVERWORLD.default_clock.unwrap()],
            clock(&worlds[0])
        );
        assert_eq!(
            read_world_clocks(&root.join("other_end")).clocks
                [Dimension::THE_END.default_clock.unwrap()],
            clock(&named)
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_missing_legacy_daytime_uses_own_time_and_preserves_seed() {
    let directory = tempfile::tempdir().unwrap();
    let own_root = directory.path().join("legacy_overworld");
    prepare_valid_legacy_root(&own_root);
    let path = own_root.join("level.dat");
    let mut root = read_gzip_compound_tag(fs::File::open(&path).unwrap()).unwrap();
    let mut data = root.get_compound("Data").unwrap().clone();
    assert!(data.child_tags.remove("DayTime").is_some());
    root.put_compound("Data", data);
    write_gzip_compound_tag(root, fs::File::create(&path).unwrap()).unwrap();
    assert_eq!(
        AnvilLevelInfo.read_world_info(&own_root).unwrap().day_time,
        3333
    );
    with_worlds(directory.path(), None, |server, worlds| async move {
        set_clock(&server, &worlds[0], 100, 2.5, true);
        server.save_all().await.unwrap();
        let named = server.create_world("legacy_overworld".into(), Dimension::OVERWORLD);
        assert_eq!(
            clock(&named),
            DimensionClock {
                total_ticks: 3333,
                partial_tick: 0.0,
                rate: 1.0,
                paused: false
            }
        );
        assert_eq!(
            named.level_time.lock().unwrap().world_age,
            server.level_info.load().game_time
        );
        server.save_all().await.unwrap();
        assert_eq!(
            AnvilLevelInfo
                .read_world_info(
                    &server
                        .basic_config
                        .get_world_path()
                        .join("legacy_overworld")
                )
                .unwrap()
                .world_gen_settings
                .seed,
            777
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_clock_shared_root_keeps_other_registered_and_unloaded_records() {
    let directory = tempfile::tempdir().unwrap();
    prepare_named_end(directory.path());
    let root = directory.path().join("other_end");
    let mut imported = read_world_clocks(&root);
    imported
        .clocks
        .insert("minecraft:the_nether".into(), imported_clock());
    imported
        .clocks
        .insert("minecraft:overworld_caves".into(), imported_clock());
    write_world_clocks(&root, &imported).unwrap();
    with_worlds(directory.path(), None, |server, _| async move {
        assert_ne!(
            Dimension::THE_END.default_clock,
            Dimension::OVERWORLD.default_clock
        );
        let end = server.create_world("other_end".into(), Dimension::THE_END);
        let overworld = server.create_world("other_end".into(), Dimension::OVERWORLD);
        set_clock(&server, &end, 700, 0.75, true);
        set_clock(&server, &overworld, 800, 1.25, true);
        let expected_end = clock(&end);
        let expected_overworld = clock(&overworld);
        server.save_all().await.unwrap();
        let root = server.basic_config.get_world_path().join("other_end");
        let saved = read_world_clocks(&root);
        assert_eq!(
            saved.clocks[Dimension::THE_END.default_clock.unwrap()],
            expected_end
        );
        assert_eq!(
            saved.clocks[Dimension::OVERWORLD.default_clock.unwrap()],
            expected_overworld
        );
        assert_eq!(saved.clocks["test:named_unloaded"], imported_clock());
        assert!(
            !saved.clocks.contains_key("minecraft:the_nether")
                && !saved.clocks.contains_key("minecraft:overworld_caves")
        );
        assert!(
            !root.join("level.dat").exists(),
            "clock-only save cloned main full metadata into named root"
        );
        server.unload_world("other_end").await.unwrap();
        end.level.world_portal.store(Arc::new(None));
        let reloaded = server.create_world("other_end".into(), Dimension::THE_END);
        assert_eq!(clock(&reloaded), expected_end);
        assert_eq!(clock(&overworld), expected_overworld);
        server.save_all().await.unwrap();
        assert_eq!(
            read_world_clocks(&root).clocks[Dimension::OVERWORLD.default_clock.unwrap()],
            expected_overworld
        );
    })
    .await;
}
