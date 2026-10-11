use super::*;
use crate::{command::CommandSender, world::random_sequences::RandomSequences};
use pumpkin_nbt::nbt_compress::read_gzip_compound_tag;
use pumpkin_util::{identifier::Identifier, random::RandomImpl};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Barrier,
};

struct MetadataSaveBarrier {
    completed: Arc<Barrier>,
    first_save: AtomicBool,
}

impl WorldInfoWriter for MetadataSaveBarrier {
    fn write_world_info(
        &self,
        info: &LevelData,
        path: &std::path::Path,
    ) -> Result<(), WorldInfoError> {
        let result = AnvilLevelInfo.write_world_info(info, path);
        if self.first_save.swap(false, Ordering::Relaxed) {
            self.completed.wait();
        }
        result
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_autosave_metadata_does_not_publish_random_sequence_stub() {
    let directory = tempfile::tempdir().unwrap();
    let completed = Arc::new(Barrier::new(2));
    let mut server = combat_test_support::server(directory.path());
    Arc::get_mut(&mut server).unwrap().world_info_writer = Arc::new(MetadataSaveBarrier {
        completed: completed.clone(),
        first_save: AtomicBool::new(true),
    });
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let level = Arc::downgrade(&world.level);
    let random_path = directory.path().join("data/minecraft/random_sequences.dat");
    let source = CommandSender::Console.into_source(&server);
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("random reset * 42 false false", &source)
        .unwrap();
    dispatcher
        .execute_input("random value 1..100 minecraft:test", &source)
        .unwrap();
    drop(dispatcher);
    drop(source);

    let check_server = server.clone();
    let check_world = world.clone();
    let check_path = random_path.clone();
    let check = tokio::task::spawn_blocking(move || {
        // Hold the live RNG lock so the real autosave waits while metadata finishes.
        let mut sequences = check_server.random_sequences.lock().unwrap();
        let id = Identifier::parse("minecraft:test").unwrap();
        let words = sequences.get_or_create(&id, 0).random().state();
        check_world.level_time.lock().unwrap().world_age =
            i64::try_from(check_world.level.autosave_ticks).unwrap() - 1;
        check_world.tick_environment();
        completed.wait();
        let result = catch_unwind(AssertUnwindSafe(|| {
            assert!(
                !check_path.exists(),
                "Metadata published an RNG stub before the real save"
            );
        }));
        drop(sequences);
        (result, words)
    })
    .await
    .unwrap();
    server.shutdown().await;
    let scheduler_stopped = world.level.cancel_token.is_cancelled();
    drop(world);
    drop(server);
    assert!(scheduler_stopped);
    assert!(level.upgrade().is_none());
    check.0.unwrap();

    let root = read_gzip_compound_tag(std::fs::File::open(random_path).unwrap()).unwrap();
    let data = root.get_compound("data").unwrap();
    assert_eq!(data.get_int("salt"), Some(42));
    assert_eq!(data.get_bool("include_world_seed"), Some(false));
    assert_eq!(data.get_bool("include_sequence_id"), Some(false));
    assert_eq!(
        data.get_compound("sequences")
            .unwrap()
            .get_compound("minecraft:test")
            .unwrap()
            .get_long_array("source"),
        Some(check.1.as_slice())
    );
    let id = Identifier::parse("minecraft:test").unwrap();
    let mut restored = RandomSequences::load(directory.path()).unwrap();
    let mut expected =
        pumpkin_util::random::xoroshiro128::Xoroshiro::new(check.1[0] as u64, check.1[1] as u64);
    assert_eq!(
        restored.get_or_create(&id, 0).random().next_i64(),
        expected.next_i64()
    );
}
