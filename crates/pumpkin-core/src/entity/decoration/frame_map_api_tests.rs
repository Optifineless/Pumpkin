use super::{
    DEADLINE, Fixture, ItemFrameEntity, MapManager, attack, import, loaded, markers, persisted,
    place, stack,
};
use crate::entity::Entity;
use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_util::math::vector3::Vector3;
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

fn temporary_frame(fixture: &Fixture, attached: bool) -> ItemFrameEntity {
    let frame = ItemFrameEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(10.5, 65.5, 9.5),
        &EntityType::ITEM_FRAME,
    ));
    if attached {
        frame.set_item(stack(0), false);
    }
    frame
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_loader_and_frame_drop_complete_under_map_guard() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let frame = place(&fixture, 0);
    let attached = temporary_frame(&fixture, true);
    let unattached = temporary_frame(&fixture, false);
    let (published, resume, finished) = fixture
        .server
        .map_manager
        .pause_next_map_publication_for_test();
    attack(&fixture, frame.as_ref());
    published.recv_timeout(Duration::from_secs(5)).unwrap();
    // The real loader published this Arc; no test inserts a replacement cache entry.
    let map = fixture.server.map_manager.get_map(0).unwrap();
    let held_map = map.clone();
    let (send, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut held = held_map.lock().unwrap();
        held.dirty = false;
        drop(unattached);
        drop(attached);
        resume.send(()).unwrap();
        let completed = finished.recv_timeout(Duration::from_secs(5));
        assert_eq!(held.frames.len(), 2);
        assert!(!held.dirty);
        drop(held);
        send.send(completed).unwrap();
    });
    let completed = done.recv_timeout(Duration::from_secs(7)).unwrap();
    worker.join().unwrap();
    completed.unwrap();
    // Exercise the actual next-tick retry, then the production save/reload path.
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    fixture.server.tick();
    assert!(map.lock().unwrap().dirty);
    markers(&map, true);
    persisted(&fixture, true).await;
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_frame_drop_never_waits_for_cleanup_owner() {
    let fixture = Fixture::new();
    let map = fixture
        .server
        .map_manager
        .create_map(0, Dimension::OVERWORLD, 0, 0, 0);
    for attached in [false, true] {
        let frame = temporary_frame(&fixture, attached);
        let server = fixture.server.clone();
        let (entered, owner_ready) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let owner = std::thread::spawn(move || {
            server
                .map_manager
                .hold_frame_cleanup_for_test(&entered, &resume);
        });
        owner_ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let held_map = map.clone();
        let (send, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _held = held_map.lock().unwrap();
            drop(frame);
            send.send(()).unwrap();
        });
        let completed = done.recv_timeout(Duration::from_secs(5));
        // Release C before reporting a regression so runtime teardown can finish.
        release.send(()).unwrap();
        owner.join().unwrap();
        if completed.is_err() {
            done.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        worker.join().unwrap();
        completed.unwrap();
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_position_only_nbt_edit_retires_original_marker() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    let source = crate::command::CommandSender::Console.into_source(&fixture.server);
    map.lock().unwrap().dirty = false;
    assert_eq!(
        fixture
            .server
            .command_dispatcher
            .load()
            .execute_input(
                &format!(
                    "data merge entity {} {{Pos:[10.5d,65.5d,9.5d]}}",
                    frame.get_entity().entity_uuid
                ),
                &source,
            )
            .unwrap(),
        1
    );
    attack(&fixture, frame.as_ref());
    assert!(map.lock().unwrap().dirty);
    markers(&map, true);
    persisted(&fixture, true).await;
    fixture.finish().await;
}
