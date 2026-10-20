use std::{
    sync::{Arc, atomic::AtomicUsize, mpsc},
    time::Duration,
};

use pumpkin_data::item::Item;
use pumpkin_util::sync::Mutex;

use super::{
    BlockDropItemEvent, EntityBase, FOOT, HEAD, PlayerFixture, Relaxed, TestPlayer, TestRecords,
    Vector3, destroy, drops, finish, place, register,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_head_loot_admission_rejects_recursive_and_concurrent_breaks() {
    let fixture = PlayerFixture::new();
    let client = place(&fixture, &Item::RED_BED);
    let recursive = Arc::new(TestPlayer::new(&fixture.world));
    fixture.world.players.store(Arc::new(vec![
        client.player.clone(),
        recursive.player.clone(),
    ]));
    recursive
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 2.5));
    let concurrent = recursive.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let (entered, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::<_, TestRecords>::new(resume);
    register::<BlockDropItemEvent>(&fixture.world, move |event| {
        if seen.fetch_add(1, Relaxed) == 0 {
            // A duplicate must return while this callback still owns no world guard.
            destroy(&recursive, &event.world, HEAD);
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(20))
                .unwrap();
        }
    });
    let runtime = tokio::runtime::Handle::current();
    let world = fixture.world.clone();
    let (completed, done) = mpsc::channel();
    std::thread::spawn(move || {
        let _runtime = runtime.enter();
        destroy(&client, &world, HEAD);
        completed.send(()).unwrap();
    });
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    let runtime = tokio::runtime::Handle::current();
    let world = fixture.world.clone();
    let (completed, duplicate_done) = mpsc::channel();
    std::thread::spawn(move || {
        let _runtime = runtime.enter();
        destroy(&concurrent, &world, HEAD);
        completed.send(()).unwrap();
    });
    let duplicate = duplicate_done.recv_timeout(Duration::from_secs(10));
    release.send(()).unwrap();
    duplicate.unwrap();
    done.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(drops(&fixture.world, &Item::RED_BED), 1);
    assert!(fixture.world.get_block(&FOOT).is_air());
    assert!(fixture.world.get_block(&HEAD).is_air());
    finish(fixture).await;
}
