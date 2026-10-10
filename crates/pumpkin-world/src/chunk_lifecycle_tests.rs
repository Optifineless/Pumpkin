use super::{Arc, AtomicBool, AtomicU8, Level, Ordering, PendingUnload, SAVED, Vector2};
use crate::chunk::ChunkData;
use pumpkin_config::world::LevelConfig;
use pumpkin_data::dimension::Dimension;

#[tokio::test]
async fn retry_keeps_admission_closed_through_the_next_snapshot_gate() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let chunk = ChunkData::empty_sync(0, 0);
    level.loaded_chunks.insert(pos, chunk.clone());
    let lifecycle = level.chunk_lifecycles.at(pos);
    {
        let mut state = lifecycle.lock().unwrap();
        state.generation = 2;
        state.pending = Some(PendingUnload {
            generation: 1,
            revision: 0,
            entity: None,
            completion: Arc::new(AtomicU8::new(SAVED)),
        });
    };
    let admitted = Arc::new(AtomicBool::new(false));
    let observed = admitted.clone();
    let weak = Arc::downgrade(&level);
    level.chunk_lifecycles.set_unload_gate(Arc::new(move |pos| {
        observed.store(
            weak.upgrade()
                .and_then(|level| level.try_chunk_mutation(pos))
                .is_some(),
            Ordering::Relaxed,
        );
        false
    }));
    assert!(!level.poll_chunk_unload(&chunk));
    assert!(
        !admitted.load(Ordering::Relaxed),
        "retry admitted a concurrent snapshot writer"
    );
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn saving_poll_never_takes_the_ticket_mutex() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let chunk = ChunkData::empty_sync(0, 0);
    level.loaded_chunks.insert(pos, chunk.clone());
    let cell = level.chunk_lifecycles.at(pos);
    {
        let mut state = cell.lock().unwrap();
        state.pending = Some(PendingUnload {
            generation: state.generation,
            revision: 0,
            entity: None,
            completion: Arc::new(AtomicU8::new(super::SAVING)),
        });
        state.set_quiescing(true);
    };
    let completed = {
        let tickets = level.chunk_loading.lock().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_level = level.clone();
        let worker = std::thread::spawn(move || {
            sender.send(worker_level.poll_chunk_unload(&chunk)).unwrap();
        });
        let completed = receiver.recv_timeout(std::time::Duration::from_secs(2));
        drop(tickets);
        worker.join().unwrap();
        completed
    };
    assert!(!completed.unwrap());
    level.shutdown().await.unwrap();
}

#[tokio::test]
async fn late_writer_cannot_reopen_a_pending_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let chunk = ChunkData::empty_sync(0, 0);
    level.loaded_chunks.insert(pos, chunk.clone());
    let cell = level.chunk_lifecycles.at(pos);
    // Writer reads open, then the unloader closes admission and starts its save.
    assert!(!cell.admission.quiescing.load(Ordering::SeqCst));
    assert!(!level.poll_chunk_unload(&chunk));
    let generation = cell.lock().unwrap().generation;
    // Pause the late writer between its increment and second flag read.
    cell.admission.mutations.fetch_add(1, Ordering::SeqCst);
    assert!(!level.poll_chunk_unload(&chunk));
    assert!(cell.admission.quiescing.load(Ordering::SeqCst));
    assert_eq!(cell.lock().unwrap().generation, generation);
    assert!(level.try_chunk_mutation(pos).is_none());
    cell.admission.mutations.fetch_sub(1, Ordering::SeqCst);
    level.shutdown().await.unwrap();
}

#[test]
fn retired_and_distinct_levels_never_reuse_generations() {
    let lifecycles = super::ChunkLifecycles::default();
    let pos = Vector2::new(0, 0);
    let old = lifecycles.at(pos).lock().unwrap().generation;
    lifecycles.retire(pos);
    assert!(lifecycles.get(pos).is_none());
    let recreated = lifecycles.at(pos).lock().unwrap().generation;
    let other = super::ChunkLifecycles::default()
        .at(pos)
        .lock()
        .unwrap()
        .generation;
    assert_ne!(old, recreated);
    assert_ne!(recreated, other);
}

#[tokio::test]
async fn tick_scope_skips_counts_and_preserves_the_closing_flag() {
    let dir = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let pos = Vector2::new(0, 0);
    let cell = level.chunk_lifecycles.at(pos);
    {
        let _tick = level.enter_tick_mutations();
        let _permit = level.try_chunk_mutation(pos).unwrap();
        assert_eq!(cell.lock().unwrap().mutations(), 0);
        cell.lock().unwrap().set_quiescing(true);
        assert!(level.try_chunk_mutation(pos).is_none());
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(!level.has_tick_mutation_scope()));
        });
    };
    assert!(!level.has_tick_mutation_scope());
    cell.lock().unwrap().set_quiescing(false);
    let permit = level.try_chunk_mutation(pos).unwrap();
    assert_eq!(cell.lock().unwrap().mutations(), 1);
    drop(permit);
    level.shutdown().await.unwrap();
}
