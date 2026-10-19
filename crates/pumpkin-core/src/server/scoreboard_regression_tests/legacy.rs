use super::{saved_components::*, *};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_world::world_info::data_files::write_saved_data;

fn legacy_fixture(path: &Path) -> Vec<u8> {
    // Pre-26.1 locations use the same direct-NBT component fields (1.21.9 / 4435).
    let mut root = vanilla::fixture();
    root.put_int("DataVersion", 4435);
    let destination = path.join("data/scoreboard.dat");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    write_saved_data(&destination, root).unwrap();
    fs::read(destination).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_legacy_pre_26_1_imports_saves_and_reloads() {
    if !isolated_child("scoreboard_regression_legacy_pre_26_1_imports_saves_and_reloads") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let original = legacy_fixture(directory.path());
    let (server, _) = startup(directory.path()).await;
    let world = server.worlds.load()[0].clone();
    assert_eq!(
        command(
            &server,
            &world,
            "scoreboard players get ImportedHolder imported"
        ),
        42
    );
    assert!(
        server
            .scoreboard
            .lock()
            .unwrap()
            .get_team("imported_red")
            .is_some()
    );
    let packets = joining_packets(&server).await;
    assert_eq!(objective_components(&packets).len(), 2);
    command(
        &server,
        &world,
        "scoreboard players set ImportedHolder imported 43",
    );
    save(&server).await;
    assert_eq!(
        fs::read(directory.path().join("data/scoreboard.dat")).unwrap(),
        original
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
    let (state, _) = super::super::scoreboard_storage::load_scoreboard(directory.path());
    assert_eq!(
        state
            .lock()
            .unwrap()
            .get_score_value("ImportedHolder", "imported"),
        Some(43)
    );
    assert!(
        directory
            .path()
            .join("data/minecraft/scoreboard.dat")
            .is_file()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_legacy_corrupt_structure_disables_saves() {
    if !isolated_child("scoreboard_regression_legacy_corrupt_structure_disables_saves") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    legacy_fixture(directory.path());
    let path = directory.path().join("data/scoreboard.dat");
    let mut root = vanilla::fixture();
    let NbtTag::Compound(data) = root.child_tags.get_mut("data").unwrap() else {
        panic!("Fixture data must be a compound")
    };
    data.put_list("Objectives", vec![NbtTag::List(Vec::new())]);
    write_saved_data(&path, root).unwrap();
    let original = fs::read(&path).unwrap();
    let (server, logs) = startup(directory.path()).await;
    assert!(
        server
            .scoreboard
            .lock()
            .unwrap()
            .get_objectives()
            .is_empty()
    );
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard objectives add fresh dummy");
    assert!(
        tokio::time::timeout(Duration::from_secs(30), server.save_all())
            .await
            .unwrap()
            .is_err()
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(
        !directory
            .path()
            .join("data/minecraft/scoreboard.dat")
            .exists()
    );
    assert!(
        String::from_utf8(logs.0.lock().unwrap().clone())
            .unwrap()
            .contains("scoreboard saves disabled")
    );
}
