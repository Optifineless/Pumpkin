use super::*;
use crate::server::combat_test_support;
use std::{fs, sync::Barrier};

const MAP: &[u8] = include_bytes!("../../../tests/fixtures/maps/0.dat");
const INDEX: &[u8] = include_bytes!("../../../tests/fixtures/maps/last_id.dat");

fn import(path: &std::path::Path, legacy: bool) {
    let folder = path.join(if legacy {
        "data"
    } else {
        "data/minecraft/maps"
    });
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(if legacy { "map_0.dat" } else { "0.dat" }), MAP).unwrap();
    fs::write(
        folder.join(if legacy {
            "idcounts.dat"
        } else {
            "last_id.dat"
        }),
        INDEX,
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vanilla_map_index_survives_real_server_restart_without_replacing_map_zero() {
    for legacy in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        import(dir.path(), legacy);
        let server = combat_test_support::server(dir.path());
        assert!(server.map_manager.maps.is_empty());
        let imported = server.map_manager.load_map(0).await.unwrap();
        let first = server.next_map_id();
        assert_eq!(first, 5);
        let _ = server
            .map_manager
            .create_map(first, Dimension::OVERWORLD, 64, 64, 1);
        // Autosave persists MapIndex without relying on a level.dat write.
        server.save_maps().await.unwrap();
        assert_eq!(imported.lock().unwrap().colors[0], 231);
        server.shutdown().await;
        drop(imported);
        drop(server);
        // Construct a different real Server, with a new session lock and disk-backed services.
        let restarted = combat_test_support::server(dir.path());
        let second = restarted.next_map_id();
        assert_eq!(second, 6);
        assert_ne!(first, second);
        assert_eq!(
            restarted
                .map_manager
                .load_map(0)
                .await
                .unwrap()
                .lock()
                .unwrap()
                .colors[0],
            231
        );
        assert_eq!(
            restarted
                .map_manager
                .load_map(first)
                .await
                .unwrap()
                .lock()
                .unwrap()
                .scale,
            1
        );
        // Reconcile an older Pumpkin level.dat whose next-ID counter is ahead of MapIndex.
        restarted.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.map_id = 20;
            info
        });
        restarted.shutdown().await;
        drop(restarted);
        let reconciled = combat_test_support::server(dir.path());
        assert_eq!(reconciled.next_map_id(), 20);
        reconciled.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archived_maximum_map_id_is_not_reused_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("data/minecraft/maps");
    fs::create_dir_all(&folder).unwrap();
    let archived = folder.join(format!("{}.dat", i32::MAX));
    fs::write(&archived, MAP).unwrap();
    let manager = MapManager::load(dir.path()).unwrap();
    // MapIndex.getNextMapId increments Java int MAX_VALUE to MIN_VALUE.
    let first = manager.next_map_id();
    assert_eq!(first, i32::MIN);
    manager
        .create_map(first, Dimension::OVERWORLD, 0, 0, 0)
        .lock()
        .unwrap()
        .set_color(0, 0, 17);
    manager.save(dir.path(), 5000).await.unwrap();
    drop(manager);
    let restarted = MapManager::load(dir.path()).unwrap();
    assert_eq!(restarted.next_map_id(), i32::MIN + 1);
    assert!(restarted.get_map(first).is_none());
    restarted.drain().await;
    assert_eq!(
        restarted.get_map(first).unwrap().lock().unwrap().colors[0],
        17
    );
    assert_eq!(fs::read(archived).unwrap(), MAP);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn map_cache_misses_load_and_unchanged_maps_are_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    import(dir.path(), false);
    let manager = MapManager::load(dir.path()).unwrap();
    assert!(manager.maps.is_empty());
    assert!(manager.get_map(0).is_none());
    manager.drain().await;
    assert!(manager.get_map(0).is_some());
    let path = dir.path().join("data/minecraft/maps/0.dat");
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    manager.save(dir.path(), 5000).await.unwrap();
    assert_eq!(fs::read(&path).unwrap(), MAP);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn map_mutation_during_save_remains_dirty_for_the_next_save() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(MapManager::load(dir.path()).unwrap());
    let map = manager.create_map(0, Dimension::OVERWORLD, 0, 0, 0);
    map.lock().unwrap().set_color(0, 0, 17);
    let barrier = Arc::new(Barrier::new(2));
    *manager.save_barrier.lock().unwrap() = Some(barrier.clone());
    let saving = manager.clone();
    let path = dir.path().to_owned();
    let save = tokio::spawn(async move {
        saving.save(&path, 5000).await.unwrap();
    });
    let changing = map.clone();
    tokio::task::spawn_blocking(move || {
        barrier.wait();
        changing.lock().unwrap().set_color(0, 0, 231);
        barrier.wait();
    })
    .await
    .unwrap();
    save.await.unwrap();
    assert_eq!(
        storage::read_map(&dir.path().join("data/minecraft/maps/0.dat"))
            .unwrap()
            .colors[0],
        17
    );
    map.lock().unwrap().dirty = false; // Player.tick_maps clears only the packet flag.
    manager.save(dir.path(), 5000).await.unwrap();
    assert_eq!(
        storage::read_map(&dir.path().join("data/minecraft/maps/0.dat"))
            .unwrap()
            .colors[0],
        231
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_drains_requested_legacy_maps_and_migrates_them() {
    let dir = tempfile::tempdir().unwrap();
    import(dir.path(), true);
    let server = combat_test_support::server(dir.path());
    assert!(server.map_manager.get_map(0).is_none());
    server.shutdown().await;
    assert!(dir.path().join("data/minecraft/maps/0.dat").exists());
    assert!(dir.path().join("data/minecraft/maps/last_id.dat").exists());
    let imported = server.map_manager.get_map(0).unwrap();
    assert_eq!(imported.lock().unwrap().colors[0], 231);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_in_flight_map_writes_and_saves_later_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let map = server
        .map_manager
        .create_map(0, Dimension::OVERWORLD, 0, 0, 0);
    map.lock().unwrap().set_color(0, 0, 17);
    let barrier = Arc::new(Barrier::new(2));
    *server.map_manager.save_barrier.lock().unwrap() = Some(barrier.clone());
    let saving = server.clone();
    let save = tokio::spawn(async move {
        saving.save_maps().await.unwrap();
    });
    let changing = map.clone();
    let (ready_sender, ready) = tokio::sync::oneshot::channel();
    let (release_sender, released) = std::sync::mpsc::channel();
    let release = tokio::task::spawn_blocking(move || {
        barrier.wait();
        ready_sender.send(()).unwrap();
        released.recv().unwrap();
        changing.lock().unwrap().set_color(0, 0, 231);
        barrier.wait();
    });
    ready.await.unwrap();
    let stopping = server.clone();
    let stopped = tokio::spawn(async move {
        stopping.shutdown().await;
    });
    tokio::task::yield_now().await;
    assert!(!stopped.is_finished());
    release_sender.send(()).unwrap();
    stopped.await.unwrap();
    save.await.unwrap();
    release.await.unwrap();
    assert_eq!(
        storage::read_map(&dir.path().join("data/minecraft/maps/0.dat"))
            .unwrap()
            .colors[0],
        231
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_banner_and_frame_markers_rebuild_named_packet_decorations() {
    use pumpkin_data::map_decoration::MapDecorationType;
    use pumpkin_protocol::{VarInt, java::client::play::MapIcon};
    use pumpkin_util::version::JavaMinecraftVersion;
    let dir = tempfile::tempdir().unwrap();
    import(dir.path(), false);
    let manager = MapManager::load(dir.path()).unwrap();
    let map = manager.load_map(0).await.unwrap();
    let map = map.lock().unwrap();
    assert_eq!(map.decorations.len(), 2);
    let banner = &map.decorations[0];
    assert_eq!(
        (banner.icon_type, banner.x, banner.z, banner.direction),
        (MapDecorationType::BANNER_RED.id as i32, 20, -19, 8)
    );
    assert_eq!(
        banner.display_name,
        Some(pumpkin_util::text::TextComponent::text("Home").bold())
    );
    let frame = &map.decorations[1];
    assert_eq!(
        (frame.icon_type, frame.x, frame.z, frame.direction),
        (MapDecorationType::FRAME.id as i32, -23, 28, 4)
    );
    let mut bytes = Vec::new();
    MapIcon::new(
        VarInt(frame.icon_type),
        frame.x,
        frame.z,
        frame.direction,
        frame.display_name.clone(),
    )
    .write_with_version(&mut bytes, &JavaMinecraftVersion::V_26_3)
    .unwrap();
    assert_eq!(bytes, [1, 233, 28, 4, 0]);
}
