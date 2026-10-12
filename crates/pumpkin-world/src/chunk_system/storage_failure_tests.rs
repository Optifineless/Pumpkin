use super::*;
use crate::chunk::ChunkData;

#[test]
fn residency_after_unload_decision_preserves_terrain_until_release() {
    let (mut schedule, _) = scheduler();
    let pos = ChunkPos::new(0, 0);
    let chunk = ChunkData::empty_sync(0, 0);
    schedule.publish_chunk(pos, chunk.clone());
    schedule.chunk_map.insert(
        pos,
        ChunkHolder {
            public: true,
            chunk: Some(Chunk::Level(chunk.clone())),
            current_stage: StagedChunkEnum::Full,
            ..Default::default()
        },
    );
    schedule.unload_chunks.insert(pos);
    // The portal's real RAII handle arrives after the scheduler's unload decision.
    // A neighbor ticket also retains this position through propagated pos_level.
    let mut residency =
        crate::chunk_system::residency::ChunkResidency::new(schedule.chunk_loading.clone());
    residency.add(ChunkPos::new(1, 0));
    schedule.process_unload_queue();
    assert!(
        schedule
            .public_chunk_map
            .get(&pos)
            .is_some_and(|entry| Arc::ptr_eq(&entry, &chunk)),
        "residency ticket failed to retain the published incarnation"
    );
    assert!(schedule.chunk_map[&pos].public);
    assert!(schedule.unload_chunks.contains(&pos));
    assert!(
        matches!(schedule.loaded_chunk_changes.pop(), Some(LoadedChunkChange::Loaded(p)) if p == pos)
    );
    assert!(schedule.loaded_chunk_changes.is_empty());
    drop(residency);
    schedule.process_unload_queue();
    assert!(!schedule.public_chunk_map.contains_key(&pos));
    assert!(!schedule.chunk_map.contains_key(&pos));
    assert!(schedule.unload_chunks.is_empty());
    assert!(
        matches!(schedule.loaded_chunk_changes.pop(), Some(LoadedChunkChange::Unloaded(p)) if p == pos)
    );
    assert!(schedule.loaded_chunk_changes.is_empty());
}

#[tokio::test]
async fn rewatch_after_terrain_check_preserves_the_published_incarnation() {
    let directory = tempfile::tempdir().unwrap();
    let level = Level::from_root_folder(
        &pumpkin_config::world::LevelConfig::default(),
        directory.path().into(),
        0,
        pumpkin_data::dimension::Dimension::OVERWORLD,
    );
    let (mut schedule, _) = scheduler();
    schedule.entity_unloads = level.entity_unloads.clone();
    schedule.public_chunk_map = level.loaded_chunks.clone();
    let pos = ChunkPos::new(0, 0);
    let chunk = ChunkData::empty_sync(0, 0);
    schedule.publish_chunk(pos, chunk.clone());
    schedule.chunk_map.insert(
        pos,
        ChunkHolder {
            public: true,
            chunk: Some(Chunk::Level(chunk.clone())),
            current_stage: StagedChunkEnum::Full,
            ..Default::default()
        },
    );
    // The earlier scheduler decision is obsolete before the final map mutation.
    assert!(!schedule.entity_unloads.lock().unwrap().retains_terrain(pos));
    level.watch_entity_chunks(&[pos]);
    schedule.unload_chunks.insert(pos);
    schedule.process_unload_queue();
    assert!(Arc::ptr_eq(
        &schedule.public_chunk_map.get(&pos).unwrap(),
        &chunk
    ));
    assert!(schedule.chunk_map.contains_key(&pos));
    assert!(schedule.unload_chunks.contains(&pos));
    assert!(
        matches!(schedule.loaded_chunk_changes.pop(), Some(LoadedChunkChange::Loaded(p)) if p == pos)
    );
    assert!(schedule.loaded_chunk_changes.is_empty());
    level.shutdown().await.unwrap();
}

#[test]
fn terrain_unload_cannot_remove_a_replacement_incarnation() {
    let (mut schedule, _) = scheduler();
    let pos = ChunkPos::new(0, 0);
    let old = ChunkData::empty_sync(0, 0);
    let replacement = ChunkData::empty_sync(0, 0);
    schedule.chunk_map.insert(
        pos,
        ChunkHolder {
            public: true,
            chunk: Some(Chunk::Level(old)),
            current_stage: StagedChunkEnum::Full,
            ..Default::default()
        },
    );
    schedule.publish_chunk(pos, replacement.clone());
    schedule.unload_chunks.insert(pos);
    schedule.process_unload_queue();
    assert!(Arc::ptr_eq(
        &schedule.public_chunk_map.get(&pos).unwrap(),
        &replacement
    ));
    assert!(schedule.chunk_map.contains_key(&pos));
}

fn scheduler() -> (
    GenerationSchedule,
    tokio::sync::mpsc::Receiver<Vec<ChunkPos>>,
) {
    let (send_chunk, recv_chunk) = crossbeam::channel::unbounded();
    let (io_read, read_rx) = tokio::sync::mpsc::channel(8);
    let (io_write, _) = tokio::sync::mpsc::channel(8);
    let level_channel = Arc::new(LevelChannel::new());
    (
        GenerationSchedule {
            failed_loads: HashMap::new(),
            queue: BinaryHeap::new(),
            graph: DAG::default(),
            last_level: ChunkLevel::default(),
            last_high_priority: Vec::new(),
            send_level: level_channel.clone(),
            chunk_loading: Arc::new(pumpkin_util::sync::Mutex::new(ChunkLoading::new(
                level_channel,
            ))),
            public_chunk_map: Arc::new(DashMap::new()),
            entity_unloads: Arc::default(),
            loaded_chunk_changes: Arc::new(crossbeam::queue::SegQueue::new()),
            chunk_map: HashMap::new(),
            unload_chunks: HashSetType::default(),
            waiting_for_chunks: HashSetType::default(),
            io_lock: Arc::new((
                Mutex::new(HashMapType::default()),
                tokio::sync::Notify::new(),
            )),
            running_task_count: 1,
            max_in_flight: 8,
            queue_dirty: false,
            recv_chunk,
            io_read,
            io_write,
            send_chunk,
            listener: Arc::new(ChunkListener::new()),
            lighting_config: LightingEngineConfig::default(),
            last_unload: std::time::Instant::now(),
            generation_pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap(),
            ),
        },
        read_rx,
    )
}

