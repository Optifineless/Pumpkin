// Compiled by both hosts to exercise each actual WIT mutation entry point.
use super::*;
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_core::{
    block::entities::{
        hanging_sign::HangingSignBlockEntity as CoreHangingSign,
        jukebox::JukeboxBlockEntity as CoreJukebox, map::MapBlockEntity as CoreMap,
        sign::SignBlockEntity as CoreSign,
    },
    server::combat_test_support,
    world::World,
};
use pumpkin_data::Block;
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
use std::sync::{
    Mutex,
    atomic::AtomicBool,
    mpsc::{self, Receiver, Sender},
};
use std::time::Duration;

const SIGN: BlockPos = BlockPos::new(8, 64, 8);
const HANGING: BlockPos = BlockPos::new(9, 64, 8);
const MAP: BlockPos = BlockPos::new(10, 64, 8);
const JUKEBOX: BlockPos = BlockPos::new(11, 64, 8);
const GATE: BlockPos = BlockPos::new(12, 64, 8);

struct Fixture {
    directory: tempfile::TempDir,
    world: Arc<World>,
    state: PluginHostState,
    sign: Arc<CoreSign>,
    sign_rep: u32,
    hanging_rep: u32,
    map_rep: u32,
    jukebox_rep: u32,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, SIGN.chunk_position());
        for (position, block) in [
            (SIGN, &Block::OAK_SIGN),
            (HANGING, &Block::OAK_HANGING_SIGN),
            (JUKEBOX, &Block::JUKEBOX),
            (GATE, &Block::OAK_SIGN),
        ] {
            world.set_block_state(&position, block.default_state.id, BlockFlags::FORCE_STATE);
        }
        let sign = Arc::new(CoreSign::empty(SIGN));
        let hanging = Arc::new(CoreHangingSign::empty(HANGING));
        let map = Arc::new(CoreMap::new(MAP, 1));
        let jukebox = Arc::new(CoreJukebox::new(JUKEBOX));
        world.add_block_entity(sign.clone());
        world.add_block_entity(hanging.clone());
        world.add_block_entity(map.clone());
        world.add_block_entity(jukebox.clone());
        let mut state = PluginHostState::new();
        state.server = Some(server);
        let sign_rep = state.add::<SignBlockEntity>(sign.clone()).unwrap().rep();
        let hanging_rep = state.add::<HangingSignBlockEntity>(hanging).unwrap().rep();
        let map_rep = state.add::<MapBlockEntity>(map).unwrap().rep();
        let jukebox_rep = state.add::<JukeboxBlockEntity>(jukebox).unwrap().rep();
        Self {
            directory,
            world,
            state,
            sign,
            sign_rep,
            hanging_rep,
            map_rep,
            jukebox_rep,
        }
    }

    fn mutation_results(&mut self) -> Vec<(&'static str, bool)> {
        let mut results = self.sign_mutation_results();
        results.extend(self.map_mutation_results());
        results.extend(self.jukebox_mutation_results());
        results
    }

    fn sign_mutation_results(&mut self) -> Vec<(&'static str, bool)> {
        let sign = self.sign_rep;
        let hanging = self.hanging_rep;
        vec![
            (
                "sign front",
                HostSignBlockEntity::set_front_text(
                    &mut self.state,
                    Resource::new_borrow(sign),
                    text(),
                )
                .is_ok(),
            ),
            (
                "sign back",
                HostSignBlockEntity::set_back_text(
                    &mut self.state,
                    Resource::new_borrow(sign),
                    text(),
                )
                .is_ok(),
            ),
            (
                "sign wax",
                HostSignBlockEntity::set_waxed(&mut self.state, Resource::new_borrow(sign), true)
                    .is_ok(),
            ),
            (
                "hanging front",
                HostHangingSignBlockEntity::set_front_text(
                    &mut self.state,
                    Resource::new_borrow(hanging),
                    text(),
                )
                .is_ok(),
            ),
            (
                "hanging back",
                HostHangingSignBlockEntity::set_back_text(
                    &mut self.state,
                    Resource::new_borrow(hanging),
                    text(),
                )
                .is_ok(),
            ),
            (
                "hanging wax",
                HostHangingSignBlockEntity::set_waxed(
                    &mut self.state,
                    Resource::new_borrow(hanging),
                    true,
                )
                .is_ok(),
            ),
        ]
    }

    fn map_mutation_results(&mut self) -> Vec<(&'static str, bool)> {
        let map = self.map_rep;
        vec![
            (
                "map id",
                HostMapBlockEntity::set_map_id(&mut self.state, Resource::new_borrow(map), 7)
                    .is_ok(),
            ),
            (
                "map colors",
                HostMapBlockEntity::set_colors(
                    &mut self.state,
                    Resource::new_borrow(map),
                    vec![9; 4],
                )
                .is_ok(),
            ),
            (
                "map pixel",
                HostMapBlockEntity::set_pixel(&mut self.state, Resource::new_borrow(map), 0, 0, 8)
                    .is_ok(),
            ),
            (
                "map stream",
                HostMapBlockEntity::stream_frame(
                    &mut self.state,
                    Resource::new_borrow(map),
                    vec![6; 4],
                )
                .is_ok(),
            ),
        ]
    }

    fn jukebox_mutation_results(&mut self) -> Vec<(&'static str, bool)> {
        let jukebox = self.jukebox_rep;
        vec![
            (
                "jukebox start",
                HostJukeboxBlockEntity::start_playing(
                    &mut self.state,
                    Resource::new_borrow(jukebox),
                    100,
                )
                .is_ok(),
            ),
            (
                "jukebox stop",
                HostJukeboxBlockEntity::stop_playing(
                    &mut self.state,
                    Resource::new_borrow(jukebox),
                )
                .is_ok(),
            ),
        ]
    }

    async fn assert_disk_reload(&mut self, accepted_live_mutations: bool) {
        // The passable fixture has no scheduler holder: flush its exact final pending image explicitly.
        let chunk = self
            .world
            .level
            .loaded_chunks
            .get(&SIGN.chunk_position())
            .unwrap()
            .clone();
        self.world
            .level
            .write_chunks(vec![(SIGN.chunk_position(), chunk)])
            .await;
        self.world
            .level
            .chunk_saver
            .block_and_await_ongoing_tasks()
            .await
            .unwrap();
        self.state.server.take();
        let server = combat_test_support::server(self.directory.path());
        let world = combat_test_support::world(&server, self.directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        load_disk_chunk(&world).await;
        let sign = world.get_block_entity(&SIGN).unwrap();
        let sign = sign.as_any().downcast_ref::<CoreSign>().unwrap();
        assert_eq!(sign.front_text.get_message(0, false).as_ref(), "");
        assert!(!sign.is_waxed.load(Ordering::Relaxed));
        let hanging = world.get_block_entity(&HANGING).unwrap();
        let hanging = hanging.as_any().downcast_ref::<CoreHangingSign>().unwrap();
        assert_eq!(
            hanging.front_text.get_message(0, false).as_ref(),
            if accepted_live_mutations {
                "host mutation"
            } else {
                ""
            }
        );
        assert_eq!(
            hanging.is_waxed.load(Ordering::Relaxed),
            accepted_live_mutations
        );
        // The fork's synthetic map entity has no vanilla terrain owner; inspect its saved record directly.
        let map = world.get_block_entity(&MAP).unwrap();
        let map = map.as_any().downcast_ref::<CoreMap>().unwrap();
        assert_eq!(
            map.get_map_id(),
            if accepted_live_mutations { 7 } else { 1 }
        );
        assert_eq!(
            map.get_pixel(0, 0),
            if accepted_live_mutations { 6 } else { 0 }
        );
        assert!(world.get_block_entity(&JUKEBOX).is_some());
        world.shutdown().await;
    }
}

