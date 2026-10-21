use crate::{
    block::entities::piston::PistonBlockEntity,
    plugin::{BoxFuture, EventHandler, EventPriority, block::block_break::BlockBreakEvent},
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{Block, block_properties::StickyPistonLikeProperties, item::Item};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct CountFluidBreaks(Arc<AtomicUsize>);

impl EventHandler<BlockBreakEvent> for CountFluidBreaks {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        if event.block == &Block::WATER || event.block == &Block::LAVA {
            self.0.fetch_add(1, Relaxed);
        }
        Box::pin(async {})
    }
}

fn item_count(world: &World, item: &Item) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            if stack.item == item {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

fn power_and_flush(world: Arc<World>, piston: BlockPos) {
    let runtime = tokio::runtime::Handle::current();
    let (finished, wait) = crossbeam::channel::bounded(1);
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        world.set_block_state(
            &piston.offset(Vector3::new(-1, 0, 0)),
            Block::REDSTONE_BLOCK.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        world.flush_synced_block_events();
        finished.send(()).unwrap();
    }));
    wait.recv_timeout(std::time::Duration::from_secs(5))
        .expect("powered piston event did not finish");
}

fn finish_push(world: &Arc<World>, piston: BlockPos, destinations: [BlockPos; 2]) {
    assert!(
        StickyPistonLikeProperties::from_state_id(world.get_block_state_id(&piston)).extended,
        "dependent target cleanup must not reject an admitted extension"
    );
    let head = piston.offset(Vector3::new(1, 0, 0));
    for destination in destinations {
        let source = destination.offset(Vector3::new(-1, 0, 0));
        if source != head {
            assert_eq!(world.get_block(&source), &Block::AIR);
        }
        assert_eq!(world.get_block(&destination), &Block::MOVING_PISTON);
        let moving = world.get_block_entity(&destination).unwrap();
        assert_eq!(
            moving
                .as_any()
                .downcast_ref::<PistonBlockEntity>()
                .unwrap()
                .pushed_block_state
                .id,
            Block::SLIME_BLOCK.default_state.id,
        );
        for _ in 0..3 {
            moving.tick(world);
        }
        assert_eq!(world.get_block(&destination), &Block::SLIME_BLOCK);
    }
    let moving_head = world.get_block_entity(&head).unwrap();
    for _ in 0..3 {
        moving_head.tick(world);
    }
    assert_eq!(world.get_block(&head), &Block::PISTON_HEAD);
    assert_eq!(item_count(world, &Item::SLIME_BLOCK), 0);
}

async fn push_double_block(target: &Block, piston_at_upper_half: bool, drop: Option<&Item>) {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    for chunk in [Vector2::new(0, 0), Vector2::new(-1, 0)] {
        combat_test_support::publish_empty_chunk(&world, chunk);
    }
    world
        .active_chunks
        .write()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let piston = BlockPos::new(0, 64 + i32::from(piston_at_upper_half), 0);
    let flags = BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE;
    let piston_state = Block::PISTON
        .from_properties(&[("facing", "east"), ("extended", "false")])
        .to_state_id(&Block::PISTON);
    world.set_block_state(&piston, piston_state, flags);
    world.set_block_state(
        &BlockPos::new(2, 63, 0),
        Block::DIRT.default_state.id,
        flags,
    );
    for (y, half) in [(64, "lower"), (65, "upper")] {
        world.set_block_state(
            &BlockPos::new(1, y, 0),
            Block::SLIME_BLOCK.default_state.id,
            flags,
        );
        world.set_block_state(
            &BlockPos::new(2, y, 0),
            target
                .from_properties(&[("half", half)])
                .to_state_id(target),
            flags,
        );
    }

    let fluid_breaks = Arc::new(AtomicUsize::new(0));
    server.plugin_manager.register::<BlockBreakEvent, _>(
        Arc::new(CountFluidBreaks(fluid_breaks.clone())),
        EventPriority::Normal,
        true,
    );

    // PistonBaseBlock.neighborChanged/triggerEvent, including dependent target cleanup.
    power_and_flush(world.clone(), piston);
    finish_push(
        &world,
        piston,
        [BlockPos::new(2, 64, 0), BlockPos::new(2, 65, 0)],
    );
    assert_eq!(
        fluid_breaks.load(Relaxed),
        0,
        "cleared fluid is not broken again"
    );
    if let Some(item) = drop {
        assert_eq!(item_count(&world, item), 1, "normal paired-block stock");
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), server.shutdown())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_piston_extends_after_rose_bush_partner_cleanup() {
    push_double_block(&Block::ROSE_BUSH, false, Some(&Item::ROSE_BUSH)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_piston_extends_after_door_partner_cleanup() {
    push_double_block(&Block::OAK_DOOR, true, Some(&Item::OAK_DOOR)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_piston_extends_after_seagrass_partner_becomes_water() {
    push_double_block(&Block::TALL_SEAGRASS, false, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_piston_preserves_tripwire_connection_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    for chunk in [Vector2::new(0, 0), Vector2::new(-1, 0)] {
        combat_test_support::publish_empty_chunk(&world, chunk);
    }
    world
        .active_chunks
        .write()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let piston = BlockPos::new(0, 64, 0);
    let flags = BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE;
    let state = Block::PISTON
        .from_properties(&[("facing", "east"), ("extended", "false")])
        .to_state_id(&Block::PISTON);
    world.set_block_state(&piston, state, flags);
    for (z, connection) in [(0, "south"), (1, "north")] {
        world.set_block_state(
            &BlockPos::new(1, 64, z),
            Block::SLIME_BLOCK.default_state.id,
            flags,
        );
        world.set_block_state(
            &BlockPos::new(2, 63, z),
            Block::DIRT.default_state.id,
            flags,
        );
        world.set_block_state(
            &BlockPos::new(2, 64, z),
            Block::TRIPWIRE
                .from_properties(&[(connection, "true")])
                .to_state_id(&Block::TRIPWIRE),
            flags,
        );
    }
    // Removing the second string changes the first string's connection, not its block type.
    power_and_flush(world.clone(), piston);
    finish_push(
        &world,
        piston,
        [BlockPos::new(2, 64, 0), BlockPos::new(2, 64, 1)],
    );
    assert_eq!(item_count(&world, &Item::STRING), 2);
    tokio::time::timeout(std::time::Duration::from_secs(20), server.shutdown())
        .await
        .unwrap();
}
