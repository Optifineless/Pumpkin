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
