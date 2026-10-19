use crate::{
    block::entities::chest::ChestBlockEntity,
    command::CommandSender,
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::Block;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const CHEST: BlockPos = BlockPos::new(8, 64, 8);
const TARGET: BlockPos = BlockPos::new(9, 64, 8);
const COMMAND: &str = "setblock 9 64 8 stone";
const BOUND: Duration = Duration::from_secs(3);

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        for chunk in [Vector2::new(0, 0), Vector2::new(1, 0)] {
            combat_test_support::publish_empty_chunk(&world, chunk);
        }
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        world.add_block_entity(Arc::new(ChestBlockEntity::new(CHEST)));
        Self {
            server,
            world,
            _directory: directory,
        }
    }

    fn retire(&self) -> crossbeam::channel::Receiver<()> {
        let world = self.world.clone();
        let runtime = tokio::runtime::Handle::current();
        let (finished, wait_for_finish) = crossbeam::channel::bounded(1);
        drop(std::thread::spawn(move || {
            let _runtime = runtime.enter();
            world.level.request_entity_unloads([CHEST.chunk_position()]);
            world.process_entity_chunk_lifecycle();
            finished.send(()).unwrap();
        }));
        wait_for_finish
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_window_command_worker_waits_for_real_retirement() {
    let fixture = Fixture::new();
    let (captured, mut wait_for_capture) = tokio::sync::mpsc::unbounded_channel();
    let (resume, wait_for_resume) = crossbeam::channel::bounded(1);
    *fixture
        .world
        .persistence_test_hooks
        .publication
        .lock()
        .unwrap() = Some(Arc::new(move || {
        captured.send(()).unwrap();
        wait_for_resume.recv_timeout(BOUND).unwrap();
    }));
    let retirement = fixture.retire();
    tokio::time::timeout(BOUND, wait_for_capture.recv())
        .await
        .unwrap()
        .unwrap();
    let (started, wait_for_start) = crossbeam::channel::bounded(1);
    let (finished, wait_for_finish) = crossbeam::channel::bounded(1);
    let server = fixture.server.clone();
    let runtime = tokio::runtime::Handle::current();
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        let source = CommandSender::Console.into_source(&server).with_silent();
        started.send(()).unwrap();
        let success = server
            .command_dispatcher
            .load()
            .execute_input(COMMAND, &source)
            == Ok(1);
        finished.send(success).unwrap();
    }));
    wait_for_start.recv_timeout(BOUND).unwrap();
    let early = wait_for_finish.recv_timeout(Duration::from_millis(25));
    let waited = matches!(early, Err(crossbeam::channel::RecvTimeoutError::Timeout));
    resume.send(()).unwrap();
    retirement.recv_timeout(BOUND).unwrap();
    let success = early.unwrap_or_else(|_| wait_for_finish.recv_timeout(BOUND).unwrap());
    let block = fixture.world.get_block(&TARGET);
    fixture.world.level.shutdown().await.unwrap();
    assert!(
        waited,
        "command returned before the active unload capture completed"
    );
    assert!(
        success,
        "command refused resident terrain after the capture completed"
    );
    assert_eq!(block, &Block::STONE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_window_reentrant_command_refuses_then_reopens() {
    let fixture = Fixture::new();
    let (called, mut wait_for_call) = tokio::sync::mpsc::unbounded_channel();
    let server = fixture.server.clone();
    *fixture
        .world
        .persistence_test_hooks
        .publication
        .lock()
        .unwrap() = Some(Arc::new(move || {
        let source = CommandSender::Console.into_source(&server).with_silent();
        let start = Instant::now();
        let refused = server
            .command_dispatcher
            .load()
            .execute_input(COMMAND, &source)
            .is_err();
        called.send((refused, start.elapsed())).unwrap();
    }));
    let retirement = fixture.retire();
    let (refused, elapsed) = tokio::time::timeout(BOUND, wait_for_call.recv())
        .await
        .unwrap()
        .unwrap();
    retirement.recv_timeout(BOUND).unwrap();
    assert!(refused, "capture callback mutated its own final image");
    assert!(
        elapsed < Duration::from_millis(200),
        "capture callback waited on itself: {elapsed:?}"
    );
    assert_eq!(fixture.world.get_block(&TARGET), &Block::AIR);
    let source = CommandSender::Console
        .into_source(&fixture.server)
        .with_silent();
    let reopened = fixture
        .server
        .command_dispatcher
        .load()
        .execute_input(COMMAND, &source);
    fixture.world.level.shutdown().await.unwrap();
    assert_eq!(
        reopened,
        Ok(1),
        "completed callback capture kept terrain sealed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_window_command_callback_defers_neighbour_retirement() {
    let fixture = Fixture::new();
    let neighbour = Vector2::new(1, 0);
    let storage = fixture
        .world
        .level
        .get_entity_chunk(neighbour)
        .await
        .unwrap();
    assert!(fixture.world.make_chunk_entities_live(&storage, None));
    let world = fixture.world.clone();
    *fixture
        .world
        .persistence_test_hooks
        .registration
        .lock()
        .unwrap() = Some(Arc::new(move || {
        // A real BlockInput.place callback tries to retire another chunk while the
        // command owns its source. A neighbour callback must retain that same exclusion.
        world.level.request_entity_unloads([neighbour]);
        world.process_entity_chunk_lifecycle();
    }));
    let source = CommandSender::Console
        .into_source(&fixture.server)
        .with_silent();
    let result = fixture
        .server
        .command_dispatcher
        .load()
        .execute_input("setblock 9 64 8 chest", &source);
    fixture
        .world
        .persistence_test_hooks
        .registration
        .lock()
        .unwrap()
        .take();
    let deferred = storage.live.load(std::sync::atomic::Ordering::Acquire);
    fixture.world.process_entity_chunk_lifecycle();
    let retired_after_command = !storage.live.load(std::sync::atomic::Ordering::Acquire);
    fixture.world.level.shutdown().await.unwrap();
    assert_eq!(result, Ok(1));
    assert!(
        deferred,
        "cross-chunk retirement entered a command's callback window"
    );
    assert!(
        retired_after_command,
        "deferred neighbour retirement was lost"
    );
}
