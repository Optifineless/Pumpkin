use super::*;
use crate::{
    block::entities::{BlockEntity, chest::ChestBlockEntity},
    server::scenario::{Scenario, run},
};
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_data::{Block, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::{
    chunk::{
        ChunkData,
        format::anvil::AnvilChunkFile,
        io::{FileIO, LoadedData, file_manager::ChunkFileManager},
    },
    world::BlockFlags,
};
use std::{
    future::{Future, poll_fn},
    sync::{Mutex, atomic::AtomicBool},
    time::Duration,
};

const CHEST: BlockPos = BlockPos::new(8, 64, 8);
const LATE_CHEST: BlockPos = BlockPos::new(9, 64, 8);

struct FinalChestCapture {
    chest: Arc<ChestBlockEntity>,
    captured: tokio::sync::mpsc::UnboundedSender<()>,
    resume: Mutex<Option<crossbeam::channel::Receiver<()>>>,
    pause: AtomicBool,
}

impl BlockEntity for FinalChestCapture {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.chest.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            let resume = self.resume.lock().unwrap().take().unwrap();
            self.captured.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(30)).unwrap();
        }
    }

    fn from_nbt(_nbt: &NbtCompound, position: BlockPos) -> Self {
        Self {
            chest: Arc::new(ChestBlockEntity::new(position)),
            captured: tokio::sync::mpsc::unbounded_channel().0,
            resume: Mutex::new(None),
            pause: AtomicBool::new(false),
        }
    }

    fn resource_location(&self) -> &'static str {
        ChestBlockEntity::ID
    }

    fn get_position(&self) -> BlockPos {
        self.chest.get_position()
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self.chest.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct CaptureGate {
    captured: tokio::sync::mpsc::UnboundedReceiver<()>,
    resume: crossbeam::channel::Sender<()>,
}

impl CaptureGate {
    fn new(world: &Arc<World>) -> Self {
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let chest = Arc::new(ChestBlockEntity::new(CHEST));
        chest.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
        let (captured, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (resume, writer) = crossbeam::channel::bounded(1);
        let entity = Arc::new(FinalChestCapture {
            chest,
            captured,
            resume: Mutex::new(Some(writer)),
            pause: AtomicBool::new(false),
        });
        world.add_block_entity(entity.clone());
        entity.chest.set_stack(0, ItemStack::new(7, &Item::EMERALD));
        entity.pause.store(true, Ordering::Release);
        Self {
            captured: receiver,
            resume,
        }
    }

    async fn reached(&mut self) {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(30), self.captured.recv())
                .await
                .unwrap(),
            Some(())
        );
    }
}

