use super::*;
use futures::FutureExt;
use std::{panic::AssertUnwindSafe, path::PathBuf, sync::atomic::Ordering};

const EXHAUSTION_POLLS: usize = 3;

struct Resources {
    directory: PathBuf,
    fixture: std::rc::Weak<Fixture>,
    server: std::sync::Weak<Server>,
    levels: Vec<Arc<Level>>,
    client: Arc<ClientPlatform>,
    probe: Uuid,
}

fn assert_levels_stopped(levels: &[Arc<Level>]) {
    for level in levels {
        assert!(level.shut_down_chunk_system.load(Ordering::Acquire));
        assert!(level.cancel_token.is_cancelled());
        assert!(level.thread_tracker.lock().unwrap().is_empty());
        assert!(level.chunk_system_tasks.is_closed());
        assert!(level.chunk_system_tasks.is_empty());
        assert!(level.world_portal.load().is_none());
    }
}

async fn fail_on_purpose(resources: &mut Option<Resources>, kind: usize) {
    let scenario = Scenario::new().await;
    let session = scenario.join(Uuid::new_v4(), "CleanupFailure").await;
    let worlds = scenario.server.worlds.load_full();
    // The tripwire needs an entity UUID, not publication into the world's ticking graph.
    let cart =
        crate::entity::r#type::from_type(&EntityType::MINECART, SPAWN, &worlds[0], Uuid::new_v4());
    let probe = cart.get_entity().entity_uuid;
    let _watch = riding_lock_probe::watch(&cart);
    assert!(riding_lock_probe::is_watched(probe));
    *resources = Some(Resources {
        directory: scenario.directory.path().to_owned(),
        fixture: Rc::downgrade(&scenario.fixture),
        server: Arc::downgrade(&scenario.server),
        levels: worlds.iter().map(|world| world.level.clone()).collect(),
        client: session.player.client.clone(),
        probe,
    });
    if kind == 1 {
        drive(
            "intentional exhaustion",
            std::future::pending::<()>(),
            || scenario.pump(),
            EXHAUSTION_POLLS,
        )
        .await;
    }
    if kind == 2 {
        // Force leave() to panic during cleanup, after its real storage/disconnect work.
        let poison = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = session.player.storage_session.lock().unwrap();
            panic!("intentional cleanup lock poison");
        }));
        assert!(poison.is_err());
    }
    assert_eq!(kind, usize::MAX, "intentional scenario assertion");
}

#[tokio::test]
async fn failure_cleans_resources_before_next_scenario() {
    for kind in 0..3 {
        let mut resources = None;
        let failed = AssertUnwindSafe(run(fail_on_purpose(&mut resources, kind)))
            .catch_unwind()
            .await;
        let error = failed.unwrap_err();
        let message = error.downcast_ref::<String>().unwrap();
        let expected = if kind == 1 {
            "scenario intentional exhaustion did not complete"
        } else {
            "intentional scenario assertion"
        };
        assert!(
            message.contains(expected),
            "original failure lost: {message}"
        );
        let resources = resources.unwrap();
        assert_levels_stopped(&resources.levels);
        assert!(resources.client.java().unwrap().is_closed());
        assert_eq!(client_task_count(resources.client.java().unwrap()), 0);
        drop(resources.client);
        assert!(resources.server.upgrade().is_none());
        assert!(!riding_lock_probe::is_watched(resources.probe));
        assert!(!resources.directory.exists());
        // This fixture owned the permit; other queued tests may already hold the gate now.
        assert!(resources.fixture.upgrade().is_none());
        run(async {
            let scenario = Scenario::new().await;
            let session = scenario.join(Uuid::new_v4(), "AfterFailure").await;
            scenario.ticks(1).await;
            assert!(session.player.has_client_loaded());
            assert!(!session.client().is_closed());
            scenario.stop().await;
        })
        .await;
    }
}

#[tokio::test]
async fn restart_releases_old_fixture_worlds_and_schedulers() {
    run(async {
        let scenario = Scenario::new().await;
        let worlds: Vec<_> = scenario
            .server
            .worlds
            .load()
            .iter()
            .map(Arc::downgrade)
            .collect();
        let levels: Vec<_> = scenario
            .server
            .worlds
            .load()
            .iter()
            .map(|world| world.level.clone())
            .collect();
        let server = Arc::downgrade(&scenario.server);
        let scenario = scenario.restart().await;
        assert_levels_stopped(&levels);
        assert!(server.upgrade().is_none());
        assert!(worlds.iter().all(|world| world.upgrade().is_none()));
        let session = scenario.join(Uuid::new_v4(), "AfterRestart").await;
        scenario.ticks(1).await;
        assert!(session.player.has_client_loaded());
        scenario.stop().await;
    })
    .await;
}
