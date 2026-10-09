use super::*;
use crate::{
    entity::{Entity, EntityBase},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
    world::WorldPortal,
};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::{
    error::Error,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

// Child harnesses isolate /proc/self/fd from the other four concurrently running tests.
fn isolated(
    test: &str,
    run: impl FnOnce(bool) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    const CHILD: &str = "PUMPKIN_FIXTURE_FD_CHILD";
    for (mode, teardown) in [("retain", false), ("close", true)] {
        let child = format!("{test}:{mode}");
        if std::env::var(CHILD).as_deref() == Ok(child.as_str()) {
            return run(teardown);
        }
    }
    let name = test.strip_prefix("pumpkin_core::").unwrap();
    for (mode, succeeds) in [("retain", false), ("close", true)] {
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", name, "--nocapture", "--test-threads=4"])
            .env(CHILD, format!("{test}:{mode}"))
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), succeeds, "{stdout}\n{stderr}");
        if !succeeds {
            assert!(
                stderr.contains("fixture retained runtime or storage descriptors"),
                "{stdout}\n{stderr}"
            );
        }
    }
    Ok(())
}

fn fd_count() -> std::io::Result<usize> {
    Ok(std::fs::read_dir("/proc/self/fd")?.count())
}

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    client_task_finished: Arc<AtomicBool>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut server = server(directory.path());
        // Fixture callers configure the unique Arc before world/player back-references exist.
        Arc::get_mut(&mut server)
            .unwrap()
            .advanced_config
            .pvp
            .enabled = false;
        let world = world(&server, directory.path());
        let player = TestPlayer::new(&world);
        let client_task_finished = Arc::new(AtomicBool::new(false));
        let task_player = player.player.clone();
        let finished = client_task_finished.clone();
        assert!(
            player
                .player
                .spawn_task(async move {
                    task_player.client.await_close_interrupt().await;
                    finished.store(true, Ordering::Release);
                })
                .is_some()
        );
        player
            .client()
            .player
            .store(Arc::new(Some(player.player.clone())));
        // Server::new installs this world -> level -> portal -> world cycle.
        world
            .level
            .world_portal
            .store(Arc::new(Some(Arc::new(WorldPortal(world.clone())))));
        let mount: Arc<dyn EntityBase> = Arc::new(Entity::new(
            world.clone(),
            Vector3::new(0.0, 64.0, 0.0),
            &EntityType::MINECART,
        ));
        mount
            .get_entity()
            .add_passenger(mount.clone(), player.player.clone());
        Self {
            server,
            world,
            player,
            client_task_finished,
            _directory: directory,
        }
    }
}

#[test]
fn fixture_teardown_releases_worlds_mounts_and_runtime_descriptors() -> Result<(), Box<dyn Error>> {
    isolated(
        concat!(
            module_path!(),
            "::fixture_teardown_releases_worlds_mounts_and_runtime_descriptors"
        ),
        |teardown| {
            // Tokio keeps its Unix signal socket pair for the lifetime of the process.
            let warm = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            drop(warm);
            let baseline = fd_count()?;
            for multithreaded in [false, true] {
                for _ in 0..16 {
                    let mut builder = if multithreaded {
                        let mut builder = tokio::runtime::Builder::new_multi_thread();
                        builder.worker_threads(2);
                        builder
                    } else {
                        tokio::runtime::Builder::new_current_thread()
                    };
                    let runtime = builder.enable_all().build()?;
                    let (server_ref, world_ref, player_ref) = runtime.block_on(async {
                        // Registration on a worker must be visible to teardown on the main task.
                        let fixture = tokio::spawn(async { Fixture::new() }).await.unwrap();
                        let weak = (
                            Arc::downgrade(&fixture.server),
                            Arc::downgrade(&fixture.world),
                            Arc::downgrade(&fixture.player.player),
                        );
                        if teardown {
                            if multithreaded {
                                fixture.player.player.remove().await;
                            }
                            finish().await;
                            assert!(fixture.client_task_finished.load(Ordering::Acquire));
                        }
                        weak
                    });
                    drop(runtime);
                    assert_eq!(
                        fd_count()?,
                        baseline,
                        "fixture retained runtime or storage descriptors"
                    );
                    assert!(
                        server_ref.upgrade().is_none(),
                        "fixture retained its server"
                    );
                    assert!(world_ref.upgrade().is_none(), "fixture retained its world");
                    assert!(
                        player_ref.upgrade().is_none(),
                        "fixture retained its mounted player"
                    );
                }
            }
            Ok(())
        },
    )
}