async fn scheduler_unload_barrier(scenario: &Scenario, world: &Arc<World>) {
    // A fresh demand cannot publish until the scheduler has processed the requested unload pass.
    let load = world
        .level
        .get_or_fetch_chunk(Vector2::new(20, 0), Clone::clone);
    tokio::pin!(load);
    scenario
        .drive(
            "final capture terrain unload barrier",
            poll_fn(|context| {
                world.level.should_unload.store(true, Ordering::Relaxed);
                world.level.level_channel.notify();
                load.as_mut().poll(context)
            }),
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entity_unload_retains_terrain_until_final_block_entity_serialization_finishes() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(uuid::Uuid::new_v4(), "RetainTerrain").await;
        let world = player.player.world();
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        let mut gate = CaptureGate::new(&world);
        scenario.command("tp RetainTerrain 163.5 64 3.5");
        player.loaded(&scenario.server);
        scenario
            .until("final capture watcher departure", || {
                player.is_idle() && scenario.server.tasks.is_empty()
            })
            .await;
        assert!(!world.level.is_chunk_watched(&CHEST.chunk_position()));
        let retiring_world = world.clone();
        let runtime = tokio::runtime::Handle::current();
        let retirement = std::thread::spawn(move || {
            let _runtime = runtime.enter();
            retiring_world.process_entity_chunk_lifecycle();
        });
        gate.reached().await;
        scheduler_unload_barrier(&scenario, &world).await;
        let kept_terrain = world.level.is_chunk_loaded(&CHEST.chunk_position());
        gate.resume.send(()).unwrap();
        retirement.join().unwrap();
        assert!(
            kept_terrain,
            "terrain eviction detached a chunk before its final block-entity image"
        );

        let scenario = scenario.restart().await;
        let player = scenario.join(uuid::Uuid::new_v4(), "ReloadTerrain").await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&CHEST)
            .unwrap()
            .get_inventory()
            .unwrap();
        assert_eq!(inventory.get_stack(0).item, &Item::EMERALD);
        assert_eq!(inventory.get_stack(0).item_count, 7);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entity_unload_keeps_old_terrain_sealed_until_deferred_reactivation() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    super::super::spawn_test_support::publish(
        world,
        super::super::spawn_test_support::proto(&pumpkin_data::biome::Biome::PLAINS, &Block::STONE),
    );
    let pos = CHEST.chunk_position();
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    assert!(world.make_chunk_entities_live(&storage, None));
    let mut gate = CaptureGate::new(world);
    world.level.request_entity_unloads([pos]);
    let retiring_world = world.clone();
    let runtime = tokio::runtime::Handle::current();
    let retirement = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        retiring_world.process_entity_chunk_lifecycle();
    });
    gate.reached().await;
    world.level.watch_entity_chunks(&[pos]);
    world.queue_entity_activation(&storage, None);
    world.process_entity_chunk_lifecycle();
    let deferred = world.pending_entity_activations.len();
    let activated_during_capture = storage.live.load(Ordering::Acquire);
    gate.resume.send(()).unwrap();
    retirement.join().unwrap();

    world.set_block_state(
        &LATE_CHEST,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let late_state = world.get_block_state_id(&LATE_CHEST);
    world.process_entity_chunk_lifecycle();
    assert_eq!(
        deferred, 1,
        "final capture discarded a pending chunk activation"
    );
    assert!(
        !activated_during_capture,
        "chunk activation crossed an active final capture"
    );
    assert_eq!(
        late_state,
        Block::AIR.default_state.id,
        "late placement crossed the completed terrain seal"
    );
    assert!(storage.live.load(Ordering::Acquire));
    world.set_block_state(
        &LATE_CHEST,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let live = world
        .get_block_entity(&LATE_CHEST)
        .expect("reactivated terrain rejected chest placement");
    live.get_inventory()
        .unwrap()
        .set_stack(0, ItemStack::new(4, &Item::GOLD_INGOT));
    world.save().await;
    flush_fixture_terrain(world, pos).await;
    let fixture = fixture.restart().await;
    load_fixture_terrain_from_disk(&fixture.world, pos).await;
    let inventory = fixture
        .world
        .get_block_entity(&LATE_CHEST)
        .unwrap()
        .get_inventory()
        .unwrap();
    assert_eq!(inventory.get_stack(0).item, &Item::GOLD_INGOT);
    assert_eq!(inventory.get_stack(0).item_count, 4);
    fixture.finish().await;
}

async fn flush_fixture_terrain(world: &World, pos: Vector2<i32>) {
    // Raw fixture publication has no scheduler holder to submit this saved image at shutdown.
    let chunk = world.level.loaded_chunks.get(&pos).unwrap().clone();
    world.level.write_chunks(vec![(pos, chunk)]).await;
    world
        .level
        .chunk_saver
        .block_and_await_ongoing_tasks()
        .await
        .unwrap();
}

async fn load_fixture_terrain_from_disk(world: &World, pos: Vector2<i32>) {
    // A fresh reader verifies disk contents without generating the fixture's lighting neighbors.
    let disk = ChunkFileManager::<AnvilChunkFile<ChunkData>>::in_dimension(
        AnvilChunkConfig::default(),
        world.dimension.clone(),
    );
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    disk.fetch_chunks(&world.level.level_folder, &[pos], send)
        .await;
    let chunk = match receive.recv().await {
        Some(LoadedData::Loaded(chunk)) => chunk,
        Some(LoadedData::Error((_, error))) => {
            panic!("reactivated terrain failed disk reload: {error}")
        }
        Some(LoadedData::Missing(_)) | None => {
            panic!("reactivated terrain missing on disk reload")
        }
    };
    world.level.loaded_chunks.insert(pos, chunk);
}
