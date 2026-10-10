use super::*;
use crate::{
    block::{BlockBehaviour, GetStateForNeighborUpdateArgs},
    entity::death_test_world::DeathTestWorld,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;
use pumpkin_macros::pumpkin_block;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

type NeighborCallback = Arc<dyn Fn(&OnNeighborUpdateArgs<'_>) + Send + Sync>;
type ShapeCallback = Arc<dyn Fn(&GetStateForNeighborUpdateArgs<'_>) -> BlockStateId + Send + Sync>;

#[pumpkin_block("minecraft:stone")]
struct Probe {
    neighbor: NeighborCallback,
    shape: ShapeCallback,
}
impl BlockBehaviour for Probe {
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        (self.neighbor)(&args);
    }
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        (self.shape)(&args)
    }
}

pub(super) fn probe_world(
    fixture: &DeathTestWorld,
    neighbor: NeighborCallback,
    shape: ShapeCallback,
) -> Arc<World> {
    let original = fixture.world();
    let mut registry = crate::block::registry::BlockRegistry::default();
    registry.register(Probe { neighbor, shape });
    Arc::new(World::load(
        original.level.clone(),
        original.level_info.clone(),
        original.dimension.clone(),
        Arc::new(registry),
        Arc::downgrade(&fixture.server),
    ))
}
fn unchanged(args: &GetStateForNeighborUpdateArgs<'_>) -> BlockStateId {
    args.state_id
}
pub(super) fn stone(world: &Arc<World>, pos: BlockPos) {
    world.set_block_state(
        &pos,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_unchanged_barrel_probes_emit_no_comparator_updates() {
    use crate::{
        block::entities::{BlockEntity, barrel::BarrelBlockEntity},
        plugin::{
            BoxFuture, EventHandler, EventPriority,
            api::events::block::block_physics::BlockPhysicsEvent,
        },
        server::Server,
    };
    use pumpkin_inventory::Inventory;
    struct PhysicsCount(Arc<AtomicUsize>);
    impl EventHandler<BlockPhysicsEvent> for PhysicsCount {
        fn handle_blocking<'a>(
            &'a self,
            _: &'a Arc<Server>,
            _: &'a mut BlockPhysicsEvent,
        ) -> BoxFuture<'a, ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {})
        }
    }
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(8, 64, 8);
    let barrel = Arc::new(BarrelBlockEntity::new(pos));
    world.set_block_state(
        &pos,
        Block::BARREL.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.add_block_entity(barrel.clone());
    let comparator = pos.offset(pumpkin_util::math::vector3::Vector3::new(1, 0, 0));
    world.set_block_state(
        &comparator,
        Block::COMPARATOR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let updates = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockPhysicsEvent, _>(
            Arc::new(PhysicsCount(updates.clone())),
            EventPriority::Normal,
            true,
        );
    barrel.clear_dirty();
    barrel.clear_comparator_dirty();
    let entities: [Arc<dyn BlockEntity>; 1] = [barrel.clone()];
    for _ in 0..100 {
        barrel.update_slot(0, &mut |_| {});
        world.flush_comparator_updates(&entities);
    }
    assert!(!barrel.is_dirty());
    assert!(!barrel.is_comparator_dirty());
    assert_eq!(updates.load(Ordering::Relaxed), 0);
    // Confirm the observation sees a comparator notification after a real mutation.
    barrel.update_slot(0, &mut |stack| {
        *stack = pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::DIAMOND);
    });
    world.flush_comparator_updates(&entities);
    assert!(updates.load(Ordering::Relaxed) > 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn neighbor_multi_update_counts_once_and_resumes_depth_first() {
    let fixture = DeathTestWorld::with_neighbor_limit(2).await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let positions = seen.clone();
    let center = BlockPos::new(8, 64, 8);
    let child = BlockPos::new(2, 64, 2);
    let world = probe_world(
        &fixture,
        Arc::new(move |args| {
            positions.lock().unwrap().push(*args.position);
            if *args.position == center.offset(BlockDirection::West.to_offset()) {
                args.world.update_neighbor(&child, &Block::STONE);
            }
        }),
        Arc::new(unchanged),
    );
    stone(&world, child);
    for direction in BlockDirection::update_order() {
        stone(&world, center.offset(direction.to_offset()));
    }
    world.update_neighbors_at(&center, &Block::STONE, None);
    let mut expected: Vec<_> = [
        BlockDirection::West,
        BlockDirection::East,
        BlockDirection::Down,
        BlockDirection::Up,
        BlockDirection::North,
        BlockDirection::South,
    ]
    .iter()
    .map(|d| center.offset(d.to_offset()))
    .collect();
    expected.insert(1, child);
    assert_eq!(*seen.lock().unwrap(), expected);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn neighbor_budgets_are_per_world() {
    let fixture = DeathTestWorld::with_neighbor_limit(1).await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    let other = probe_world(
        &fixture,
        Arc::new(move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
        }),
        Arc::new(unchanged),
    );
    let source = probe_world(
        &fixture,
        Arc::new(move |args| {
            other.update_neighbor(args.position, &Block::STONE);
        }),
        Arc::new(unchanged),
    );
    let pos = BlockPos::new(8, 64, 8);
    stone(&source, pos);
    source.update_neighbor(&pos, &Block::STONE);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shape_update_uses_enqueued_neighbor_state() {
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let target = BlockPos::new(8, 64, 8);
    let trigger = BlockPos::new(2, 64, 2);
    let captured = Arc::new(Mutex::new(Vec::new()));
    let records = captured.clone();
    let world = probe_world(
        &fixture,
        Arc::new(move |args| {
            args.world.replace_with_state_for_neighbor_update(
                &target,
                BlockDirection::East,
                BlockFlags::FORCE_STATE,
            );
            args.world.set_block_state(
                &target.offset(BlockDirection::East.to_offset()),
                BlockStateId::AIR,
                BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }),
        Arc::new(move |args| {
            records.lock().unwrap().push(args.neighbor_state_id);
            args.state_id
        }),
    );
    for pos in [
        target,
        trigger,
        target.offset(BlockDirection::East.to_offset()),
    ] {
        stone(&world, pos);
    }
    captured.lock().unwrap().clear();
    world.update_neighbor(&trigger, &Block::STONE);
    assert_eq!(*captured.lock().unwrap(), [Block::STONE.default_state.id]);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cascade_panic_cleanup_allows_next_update() {
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let first = Arc::new(AtomicBool::new(true));
    let flag = first.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let world = probe_world(
        &fixture,
        Arc::new(move |_| {
            assert!(!flag.swap(false, Ordering::Relaxed), "block callback panic");
            count.fetch_add(1, Ordering::Relaxed);
        }),
        Arc::new(unchanged),
    );
    let pos = BlockPos::new(8, 64, 8);
    stone(&world, pos);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || world.update_neighbor(&pos, &Block::STONE)
        ))
        .is_err()
    );
    world.update_neighbor(&pos, &Block::STONE);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deep_neighbor_cascade_does_not_overflow() {
    let fixture = DeathTestWorld::with_neighbor_limit(-1).await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let world = probe_world(
        &fixture,
        Arc::new(move |args| {
            if counted.fetch_add(1, Ordering::Relaxed) < 20_000 {
                args.world.update_neighbor(args.position, &Block::STONE);
            }
        }),
        Arc::new(unchanged),
    );
    let pos = BlockPos::new(8, 64, 8);
    stone(&world, pos);
    world.update_neighbor(&pos, &Block::STONE);
    assert_eq!(calls.load(Ordering::Relaxed), 20_001);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_neighbor_budget_suppresses_callbacks() {
    let fixture = DeathTestWorld::with_neighbor_limit(0).await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let world = probe_world(
        &fixture,
        Arc::new(move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
        }),
        Arc::new(unchanged),
    );
    let pos = BlockPos::new(8, 64, 8);
    stone(&world, pos);
    world.update_neighbors_at(&pos, &Block::STONE, None);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shape_update_limit_stops_replacement_propagation() {
    let fixture = DeathTestWorld::with_neighbor_limit(-1).await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    let world = probe_world(
        &fixture,
        Arc::new(|_| {}),
        Arc::new(move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
            Block::DIRT.default_state.id
        }),
    );
    let pos = BlockPos::new(8, 64, 8);
    stone(&world, pos);
    for direction in BlockDirection::all() {
        stone(&world, pos.offset(direction.to_offset()));
    }
    // An explicit depth of zero still applies this shape replacement, but no child shapes.
    world.replace_with_state_for_neighbor_update_with_limit(
        &pos,
        BlockDirection::East,
        BlockFlags::NOTIFY_ALL,
        0,
    );
    assert_eq!(world.get_block(&pos), &Block::DIRT);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_shape_submit_waits_until_its_update_is_applied() {
    use std::{
        sync::{Barrier, mpsc},
        time::Duration,
    };
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let callback_release = release.clone();
    let callback_entered = entered.clone();
    let target = BlockPos::new(8, 64, 8);
    let trigger = BlockPos::new(2, 64, 2);
    let world = probe_world(
        &fixture,
        Arc::new(move |_| {
            callback_entered.wait();
            callback_release.wait();
        }),
        Arc::new(|_| Block::DIRT.default_state.id),
    );
    stone(&world, target);
    stone(&world, trigger);
    let (waiting, waiting_rx) = mpsc::channel();
    world.neighbor_updates.lock().unwrap().waiting_probe = Some(waiting);
    let (returned, returned_rx) = mpsc::channel();
    let early_return = std::thread::scope(|scope| {
        let first = scope.spawn(|| world.update_neighbor(&trigger, &Block::STONE));
        entered.wait();
        let second = scope.spawn(|| {
            world.replace_with_state_for_neighbor_update(
                &target,
                BlockDirection::East,
                BlockFlags::FORCE_STATE,
            );
            returned.send(world.get_block_state_id(&target)).unwrap();
        });
        waiting_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let early = returned_rx.recv_timeout(Duration::from_millis(100)).ok();
        release.wait();
        first.join().unwrap();
        second.join().unwrap();
        if early.is_none() {
            assert_eq!(returned_rx.recv().unwrap(), Block::DIRT.default_state.id);
        }
        early
    });
    assert!(
        early_return.is_none(),
        "unrelated submission returned before shape update"
    );
    assert_eq!(world.get_block(&target), &Block::DIRT);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn callback_worker_shape_submission_is_recursive_and_does_not_deadlock() {
    use std::{sync::mpsc, time::Duration};
    let fixture = DeathTestWorld::new().await;
    publish(&fixture.world(), proto(&Biome::PLAINS, &Block::STONE));
    let target = BlockPos::new(8, 64, 8);
    let trigger = BlockPos::new(2, 64, 2);
    let returned = Arc::new(AtomicBool::new(false));
    let observed = returned.clone();
    let workers = Arc::new(Mutex::new(Vec::new()));
    let handles = workers.clone();
    let world = probe_world(
        &fixture,
        Arc::new(move |args| {
            let context = NeighborUpdateContext::capture();
            let world = args.world.clone();
            let (done, received) = mpsc::channel();
            handles.lock().unwrap().push(std::thread::spawn(move || {
                context.with(|| {
                    world.replace_with_state_for_neighbor_update(
                        &target,
                        BlockDirection::East,
                        BlockFlags::FORCE_STATE,
                    );
                });
                done.send(()).unwrap();
            }));
            // The cascade waits for a plugin worker, so that worker must enqueue and return.
            observed.store(
                received.recv_timeout(Duration::from_secs(5)).is_ok(),
                Ordering::Relaxed,
            );
        }),
        Arc::new(|_| Block::DIRT.default_state.id),
    );
    stone(&world, trigger);
    stone(&world, target);
    world.update_neighbor(&trigger, &Block::STONE);
    for worker in workers.lock().unwrap().drain(..) {
        worker.join().unwrap();
    }
    assert!(
        returned.load(Ordering::Relaxed),
        "callback worker waited for its own cascade"
    );
    assert_eq!(world.get_block(&target), &Block::DIRT);
    fixture.server.shutdown().await;
}
