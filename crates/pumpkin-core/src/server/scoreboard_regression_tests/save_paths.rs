use super::*;

fn saved_value(path: &Path) -> i32 {
    payload(&read_scoreboard(path))
        .get_list("PlayerScores")
        .unwrap()
        .iter()
        .filter_map(pumpkin_nbt::tag::NbtTag::extract_compound)
        .find(|score| score.get_string("Name") == Some("Holder"))
        .unwrap()
        .get_int("Score")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_autosave_and_shutdown_use_the_live_shared_owner() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        command(&server, &worlds[1], "scoreboard objectives add path dummy");
        command(&server, &worlds[1], "scoreboard players set Holder path 10");
        worlds[0].level_time.lock().unwrap().world_age =
            i64::try_from(worlds[0].level.autosave_ticks).unwrap() - 1;
        worlds[0].tick_environment();
        tokio::time::timeout(
            Duration::from_secs(30),
            combat_test_support::wait_for_tasks(&server),
        )
        .await
        .expect("Autosave tasks did not finish");
        // Inspect before final shutdown can mask a missing autosave hook.
        assert_eq!(saved_value(&server.basic_config.get_world_path()), 10);
        command(&server, &worlds[2], "scoreboard players set Holder path 20");
    })
    .await;
    // with_worlds calls the production shutdown path after its body returns.
    assert_eq!(saved_value(directory.path()), 20);
    with_worlds(directory.path(), None, |server, worlds| async move {
        assert_eq!(
            command(&server, &worlds[1], "scoreboard players get Holder path"),
            20
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_failed_save_keeps_dirty_state_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        command(&server, &worlds[0], "scoreboard objectives add retry dummy");
        command(
            &server,
            &worlds[2],
            "scoreboard players set Holder retry 41",
        );
        let path = server.basic_config.get_world_path();
        let obstruction = path.join("data/minecraft/scoreboard.dat");
        fs::create_dir_all(&obstruction).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(30), server.save_all())
                .await
                .expect("Failed save did not finish")
                .is_err()
        );
        assert!(server.scoreboard.lock().unwrap().is_dirty());
        fs::remove_dir(obstruction).unwrap();
        save(&server).await;
        assert!(!server.scoreboard.lock().unwrap().is_dirty());
        assert_eq!(saved_value(&path), 41);
    })
    .await;
}