async fn load_disk_chunk(world: &World) {
    // A fresh disk reader bypasses retained images and avoids generating relight neighbors for this fixture.
    let disk = ChunkFileManager::<AnvilChunkFile<ChunkData>>::in_dimension(
        AnvilChunkConfig::default(),
        world.dimension.clone(),
    );
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    disk.fetch_chunks(&world.level.level_folder, &[SIGN.chunk_position()], send)
        .await;
    let chunk = match receive.recv().await {
        Some(LoadedData::Loaded(chunk)) => chunk,
        Some(LoadedData::Error((_, error))) => {
            panic!("saved host chunk failed disk reload: {error}")
        }
        Some(LoadedData::Missing(_)) | None => panic!("saved host chunk missing on disk reload"),
    };
    world
        .level
        .loaded_chunks
        .insert(SIGN.chunk_position(), chunk);
}

fn text() -> SignText {
    SignText {
        messages: vec!["host mutation".into()],
        color: DyeColor::Red,
        has_glowing_text: true,
    }
}

struct PausedFinalCapture {
    captured: Sender<()>,
    resume: Mutex<Option<Receiver<()>>>,
    pause: AtomicBool,
}

impl InternalBlockEntity for PausedFinalCapture {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        CoreSign::empty(GATE).write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            let resume = self.resume.lock().unwrap().take().unwrap();
            self.captured.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(30)).unwrap();
        }
    }

    fn from_nbt(_nbt: &NbtCompound, _position: BlockPos) -> Self {
        Self {
            captured: mpsc::channel().0,
            resume: Mutex::new(None),
            pause: AtomicBool::new(false),
        }
    }

    fn resource_location(&self) -> &'static str {
        "minecraft:sign"
    }

    fn get_position(&self) -> BlockPos {
        GATE
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test]
async fn block_entity_host_mutations_accept_live_and_reject_replaced_resources() {
    let mut fixture = Fixture::new();
    let results = fixture.mutation_results();
    fixture
        .world
        .add_block_entity(Arc::new(CoreSign::empty(SIGN)));
    let replaced = HostSignBlockEntity::set_waxed(
        &mut fixture.state,
        Resource::new_borrow(fixture.sign_rep),
        false,
    );
    fixture.world.shutdown().await;
    for (name, accepted) in results {
        assert!(accepted, "live {name} host mutation was rejected");
    }
    assert!(replaced.is_err(), "a replaced sign resource was mutated");
    assert!(fixture.sign.is_waxed.load(Ordering::Relaxed));
    fixture.assert_disk_reload(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_entity_host_mutations_reject_sealed_shutdown_before_final_capture() {
    let mut fixture = Fixture::new();
    let (captured, wait_for_capture) = mpsc::channel();
    let (resume_capture, resume) = mpsc::channel();
    let gate = Arc::new(PausedFinalCapture {
        captured,
        resume: Mutex::new(Some(resume)),
        pause: AtomicBool::new(false),
    });
    fixture.world.add_block_entity(gate.clone());
    gate.pause.store(true, Ordering::Release);
    let world = fixture.world.clone();
    let shutdown = tokio::spawn(async move { world.shutdown().await });
    tokio::task::spawn_blocking(move || {
        wait_for_capture
            .recv_timeout(Duration::from_secs(30))
            .unwrap();
    })
    .await
    .unwrap();
    let results = fixture.mutation_results();
    resume_capture.send(()).unwrap();
    shutdown.await.unwrap();
    for (name, accepted) in results {
        assert!(!accepted, "sealed shutdown admitted {name} host mutation");
    }
    assert_eq!(fixture.sign.front_text.get_message(0, false).as_ref(), "");
    assert!(!fixture.sign.is_waxed.load(Ordering::Relaxed));
    fixture.assert_disk_reload(false).await;
}
