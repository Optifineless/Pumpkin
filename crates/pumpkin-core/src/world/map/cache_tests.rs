use super::*;
use std::{fs, time::Duration};

const DEADLINE: Duration = Duration::from_secs(5);
const MAP: &[u8] = include_bytes!("../../../tests/fixtures/maps/0.dat");

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_failed_map_reads_back_off_and_recover() {
    for malformed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("data/minecraft/maps");
        fs::create_dir_all(&folder).unwrap();
        if malformed {
            fs::write(folder.join("0.dat"), [0, 1, 2]).unwrap();
        }
        let manager = MapManager::load(directory.path()).unwrap();
        let (_, _, finished) = manager.pause_next_map_publication_for_test();
        assert!(manager.get_map(0).is_none());
        finished.recv_timeout(DEADLINE).unwrap();
        let (entered, release) = manager.pause_next_load_for_test();
        let (_, _, finished) = manager.pause_next_map_publication_for_test();
        for _ in 0..100 {
            assert!(manager.get_map(0).is_none());
        }
        // request_load consumes its gate synchronously only when it starts a worker.
        let retained = manager.load_gate.lock().unwrap().before_read.is_some();
        let _ = release.send(());
        if !retained {
            entered.recv_timeout(DEADLINE).unwrap();
            finished.recv_timeout(DEADLINE).unwrap();
        }
        assert!(retained, "failed map was immediately scanned again");
        *manager.load_gate.lock().unwrap() = cache::LoadTestGates::default();

        // An external repair/import is observed immediately by explicit load_map.
        fs::write(folder.join("0.dat"), MAP).unwrap();
        assert!(manager.load_map(0).await.is_some());
        assert!(!manager.load_backoff.contains_key(&0));

        let (_, _, finished) = manager.pause_next_map_publication_for_test();
        assert!(manager.get_map(1).is_none());
        finished.recv_timeout(DEADLINE).unwrap();
        let created = manager.create_map(1, Dimension::OVERWORLD, 0, 0, 0);
        assert!(!manager.load_backoff.contains_key(&1));
        assert!(Arc::ptr_eq(&created, &manager.get_map(1).unwrap()));

        let (_, _, finished) = manager.pause_next_map_publication_for_test();
        assert!(manager.get_map(2).is_none());
        finished.recv_timeout(DEADLINE).unwrap();
        fs::write(folder.join("2.dat"), MAP).unwrap();
        // Expire the deadline deterministically; never sleep to test backoff.
        manager.load_backoff.insert(2, std::time::Instant::now());
        let (published, release, finished) = manager.pause_next_map_publication_for_test();
        assert!(manager.get_map(2).is_none());
        published.recv_timeout(DEADLINE).unwrap();
        release.send(()).unwrap();
        finished.recv_timeout(DEADLINE).unwrap();
        assert_eq!(manager.get_map(2).unwrap().lock().unwrap().colors[0], 231);
        tokio::time::timeout(DEADLINE, manager.drain())
            .await
            .unwrap();
    }
}