fn add_read(schedule: &mut GenerationSchedule, pos: ChunkPos) {
    let occupied = schedule.graph.nodes.insert(Node::new(
        ChunkPos::new(i32::MAX, i32::MAX),
        StagedChunkEnum::None,
    ));
    let task = schedule
        .graph
        .nodes
        .insert(Node::new(pos, StagedChunkEnum::Empty));
    schedule.graph.nodes[task].in_flight = true;
    schedule.graph.add_edge(occupied, task);
    let mut holder = ChunkHolder {
        occupied,
        ..Default::default()
    };
    holder.tasks[StagedChunkEnum::Empty as usize] = task;
    schedule.chunk_map.insert(pos, holder);
}

#[test]
fn storage_review_failed_read_completes_waiters_idles_and_recovers_without_generation() {
    let (mut schedule, mut read_rx) = scheduler();
    let pos = ChunkPos::new(0, 0);
    add_read(&mut schedule, pos);
    let dependent = ChunkPos::new(1, 0);
    let node = schedule
        .graph
        .nodes
        .insert(Node::new(dependent, StagedChunkEnum::Empty));
    schedule.graph.add_edge(
        schedule.chunk_map[&pos].tasks[StagedChunkEnum::Empty as usize],
        node,
    );
    let mut waiter = schedule.listener.add_single_chunk_listener(pos);
    let mut neighbor_waiter = schedule.listener.add_single_chunk_listener(dependent);
    schedule.receive_chunk(pos, RecvChunk::IOFailure("failed read".into()));
    assert_eq!(schedule.running_task_count, 0);
    assert!(schedule.chunk_map[&pos].tasks[StagedChunkEnum::Empty as usize].is_null());
    assert!(schedule.graph.nodes.values().all(|node| !node.in_flight));
    assert!(schedule.debug_check());
    assert!(waiter.try_recv().unwrap().is_err());
    assert!(neighbor_waiter.try_recv().unwrap().is_err());
    assert!(schedule.queue.is_empty());
    schedule.failed_loads.insert(
        pos,
        std::time::Instant::now()
            .checked_sub(Duration::from_secs(2))
            .unwrap(),
    );
    schedule.retry_storage_reads();
    assert_eq!(read_rx.try_recv().unwrap(), vec![pos]);
    assert_eq!(schedule.running_task_count, 1);
    schedule.receive_chunk(
        pos,
        RecvChunk::IO(Chunk::Level(ChunkData::empty_sync(0, 0))),
    );
    assert_eq!(schedule.running_task_count, 0);
    assert!(schedule.failed_loads.is_empty());
    assert!(schedule.public_chunk_map.contains_key(&pos));
    assert_eq!(schedule.queue.pop().unwrap().node_key(), node);
    let mut new_waiter = schedule.listener.add_single_chunk_listener(pos);
    assert!(matches!(
        new_waiter.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    // A later login at the healthy neighbor must wait for readiness, not inherit F's error.
    let mut neighbor_login = schedule.listener.add_single_chunk_listener(dependent);
    assert!(matches!(
        neighbor_login.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    let neighbor = ChunkData::empty_sync(dependent.x, dependent.y);
    schedule.listener.process_new_chunk(dependent, &neighbor);
    assert!(Arc::ptr_eq(
        &neighbor_login.try_recv().unwrap().unwrap(),
        &neighbor
    ));
}

#[test]
fn failed_read_can_unload_and_accept_a_fresh_load() {
    let (mut schedule, _) = scheduler();
    let pos = ChunkPos::new(0, 0);
    add_read(&mut schedule, pos);
    schedule.receive_chunk(pos, RecvChunk::IOFailure("failed read".into()));
    let dependent = ChunkPos::new(1, 0);
    schedule.chunk_map.insert(dependent, ChunkHolder::default());
    schedule
        .listener
        .process_failed_chunk(dependent, "dependency failed");
    schedule.garbage_collect_dependencies();
    schedule.unload_chunks.insert(pos);
    schedule.process_unload_queue();
    assert!(schedule.chunk_map.is_empty());
    assert!(schedule.graph.nodes.is_empty());
    assert!(schedule.failed_loads.is_empty());
    assert!(schedule.debug_check());
    let mut waiter = schedule.listener.add_single_chunk_listener(dependent);
    assert!(matches!(
        waiter.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
}

#[test]
fn shutdown_collects_late_worker_results_before_the_final_save() {
    let (mut schedule, _) = scheduler();
    let (write, mut writes) = tokio::sync::mpsc::channel(8);
    schedule.io_write = write;
    let pos = ChunkPos::new(0, 0);
    add_read(&mut schedule, pos);
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.mark_dirty(true);
    let sender = schedule.send_chunk.clone();
    let worker = std::thread::spawn(move || {
        sender
            .send((pos, RecvChunk::IO(Chunk::Level(chunk))))
            .unwrap();
    });
    schedule.finish_storage();
    worker.join().unwrap();
    let written = writes
        .try_recv()
        .expect("late result must be included in final save");
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].0, pos);
}
