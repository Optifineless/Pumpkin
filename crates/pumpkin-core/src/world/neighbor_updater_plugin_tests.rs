use super::{tests::*, *};
use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::block::block_physics::BlockPhysicsEvent,
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

struct WorkerMutation {
    world: Arc<World>,
    trigger: BlockPos,
    target: BlockPos,
    completed: Arc<AtomicBool>,
    propagate: bool,
    notify_all: bool,
}
impl EventHandler<BlockPhysicsEvent> for WorkerMutation {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.block_pos != self.trigger {
                return;
            }
            // Capture while polling the handler, then carry it through an executor/worker hop.
            // Task context must survive polling without the original thread-local context.
            let context = NeighborUpdateContext::default().with(NeighborUpdateContext::capture);
            let world = self.world.clone();
            let target = self.target;
            let propagate = self.propagate;
            let notify_all = self.notify_all;
            let (sent, received) = tokio::sync::oneshot::channel();
            let worker = server.runtime.spawn(async move {
                let operation = || {
                    if notify_all {
                        world.set_block_state(
                            &target,
                            Block::DIRT.default_state.id,
                            BlockFlags::NOTIFY_ALL,
                        );
                    } else {
                        world.replace_with_state_for_neighbor_update(
                            &target,
                            BlockDirection::East,
                            BlockFlags::FORCE_STATE,
                        );
                    }
                };
                if propagate {
                    context.with(operation);
                } else {
                    operation();
                }
                let _ = sent.send(());
            });
            // Reverted code must fail without hanging the test process forever.
            let done = matches!(
                tokio::time::timeout(Duration::from_secs(4), received).await,
                Ok(Ok(()))
            );
            self.completed.store(done, Ordering::Relaxed);
            if done {
                worker.await.unwrap();
            }
        })
    }
}

