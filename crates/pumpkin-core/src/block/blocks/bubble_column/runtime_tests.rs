use super::runtime_test_support::{RuntimeFixture, SUPPORT, bubble, cell, water, waterlogged_slab};
use pumpkin_data::{Block, BlockStateId, block_properties::WaterLikeProperties, fluid::Fluid};
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::Ordering::Relaxed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_support_replacement_initializes_settled_sources() {
    let mut results = Vec::new();
    for (support, drag_down) in [(&Block::MAGMA_BLOCK, true), (&Block::SOUL_SAND, false)] {
        let control = RuntimeFixture::new(support, &[BlockStateId::AIR]);
        control.put(0, water(), BlockFlags::NOTIFY_ALL);
        control.advance(19);
        let control_before = control.states();
        control.advance(1);
        let control_after = control.states();
        control.finish().await;

        let fixture = RuntimeFixture::settled_water(3);
        fixture.replace_support(support, BlockFlags::NOTIFY_ALL);
        fixture.advance(19);
        let before = fixture.states();
        fixture.advance(1);
        let after = fixture.states();
        fixture.finish().await;
        results.push((
            support.name,
            drag_down,
            control_before,
            control_after,
            before,
            after,
        ));
    }
    for (name, drag_down, control_before, control_after, before, after) in results {
        assert_eq!(
            control_before,
            vec![water()],
            "support-first timing: {name}"
        );
        assert_eq!(
            control_after,
            vec![bubble(drag_down)],
            "support-first control: {name}"
        );
        assert_eq!(
            before,
            vec![water(); 3],
            "column formed before tick 20: {name}"
        );
        assert_eq!(
            after,
            vec![bubble(drag_down); 3],
            "settled water at tick 20: {name}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_neighbor_callback_uses_current_support() {
    let fixture = RuntimeFixture::settled_water(1);
    let monitor = fixture.watch_physics();
    fixture.replace_support(
        &Block::MAGMA_BLOCK,
        BlockFlags::NOTIFY_NEIGHBORS | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let calls = monitor.support_calls.load(Relaxed);
    let block_queued = fixture
        .world
        .is_block_tick_scheduled(&cell(0), &Block::WATER);
    let fluid_queued = fixture
        .world
        .is_fluid_tick_scheduled(&cell(0), &Fluid::FLOWING_WATER);
    fixture.advance(19);
    let before = fixture.states();
    fixture.advance(1);
    let after = fixture.states();
    fixture.finish().await;

    assert_eq!(
        calls, 1,
        "replacement must reach the ordinary neighbor callback"
    );
    assert!(
        fluid_queued,
        "the uncancelled fluid callback is a positive control"
    );
    assert_eq!(before, vec![water()]);
    assert_eq!(
        after,
        vec![bubble(true)],
        "the callback must read the current support below"
    );
    assert!(block_queued);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_shape_update_propagates_without_physics_notifications() {
    let fixture = RuntimeFixture::settled_water(3);
    let monitor = fixture.watch_physics();
    fixture.replace_support(&Block::SOUL_SAND, BlockFlags::NOTIFY_LISTENERS);
    let initial_calls = monitor.support_calls.load(Relaxed);
    fixture.advance(19);
    let before = fixture.states();
    fixture.advance(1);
    let after = fixture.states();
    let support_calls = monitor.support_calls.load(Relaxed);
    let propagation_calls = monitor.propagation_calls.load(Relaxed);
    fixture.finish().await;

    assert_eq!(
        initial_calls, 0,
        "support replacement must use only the shape route"
    );
    assert_eq!(before, vec![water(); 3]);
    assert_eq!(
        after,
        vec![bubble(false); 3],
        "DOWN shape change must initialize the whole shaft"
    );
    assert_eq!(support_calls, 0);
    assert_eq!(
        propagation_calls, 0,
        "BubbleColumnBlock.updateColumn uses listener/shape writes, without ordinary physics notifications"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_cancelled_neighbor_blocks_scheduling_then_allowed_update_forms() {
    let fixture = RuntimeFixture::settled_water(1);
    let monitor = fixture.watch_physics();
    fixture.replace_support(&Block::MAGMA_BLOCK, BlockFlags::UPDATE_KNOWN_SHAPE);
    let quiet = fixture.queues_empty() && monitor.support_calls.load(Relaxed) == 0;
    monitor.cancel_support.store(true, Relaxed);
    fixture
        .world
        .update_neighbors_at(&SUPPORT, &Block::MAGMA_BLOCK, None);
    let cancelled_calls = monitor.support_calls.load(Relaxed);
    let cancelled_queues_empty = fixture.queues_empty();
    fixture.advance(20);
    let cancelled_state = fixture.states();

    monitor.cancel_support.store(false, Relaxed);
    fixture
        .world
        .update_neighbors_at(&SUPPORT, &Block::MAGMA_BLOCK, None);
    let allowed_calls = monitor.support_calls.load(Relaxed);
    let allowed_block = fixture
        .world
        .is_block_tick_scheduled(&cell(0), &Block::WATER);
    let allowed_fluid = fixture
        .world
        .is_fluid_tick_scheduled(&cell(0), &Fluid::FLOWING_WATER);
    fixture.advance(19);
    let before = fixture.states();
    fixture.advance(1);
    let after = fixture.states();
    fixture.finish().await;

    assert!(
        quiet,
        "quiet support replacement must leave no competing shape/placement tick"
    );
    assert_eq!(cancelled_calls, 1);
    assert!(
        cancelled_queues_empty,
        "cancellation must block both block and fluid callbacks"
    );
    assert_eq!(cancelled_state, vec![water()]);
    assert_eq!(allowed_calls, 2);
    assert!(
        allowed_block && allowed_fluid,
        "the same uncancelled notification must reach both callbacks"
    );
    assert_eq!(before, vec![water()]);
    assert_eq!(after, vec![bubble(true)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_source_added_above_column_extends_whole_run_at_five() {
    let fixture = RuntimeFixture::new(
        &Block::MAGMA_BLOCK,
        &[bubble(true), Block::STONE.default_state.id, water()],
    );
    fixture.put(1, water(), BlockFlags::NOTIFY_LISTENERS);
    let bubble_queued = fixture
        .world
        .is_block_tick_scheduled(&cell(0), &Block::BUBBLE_COLUMN);
    let fluid_queued = fixture
        .world
        .is_fluid_tick_scheduled(&cell(0), &Fluid::FLOWING_WATER);
    fixture.advance(4);
    let before = fixture.states();
    fixture.advance(1);
    let after = fixture.states();
    fixture.finish().await;

    assert_eq!(before, vec![bubble(true), water(), water()]);
    assert_eq!(
        after,
        vec![bubble(true); 3],
        "UP shape notification must extend at tick 5, not the later water-placement deadline"
    );
    assert!(
        bubble_queued && fluid_queued,
        "the existing bubble must schedule block and normalized water ticks"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_support_reversal_and_removal_reconcile_whole_column_at_five() {
    let mut results = Vec::new();
    for (initial_support, initial_drag, replacement, expected) in [
        (&Block::MAGMA_BLOCK, true, &Block::SOUL_SAND, bubble(false)),
        (&Block::SOUL_SAND, false, &Block::MAGMA_BLOCK, bubble(true)),
        (&Block::MAGMA_BLOCK, true, &Block::STONE, water()),
    ] {
        let original = vec![bubble(initial_drag); 3];
        let fixture = RuntimeFixture::new(initial_support, &original);
        fixture.replace_support(replacement, BlockFlags::NOTIFY_ALL);
        fixture.advance(4);
        let before = fixture.states();
        fixture.advance(1);
        let after = fixture.states();
        fixture.finish().await;
        results.push((replacement.name, original, before, after, expected));
    }
    for (name, original, before, after, expected) in results {
        assert_eq!(before, original, "changed before tick 5: {name}");
        assert_eq!(
            after,
            vec![expected; 3],
            "only part of the column reconciled at tick 5: {name}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_barriers_and_replaced_queued_water_remain_intact() {
    let flowing = WaterLikeProperties { level: 1 }.to_state_id(&Block::WATER);
    let falling = WaterLikeProperties { level: 8 }.to_state_id(&Block::WATER);
    for (name, barrier, non_source) in [
        ("flowing", flowing, true),
        ("falling", falling, true),
        ("waterlogged slab", waterlogged_slab(), false),
        ("solid", Block::STONE.default_state.id, false),
        ("air endpoint", BlockStateId::AIR, false),
    ] {
        let mut states = vec![BlockStateId::AIR, barrier];
        if barrier != BlockStateId::AIR {
            states.push(water());
        }
        let fixture = RuntimeFixture::new(&Block::MAGMA_BLOCK, &states);
        // Keep the boundary quiet while queuing the actual WATER placement callback below it.
        fixture.put(0, water(), BlockFlags::UPDATE_KNOWN_SHAPE);
        fixture.advance(19);
        let before = fixture.states();
        let source_before = fixture
            .world
            .get_fluid_and_fluid_state(&cell(1))
            .1
            .is_source;
        fixture.advance(1);
        let after = fixture.states();
        let source_after = fixture
            .world
            .get_fluid_and_fluid_state(&cell(1))
            .1
            .is_source;
        fixture.finish().await;

        assert_eq!(before[0], water(), "formation timing: {name}");
        assert_eq!(after[0], bubble(true), "positive formation control: {name}");
        if non_source {
            assert!(
                !source_before && !source_after,
                "fixture must retain non-source fluid: {name}"
            );
            assert!(
                after[1].to_block() == &Block::WATER,
                "flowing boundary became a bubble: {name}"
            );
        } else {
            assert_eq!(after[1], barrier, "occupied boundary changed: {name}");
        }
        if after.len() == 3 {
            assert_eq!(
                after[2],
                water(),
                "propagation crossed the boundary: {name}"
            );
        }
    }

    let fixture = RuntimeFixture::new(&Block::MAGMA_BLOCK, &[BlockStateId::AIR]);
    fixture.put(0, water(), BlockFlags::UPDATE_KNOWN_SHAPE);
    let queued = fixture
        .world
        .is_block_tick_scheduled(&cell(0), &Block::WATER);
    fixture.put(
        0,
        Block::STONE.default_state.id,
        BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    fixture.advance(20);
    let after = fixture.states();
    let drained = fixture.queues_empty();
    fixture.finish().await;
    assert!(
        queued,
        "replacement must invalidate a real pending WATER tick"
    );
    assert_eq!(after, vec![Block::STONE.default_state.id]);
    assert!(drained);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue62_runtime_first_unchanged_water_continues_but_later_unchanged_water_stops() {
    let fixture = RuntimeFixture::new(
        &Block::MAGMA_BLOCK,
        &[BlockStateId::AIR, bubble(true), water(), bubble(true)],
    );
    // UPDATE_KNOWN_SHAPE retains the real placement callback while keeping the upper cells quiet.
    fixture.put(0, water(), BlockFlags::UPDATE_KNOWN_SHAPE);
    let queued = fixture
        .world
        .is_block_tick_scheduled(&cell(0), &Block::WATER);
    fixture.replace_support(&Block::STONE, BlockFlags::UPDATE_KNOWN_SHAPE);
    fixture.advance(19);
    let before = fixture.states();
    fixture.advance(1);
    let after = fixture.states();
    fixture.finish().await;

    assert!(queued);
    assert_eq!(before, vec![water(), bubble(true), water(), bubble(true)]);
    // BubbleColumnBlock.updateColumn ignores the first unchanged write, then stops on a later one.
    assert_eq!(after, vec![water(), water(), water(), bubble(true)]);
}
