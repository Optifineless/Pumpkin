use super::*;
use crossbeam::channel::bounded;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_invalidated_lazy_faces_are_not_consumed() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let entity = fixture.world.get_block_entity(&position).unwrap();
    let mut saved = NbtCompound::new();
    entity.write_internal(&mut saved);
    fixture.world.remove_block_entity(&position);
    fixture.world.add_block_entity_nbt(position, &saved);
    let (entered, wait_for_entry) = bounded(1);
    let (resume, wait_for_resume) = bounded(1);
    let calls = AtomicUsize::new(0);
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = Some(Arc::new(move || {
        if calls.fetch_add(1, Relaxed) == 0 {
            entered.send(()).unwrap();
            wait_for_resume
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
    }));
    let finished = super::overlap::hit_on_worker(arrow, position);
    wait_for_entry.recv_timeout(Duration::from_secs(5)).unwrap();
    // The supported native/Wasm NBT setter invalidates even an identical pending image.
    fixture.world.add_block_entity_nbt(position, &saved);
    resume.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = None;
    if fixture.world.get_block(&position) == &Block::DECORATED_POT {
        let pending = fixture
            .world
            .level
            .read_chunk_sync(&position.chunk_position(), |chunk| {
                chunk
                    .pending_block_entities
                    .lock()
                    .unwrap()
                    .get(&position)
                    .cloned()
            })
            .flatten()
            .unwrap();
        assert_eq!(
            pending, saved,
            "denied removal leaves the pending image unchanged"
        );
        assert_retained(&fixture, position);
        let current = fixture.world.get_block_entity(&position).unwrap();
        let mut after = NbtCompound::new();
        current.write_internal(&mut after);
        // Promotion may add inert raw-source backups; the actual faces and contents stay exact.
        assert_eq!(after.get("sherds"), saved.get("sherds"));
        assert_eq!(after.get("item"), saved.get("item"));
    } else {
        assert_shattered(&fixture, position);
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_capture_preserves_a_replacement_bed() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let head = position.offset(Vector3::new(1, 0, 0));
    fixture.set_block(head.down(), &Block::STONE);
    let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let mut saved = NbtCompound::new();
    fixture
        .world
        .get_block_entity(&position)
        .unwrap()
        .write_internal(&mut saved);
    fixture.world.remove_block_entity(&position);
    fixture.world.add_block_entity_nbt(position, &saved);
    let (entered, wait_for_entry) = bounded(1);
    let (resume, wait_for_resume) = bounded(1);
    let calls = AtomicUsize::new(0);
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = Some(Arc::new(move || {
        if calls.fetch_add(1, Relaxed) == 0 {
            entered.send(()).unwrap();
            wait_for_resume
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
    }));
    let finished = super::overlap::hit_on_worker(arrow, position);
    wait_for_entry.recv_timeout(Duration::from_secs(5)).unwrap();
    let bed_state = |part| {
        Block::RED_BED
            .from_properties(&[("facing", "east"), ("part", part)])
            .to_state_id(&Block::RED_BED)
    };
    let foot_state = bed_state("foot");
    let head_state = bed_state("head");
    for (position, state) in [(position, foot_state), (head, head_state)] {
        fixture.world.set_block_state(
            &position,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
    let bed_entity = fixture.world.get_block_entity(&position).unwrap();
    resume.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = None;
    assert_eq!(fixture.world.get_block_state_id(&position), foot_state);
    assert_eq!(fixture.world.get_block_state_id(&head), head_state);
    assert!(Arc::ptr_eq(
        &bed_entity,
        &fixture.world.get_block_entity(&position).unwrap()
    ));
    assert_eq!(item_count(&fixture.world, &Item::RED_BED), 0);
    fixture.shutdown().await;
}
