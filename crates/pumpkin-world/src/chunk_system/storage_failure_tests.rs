use super::*;
use crate::chunk::ChunkData;

fn scheduler() -> (
    GenerationSchedule,
    tokio::sync::mpsc::Receiver<Vec<ChunkPos>>,
) {
    let (send_chunk, recv_chunk) = crossbeam::channel::unbounded();
    let (io_read, read_rx) = tokio::sync::mpsc::channel(8);
    let (io_write, _) = tokio::sync::mpsc::channel(8);
    (
        GenerationSchedule {
            level: Weak::new(),
            failed_loads: HashMap::new(),
            queue: BinaryHeap::new(),
            graph: DAG::default(),
            last_level: ChunkLevel::default(),
            last_high_priority: Vec::new(),
            send_level: Arc::new(LevelChannel::new()),
            public_chunk_map: Arc::new(DashMap::new()),
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
