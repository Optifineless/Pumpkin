use super::*;
use std::cell::RefCell;
use std::sync::atomic::AtomicBool;

thread_local! {
    static QUEUED: RefCell<Option<GuardEffects>> = const { RefCell::new(None) };
}

fn drain_during_final_capture(fixture: &Fixture) -> Arc<AtomicBool> {
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    *fixture
        .world
        .persistence_test_hooks
        .publication
        .lock()
        .unwrap() = Some(Arc::new(move || {
        observed.store(true, Ordering::Relaxed);
        // This is the capture thread, after Page was copied and before registry detach.
        QUEUED.with_borrow_mut(|queued| drop(queued.take()));
    }));
    called
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_queued_page_button_refuses_final_capture_mutation() {
    let fixture = Fixture::new();
    let original = book("original");
    let lectern = place_lectern(&fixture, &original, 0);
    let window = open_lectern(&fixture);
    QUEUED.with_borrow_mut(|queued| *queued = Some(GuardEffects::new()));
    click(&fixture, window, 2);
    assert_eq!(lectern.page.load(Ordering::Relaxed), 0);
    let called = drain_during_final_capture(&fixture);

    assert!(
        fixture
            .world
            .unload_block_entities(POSITION.chunk_position())
    );

    assert!(called.load(Ordering::Relaxed));
    let saved = fixture
        .world
        .level
        .loaded_chunks
        .get(&POSITION.chunk_position())
        .unwrap()
        .pending_block_entities
        .lock()
        .unwrap()
        .get(&POSITION)
        .unwrap()
        .clone();
    assert_eq!(saved.get_int("Page"), Some(0));
    assert_eq!(
        lectern.page.load(Ordering::Relaxed),
        0,
        "queued menu click changed Page after the final persistent capture"
    );
    assert!(!properties(&fixture).powered);
    assert!(lectern.get_stack(0).are_equal(&original));
    close(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_queued_take_agrees_with_final_capture_after_reload() {
    use pumpkin_world::chunk::io::{FileIO, LoadedData};

    let fixture = Fixture::new();
    let original = book("withdrawn during final capture");
    let lectern = place_lectern(&fixture, &original, 2);
    let mut state = properties(&fixture);
    state.powered = true;
    fixture.world.set_block_state(
        &POSITION,
        state.to_state_id(&Block::LECTERN),
        BlockFlags::NOTIFY_ALL,
    );
    let window = open_lectern(&fixture);
    QUEUED.with_borrow_mut(|queued| *queued = Some(GuardEffects::new()));
    click(&fixture, window, 3);
    click(&fixture, window, 3);
    let called = drain_during_final_capture(&fixture);

    assert!(
        fixture
            .world
            .unload_block_entities(POSITION.chunk_position())
    );

    assert!(called.load(Ordering::Relaxed));
    assert!(lectern.get_stack(0).is_empty());
    assert_eq!(book_counts(&fixture, &original), (1, 0));
    let pos = POSITION.chunk_position();
    let chunk = fixture.world.level.loaded_chunks.get(&pos).unwrap().clone();
    let saved = chunk
        .pending_block_entities
        .lock()
        .unwrap()
        .get(&POSITION)
        .unwrap()
        .clone();
    assert!(saved.get_compound("Book").is_none());
    fixture
        .world
        .level
        .chunk_saver
        .save_chunks(&fixture.world.level.level_folder, vec![(pos, chunk)])
        .await
        .unwrap();
    fixture.world.level.loaded_chunks.remove(&pos);
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    fixture
        .world
        .level
        .chunk_saver
        .fetch_chunks(&fixture.world.level.level_folder, &[pos], send)
        .await;
    let LoadedData::Loaded(reloaded) = receive.recv().await.unwrap() else {
        panic!("saved lectern chunk missing");
    };
    fixture.world.level.loaded_chunks.insert(pos, reloaded);

    let loaded = fixture.world.get_block_entity(&POSITION).unwrap();
    let loaded = loaded
        .as_any()
        .downcast_ref::<LecternBlockEntity>()
        .unwrap();
    assert!(loaded.get_stack(0).is_empty());
    assert!(
        !properties(&fixture).has_book,
        "reloaded empty lectern retained has_book"
    );
    assert!(
        !properties(&fixture).powered,
        "reloaded empty lectern stayed powered"
    );
    assert_eq!(book_counts(&fixture, &original), (1, 0));
    close(fixture).await;
}
