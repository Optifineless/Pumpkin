use super::{lifecycle::LogBuffer, *};
use pumpkin_world::world_info::{
    MAXIMUM_SUPPORTED_WORLD_DATA_VERSION, data_files::write_saved_data,
};

fn open_with_logs(path: &Path) -> (Arc<Server>, LogBuffer) {
    let logs = LogBuffer(Arc::default());
    let captured = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || captured.clone())
        .finish();
    let server =
        tracing::subscriber::with_default(subscriber, || combat_test_support::server(path));
    let world = combat_test_support::world(&server, path);
    server.worlds.store(Arc::new(vec![world]));
    (server, logs)
}

fn write_version(path: &Path, version: i32) -> Vec<u8> {
    let mut root = vanilla::fixture();
    root.put_int("DataVersion", version);
    let destination = path.join("data/minecraft/scoreboard.dat");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    write_saved_data(&destination, root).unwrap();
    fs::read(destination).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_older_version_loads_known_fields_and_warns_once() {
    let directory = tempfile::tempdir().unwrap();
    let older = MAXIMUM_SUPPORTED_WORLD_DATA_VERSION - 1;
    write_version(directory.path(), older);
    let (server, logs) = open_with_logs(directory.path());
    let world = server.worlds.load()[0].clone();
    assert_eq!(
        command(
            &server,
            &world,
            "scoreboard players get ImportedHolder imported"
        ),
        42
    );
    command(&server, &world, "scoreboard objectives add changed dummy");
    save(&server).await;
    assert_eq!(
        read_scoreboard(directory.path()).get_int("DataVersion"),
        Some(MAXIMUM_SUPPORTED_WORLD_DATA_VERSION)
    );
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert_eq!(
        log.lines()
            .filter(|line| line.to_ascii_lowercase().contains("scoreboard")
                && line.contains(&older.to_string()))
            .count(),
        1,
        "Expected one warning naming the older version: {log}"
    );
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_future_version_remains_untouched_after_save() {
    let directory = tempfile::tempdir().unwrap();
    let future = MAXIMUM_SUPPORTED_WORLD_DATA_VERSION + 1;
    let original = write_version(directory.path(), future);
    let (server, logs) = open_with_logs(directory.path());
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard objectives add changed dummy");
    let _ = tokio::time::timeout(Duration::from_secs(30), server.save_all())
        .await
        .expect("Real scoreboard save timed out");
    assert_eq!(
        fs::read(directory.path().join("data/minecraft/scoreboard.dat")).unwrap(),
        original
    );
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains(&future.to_string()) && log.to_ascii_lowercase().contains("scoreboard"),
        "Future scoreboard version was not reported: {log}"
    );
    server.shutdown().await;
}
