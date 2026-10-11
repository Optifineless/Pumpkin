//! Retained unload fixture that runs the production scheduler's queue and retirement path.
use super::{
    Chunk, ChunkHolder, ChunkLevel, ChunkListener, ChunkPos, DAG, GenerationSchedule, HashMapType,
    HashSetType, LevelChannel, StagedChunkEnum,
};
use crate::level::{Level, SyncChunk};
use pumpkin_util::math::vector2::Vector2;
use std::{collections::BinaryHeap, collections::HashMap, sync::Arc, sync::Mutex};

pub struct UnloadTestSchedule {
    schedule: GenerationSchedule,
    pos: ChunkPos,
}

impl UnloadTestSchedule {
    /// Queues an already published terrain holder without generation or scheduler workers.
    pub fn new(level: &Arc<Level>, chunk: SyncChunk) -> Result<Self, rayon::ThreadPoolBuildError> {
        let pos = Vector2::new(chunk.x, chunk.z);
        let (send_chunk, recv_chunk) = crossbeam::channel::unbounded();
        let (io_read, _) = tokio::sync::mpsc::channel(1);
        let (io_write, _) = tokio::sync::mpsc::channel(1);
        let schedule = GenerationSchedule {
            level: Arc::downgrade(level),
            failed_loads: HashMap::new(),
            queue: BinaryHeap::new(),
            graph: DAG::default(),
            last_level: ChunkLevel::default(),
            last_high_priority: Vec::new(),
            send_level: Arc::new(LevelChannel::new()),
            public_chunk_map: level.loaded_chunks.clone(),
            loaded_chunk_changes: level.loaded_chunk_changes.clone(),
            chunk_map: HashMap::from([(
                pos,
                ChunkHolder {
                    chunk: Some(Chunk::Level(chunk)),
                    current_stage: StagedChunkEnum::Full,
                    public: true,
                    ..ChunkHolder::default()
                },
            )]),
            unload_chunks: HashSetType::from_iter([pos]),
            waiting_for_chunks: HashSetType::default(),
            io_lock: Arc::new((
                Mutex::new(HashMapType::default()),
                tokio::sync::Notify::new(),
            )),
            running_task_count: 0,
            max_in_flight: 1,
            queue_dirty: false,
            recv_chunk,
            io_read,
            io_write,
            send_chunk,
            listener: Arc::new(ChunkListener::new()),
            lighting_config: level.lighting_config,
            last_unload: std::time::Instant::now(),
            generation_pool: Arc::new(rayon::ThreadPoolBuilder::new().num_threads(1).build()?),
        };
        Ok(Self { schedule, pos })
    }

    /// Polls the actual unload queue; completion includes its holder removal and retire call.
    pub fn poll(&mut self) -> bool {
        self.schedule.process_unload_queue();
        !self.schedule.chunk_map.contains_key(&self.pos)
    }
}
