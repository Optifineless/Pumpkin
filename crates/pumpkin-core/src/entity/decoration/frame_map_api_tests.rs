use super::{DEADLINE, Fixture, MapManager, attack, import, loaded, markers, persisted, place};
use pumpkin_data::dimension::Dimension;
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

#[test]
fn b1020b_g04_existing_id_create_returns_while_caller_holds_map_mutex() {
    let manager = MapManager::new();
    let _ = manager.create_map(7, Dimension::OVERWORLD, 64, -64, 1);
    let (send, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let map = manager.get_map(7).unwrap();
        let mut held = map.lock().unwrap();
        held.colors[0] = 17;
        held.dirty = false;
        let existing = manager.create_map(
            7,
            held.dimension.clone(),
            held.center_x,
            held.center_z,
            held.scale,
        );
        assert!(Arc::ptr_eq(&map, &existing));
        assert_eq!(held.colors[0], 17);
        assert!(!held.dirty);
        drop(held);
        send.send(()).unwrap();
    });
    // On regression the isolated worker self-deadlocks; never join it before completion.
    done.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_busy_existing_map_preserves_deferred_frame_cleanup() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    fixture.server.map_manager.maps.remove(&0).unwrap();
    let (entered, release) = fixture.server.map_manager.pause_next_load_for_test();
    attack(&fixture, frame.as_ref());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(fixture.drops(&pumpkin_data::item::Item::FILLED_MAP), 1);
    fixture.server.map_manager.maps.insert(0, map.clone());
    let (worker, returned) = {
        let mut held = map.lock().unwrap();
        held.dirty = false;
        let server = fixture.server.clone();
        let (send, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let existing = server
                .map_manager
                .create_map(0, Dimension::THE_NETHER, 128, 128, 4);
            send.send(existing).unwrap();
        });
        let returned = done.recv_timeout(Duration::from_secs(5));
        assert_eq!(held.frames.len(), 2);
        assert!(!held.dirty);
        (worker, returned)
    };
    release.send(()).unwrap();
    let returned = returned.unwrap();
    worker.join().unwrap();
    assert!(Arc::ptr_eq(&map, &returned));
    tokio::time::timeout(DEADLINE, fixture.server.map_manager.drain())
        .await
        .unwrap();
    assert!(map.lock().unwrap().dirty);
    markers(&map, true);
    persisted(&fixture, true).await;
    fixture.finish().await;
}