async fn native_worker(propagate: bool, notify_all: bool) {
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let target = BlockPos::new(8, 64, 8);
    let trigger = BlockPos::new(2, 64, 2);
    let world = probe_world(
        &fixture,
        Arc::new(|_| {}),
        Arc::new(|_| Block::DIRT.default_state.id),
    );
    stone(&world, target);
    stone(&world, trigger);
    let completed = Arc::new(AtomicBool::new(false));
    fixture
        .server
        .plugin_manager
        .register::<BlockPhysicsEvent, _>(
            Arc::new(WorkerMutation {
                world: world.clone(),
                trigger,
                target,
                completed: completed.clone(),
                propagate,
                notify_all,
            }),
            EventPriority::Normal,
            true,
        );
    let (waiting, waiting_rx) = mpsc::channel();
    world.neighbor_updates.lock().unwrap().waiting_probe = Some(waiting);
    world.update_neighbor(&trigger, &Block::STONE);
    assert!(
        completed.load(Ordering::Relaxed),
        "native worker waited for its own cascade"
    );
    if propagate {
        assert!(
            waiting_rx.try_recv().is_err(),
            "context propagation fell back to the timeout"
        );
    }
    assert_eq!(world.get_block(&target), &Block::DIRT);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_native_handler_future_propagates_cascade_to_spawned_task() {
    native_worker(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_native_handler_unscoped_worker_has_bounded_wait() {
    native_worker(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf4_unscoped_notify_all_worker_falls_back_once_per_cascade() {
    native_worker(false, true).await;
}

async fn cross_world_cascades(notify_all: bool) {
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(8, 64, 8);
    let owners_entered = Arc::new(std::sync::Barrier::new(3));
    let release = Arc::new(std::sync::Barrier::new(3));
    let targets = Arc::new(Mutex::new(Vec::<Arc<World>>::new()));
    let completed = Arc::new(AtomicBool::new(false));
    let worlds: Vec<_> = (0..2)
        .map(|index| {
            let entered = owners_entered.clone();
            let release = release.clone();
            let targets = targets.clone();
            let completed = completed.clone();
            let called = AtomicBool::new(false);
            probe_world(
                &fixture,
                Arc::new(move |args| {
                    if *args.position != pos || called.swap(true, Ordering::Relaxed) {
                        return;
                    }
                    entered.wait();
                    let other = targets.lock().unwrap()[1 - index].clone();
                    let (sent, received) = mpsc::channel();
                    let worker = std::thread::spawn(move || {
                        if notify_all {
                            other.set_block_state(
                                &pos,
                                Block::DIRT.default_state.id,
                                BlockFlags::NOTIFY_ALL | BlockFlags::FORCE_STATE,
                            );
                        } else {
                            other.replace_with_state_for_neighbor_update(
                                &pos,
                                BlockDirection::East,
                                BlockFlags::FORCE_STATE,
                            );
                        }
                        let _ = sent.send(());
                    });
                    let done = received.recv_timeout(Duration::from_secs(4)).is_ok();
                    completed.fetch_or(done, Ordering::Relaxed);
                    release.wait();
                    if done {
                        worker.join().unwrap();
                    }
                    assert!(done, "cross-world callback waited for the other cascade");
                    assert_eq!(*args.position, pos);
                }),
                Arc::new(|_| Block::DIRT.default_state.id),
            )
        })
        .collect();
    *targets.lock().unwrap() = worlds.clone();
    for world in &worlds {
        stone(world, pos);
    }
    std::thread::scope(|scope| {
        let owners: Vec<_> = worlds
            .iter()
            .map(|world| scope.spawn(move || world.update_neighbor(&pos, &Block::STONE)))
            .collect();
        owners_entered.wait();
        release.wait();
        for owner in owners {
            owner.join().unwrap();
        }
    });
    assert!(completed.load(Ordering::Relaxed));
    for world in &worlds {
        assert_eq!(world.get_block(&pos), &Block::DIRT);
    }
    targets.lock().unwrap().clear();
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_cross_world_cascades_have_bounded_wait() {
    cross_world_cascades(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf4_cross_world_notify_all_falls_back_once_per_cascade() {
    cross_world_cascades(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_same_thread_cascade_captures_context_only_at_entry() {
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let world = probe_world(
        &fixture,
        Arc::new(move |args| {
            if counted.fetch_add(1, Ordering::Relaxed) < 64 {
                args.world.update_neighbor(args.position, &Block::STONE);
            }
        }),
        Arc::new(|args| args.state_id),
    );
    let pos = BlockPos::new(8, 64, 8);
    stone(&world, pos);
    let before = NeighborUpdateContext::capture_count();
    world.update_neighbor(&pos, &Block::STONE);
    assert_eq!(NeighborUpdateContext::capture_count() - before, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 65);
    fixture.server.shutdown().await;
}

struct CheckScopedOwner {
    world: Arc<World>,
    cascade: Arc<AtomicU64>,
}
impl CheckScopedOwner {
    fn check(&self) {
        // Clear thread context to model the future moving onto another executor thread.
        assert!(NeighborUpdateContext::default().with(|| {
            NeighborUpdateContext::capture()
                .contains(&self.world, self.cascade.load(Ordering::Relaxed))
        }));
    }
}
impl EventHandler<BlockPhysicsEvent> for CheckScopedOwner {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move { self.check() })
    }
    fn handle<'a>(&'a self, _: &'a Arc<Server>, _: &'a BlockPhysicsEvent) -> BoxFuture<'a, ()> {
        Box::pin(async move { self.check() })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf4_scoped_physics_handlers_reuse_owner_without_recapturing() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let cascade = NeighborUpdateContext::next_cascade();
    let expected = Arc::new(AtomicU64::new(cascade));
    for blocking in [true, false] {
        fixture
            .server
            .plugin_manager
            .register::<BlockPhysicsEvent, _>(
                Arc::new(CheckScopedOwner {
                    world: world.clone(),
                    cascade: expected.clone(),
                }),
                EventPriority::Normal,
                blocking,
            );
    }
    let pos = BlockPos::new(8, 64, 8);
    let context =
        NeighborUpdateContext::in_cascade(&world, cascade, NeighborUpdateContext::capture);
    context
        .scope(async {
            let before = NeighborUpdateContext::capture_count();
            fixture
                .server
                .plugin_manager
                .fire(&fixture.server, &mut BlockPhysicsEvent::new(pos, pos))
                .await;
            // Only the handlers' ownership checks capture; dispatch must reuse the task scope.
            assert_eq!(NeighborUpdateContext::capture_count() - before, 2);
            // A newly entered synchronous cascade still needs to be added to the task scope.
            let nested = NeighborUpdateContext::next_cascade();
            expected.store(nested, Ordering::Relaxed);
            NeighborUpdateContext::in_cascade(&world, nested, || {
                let before = NeighborUpdateContext::capture_count();
                fixture
                    .server
                    .plugin_manager
                    .fire_blocking(&fixture.server, &mut BlockPhysicsEvent::new(pos, pos));
                assert_eq!(NeighborUpdateContext::capture_count() - before, 3);
            });
        })
        .await;
    fixture.server.shutdown().await;
}
