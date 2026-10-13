use crate::server::combat_test_support::{publish_empty_chunk, server, world};
use bytes::Bytes;
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_data::{Block, block_properties::RedstoneOreLikeProperties};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{
    chunk::{
        ChunkData, ChunkReadingError, ChunkWritingError,
        format::anvil::{AnvilChunkFile, SingleChunkDataSerializer},
        io::{ChunkSerializer, Dirtiable, FileIO, LoadedData, file_manager::ChunkFileManager},
    },
    tick::TickPriority,
    world::BlockFlags,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Notify, OnceCell, mpsc};

struct TickWorld(Arc<super::World>);

impl Drop for TickWorld {
    fn drop(&mut self) {
        // Keep worker teardown safe when a behavioural assertion fails.
        self.0.level.cancel_token.cancel();
        self.0
            .level
            .shut_down_chunk_system
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.0.level.level_channel.notify();
    }
}

#[tokio::test]
async fn scheduled_time_production_world_tick_publishes_time_while_chunk_is_inactive() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let fixture = TickWorld(world(&server, directory.path()));
    let world = &fixture.0;
    publish_empty_chunk(world, Vector2::new(0, 0));
    let position = BlockPos::new(8, 64, 8);
    let mut properties = RedstoneOreLikeProperties::default(&Block::REDSTONE_LAMP);
    properties.lit = true;
    world.set_block_state(
        &position,
        properties.to_state_id(&Block::REDSTONE_LAMP),
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    // ServerLevel.tickTime precedes LevelTicks.tick; no fixture publishes the clock.
    world.tick(&server);
    world.schedule_block_tick(&Block::REDSTONE_LAMP, position, 4, TickPriority::Normal);
    for _ in 0..5 {
        world.tick(&server);
        assert!(RedstoneOreLikeProperties::from_state_id(world.get_block_state(&position).id).lit);
    }
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    world.tick(&server);
    assert!(!RedstoneOreLikeProperties::from_state_id(world.get_block_state(&position).id).lit);
    assert_eq!(world.level_time.lock().unwrap().world_age, 7);
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
async fn collection_invariant_world_load_publishes_restored_age_before_scheduling() {
    use pumpkin_world::world_info::{LevelData, WorldInfoWriter, anvil::AnvilLevelInfo};
    let directory = tempfile::tempdir().unwrap();
    let mut saved = LevelData::default(pumpkin_util::world_seed::Seed(0));
    saved.game_time = 10_000;
    AnvilLevelInfo
        .write_world_info(&saved, directory.path())
        .unwrap();
    // Both loads use real level.dat decoding and World::load, with no fixture clock publication.
    for _ in 0..2 {
        let server = server(directory.path());
        let fixture = TickWorld(world(&server, directory.path()));
        let world = &fixture.0;
        let chunk_pos = Vector2::new(0, 0);
        publish_empty_chunk(world, chunk_pos);
        world.forced_chunks.lock().unwrap().insert(chunk_pos);
        let position = BlockPos::new(8, 64, 8);
        let mut properties = RedstoneOreLikeProperties::default(&Block::REDSTONE_LAMP);
        properties.lit = true;
        world.set_block_state(
            &position,
            properties.to_state_id(&Block::REDSTONE_LAMP),
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        world.schedule_block_tick(&Block::REDSTONE_LAMP, position, 100, TickPriority::Normal);
        let chunk = world
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .unwrap()
            .value()
            .clone();
        world.tick(&server);
        assert_eq!(world.get_world_age(), 10_001);
        assert!(
            chunk
                .block_ticks
                .is_scheduled(position, &Block::REDSTONE_LAMP),
            "pre-tick schedule fired immediately after restart"
        );
        let pending = chunk.block_ticks.to_vec();
        let tick = pending
            .iter()
            .find(|tick| tick.position == position)
            .unwrap();
        assert_eq!(tick.delay, 99);
        assert_eq!(world.get_world_age() + i64::from(tick.delay), 10_100);
        assert!(RedstoneOreLikeProperties::from_state_id(world.get_block_state(&position).id).lit);
        world.level.shutdown().await.unwrap();
    }
}

struct PublicationPause {
    started: Notify,
    resume: Notify,
    first_write: OnceCell<()>,
}

#[derive(Default)]
struct PausedChunkWriter {
    anvil: AnvilChunkFile<ChunkData>,
    pause: Option<Arc<PublicationPause>>,
}

impl ChunkSerializer for PausedChunkWriter {
    type Data = ChunkData;
    type WriteBackend = PathBuf;
    type ChunkConfig = Arc<PublicationPause>;

    fn get_chunk_key(chunk: &Vector2<i32>) -> String {
        AnvilChunkFile::<ChunkData>::get_chunk_key(chunk)
    }

    fn should_write(&self, watched: bool) -> bool {
        self.anvil.should_write(watched)
    }

    async fn write(&self, path: &PathBuf) -> Result<(), std::io::Error> {
        let pause = self.pause.as_ref().unwrap();
        pause
            .first_write
            .get_or_init(|| async {
                pause.started.notify_one();
                pause.resume.notified().await;
            })
            .await;
        self.anvil.write(path).await
    }

    fn read(bytes: Bytes) -> Result<Self, ChunkReadingError> {
        Ok(Self {
            anvil: AnvilChunkFile::read(bytes)?,
            pause: None,
        })
    }

    async fn update_chunk(
        &mut self,
        chunk: Arc<Self::Data>,
        pause: &Self::ChunkConfig,
    ) -> Result<(), ChunkWritingError> {
        self.pause = Some(pause.clone());
        self.anvil
            .update_chunk(chunk, &AnvilChunkConfig::default())
            .await
    }

    async fn get_chunks(
        &self,
        chunks: Vec<Vector2<i32>>,
        stream: mpsc::Sender<LoadedData<Self::Data, ChunkReadingError>>,
        dimension: pumpkin_data::dimension::Dimension,
    ) {
        self.anvil.get_chunks(chunks, stream, dimension).await;
    }
}

#[tokio::test]
async fn clock_dirty_block_edit_survives_the_pending_image_publication() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let fixture = TickWorld(world(&server, directory.path()));
    let world = &fixture.0;
    let chunk_pos = Vector2::new(0, 0);
    let position = BlockPos::new(8, 64, 8);
    publish_empty_chunk(world, chunk_pos);
    world.level.set_game_time(1000);

    let chunk = world.level.loaded_chunks.get(&chunk_pos).unwrap().clone();
    chunk
        .section
        .set_block_absolute_y(8, 64, 8, Block::DIRT.default_state.id);
    world.schedule_block_tick(&Block::DIRT, position, 200, TickPriority::Normal);
    chunk.to_bytes().unwrap();
    chunk.mark_dirty(false);
    assert!(!chunk.is_dirty());
    world.level.set_game_time(1001);
    assert!(chunk.is_dirty(), "elapsed tick time should dirty the chunk");

    let pause = Arc::new(PublicationPause {
        started: Notify::new(),
        resume: Notify::new(),
        first_write: OnceCell::new(),
    });
    let manager = ChunkFileManager::<PausedChunkWriter>::new(pause.clone());
    manager.queue_chunks(&world.level.level_folder, vec![(chunk_pos, chunk.clone())]);

    let (save_result, ()) = tokio::join!(manager.block_and_await_ongoing_tasks(), async {
        pause.started.notified().await;
        world.level.set_game_time(1002);
        assert!(chunk.is_dirty());
        assert_eq!(
            world.set_block_state(
                &position,
                Block::STONE.default_state.id,
                BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
            ),
            Block::DIRT.default_state.id
        );
        pause.resume.notify_one();
    });
    save_result.unwrap();

    let reader = ChunkFileManager::<AnvilChunkFile<ChunkData>>::new(AnvilChunkConfig::default());
    let (send, mut receive) = mpsc::channel(1);
    reader
        .fetch_chunks(&world.level.level_folder, &[chunk_pos], send)
        .await;
    let Some(LoadedData::Loaded(saved)) = receive.recv().await else {
        panic!("published chunk image was not readable");
    };
    assert_eq!(
        saved.section.get_block_absolute_y(8, 64, 8),
        Some(Block::STONE.default_state.id),
        "the saved image must include the edit made before publication"
    );
    let saved_ticks = saved.block_ticks.to_vec();
    assert_eq!(saved_ticks.len(), 1);
    assert_eq!(saved_ticks[0].delay, 198);
    world.level.shutdown().await.unwrap();
}
