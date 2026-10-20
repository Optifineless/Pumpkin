#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real unload regression uses bounded worker barriers and fixture setup"
)]

use crate::server::combat_test_support;
use pumpkin_data::dimension::Dimension;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use std::{
    future::{Future, poll_fn},
    sync::{Arc, mpsc},
    task::Poll,
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(10);
const POSITION: BlockPos = BlockPos::new(8, 64, 8);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_world_unload_keeps_nested_mutation_admission_open_until_outer_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let primary = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![primary.clone()]));
    let world = server.create_world("nested_leaf".into(), Dimension::OVERWORLD);
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let (entered, held) = mpsc::sync_channel(1);
    let (request_nested, begin_nested) = mpsc::sync_channel(1);
    let (nested_result, observed_nested) = mpsc::sync_channel(1);
    let (release, finish) = mpsc::sync_channel(1);
    let (completed, joined) = mpsc::sync_channel(1);
    let worker_world = world.clone();
    let worker = std::thread::spawn(move || {
        let outer = worker_world
            .reserve_block_entity_mutation(POSITION)
            .expect("resident source must admit the outer mutation");
        entered.send(()).unwrap();
        begin_nested.recv_timeout(DEADLINE).unwrap();
        let nested = worker_world.reserve_block_entity_mutation(POSITION);
        nested_result.send(nested.is_some()).unwrap();
        drop(nested);
        finish.recv_timeout(DEADLINE).unwrap();
        drop(outer);
        completed.send(()).unwrap();
    });
    held.recv_timeout(DEADLINE).unwrap();
    let mut unload = Box::pin(server.unload_world("nested_leaf"));
    let first_poll = poll_fn(|context| Poll::Ready(unload.as_mut().poll(context))).await;
    assert!(
        first_poll.is_pending(),
        "real unload must defer while the outer mutation owns admission"
    );
    assert!(
        !world
            .pending_block_entity_loads
            .lock()
            .unwrap()
            .stop_admission,
        "deferral must leave nested synchronous leaf admission open"
    );
    request_nested.send(()).unwrap();
    let nested_was_admitted = observed_nested.recv_timeout(DEADLINE).unwrap();
    release.send(()).unwrap();
    joined.recv_timeout(DEADLINE).unwrap();
    worker.join().unwrap();
    assert!(
        nested_was_admitted,
        "an admitted outer operation must still reserve its nested leaf"
    );
    tokio::time::timeout(DEADLINE, unload)
        .await
        .expect("unload must finish after synchronous mutations drain")
        .unwrap();
    assert!(world.level.cancel_token.is_cancelled());
    assert!(
        !server
            .worlds
            .load()
            .iter()
            .any(|current| Arc::ptr_eq(current, &world))
    );
    world.level.world_portal.store(Arc::new(None));
    tokio::time::timeout(DEADLINE, primary.level.shutdown())
        .await
        .unwrap()
        .unwrap();
}
