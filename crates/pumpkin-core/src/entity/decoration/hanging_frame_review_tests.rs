#![expect(clippy::unwrap_used, reason = "Bounded real-entry regression fixtures")]

use super::super::{hanging_test_support::Fixture, item_frame::ItemFrameEntity};
use super::{
    BreakGate, BreakRecorder, CloneGate, DEADLINE, FrameCloneReentry, SUPPORT, attack, painting,
};
use crate::{
    entity::EntityBase,
    plugin::{EventPriority, hanging::HangingBreakEvent},
};
use pumpkin_data::{
    Block, BlockDirection, data_component_impl::CustomDataImpl, item::Item, item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    mpsc,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_tick_never_clones_unrelated_frame_components() {
    let fixture = Fixture::new();
    let painting = painting(&fixture);
    let support = BlockPos::new(4, 65, 10);
    fixture.set_block(support, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, support, BlockDirection::North, None);
    let armed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut stack = ItemStack::new(1, &Item::DIAMOND);
    stack.set_data_component(FrameCloneReentry {
        component: CustomDataImpl {
            data: NbtCompound::new(),
        },
        frame: Arc::downgrade(&frame),
        armed: armed.clone(),
        calls: calls.clone(),
        replacement: None,
        when_empty: None,
        gate: None,
    });
    let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    concrete.set_item(stack, false);
    armed.store(true, Ordering::SeqCst);
    // Facing changes also recalculate geometry, without cloning the displayed item.
    concrete.set_facing(BlockDirection::North);
    fixture.tick(101);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "geometry invoked component clone callbacks"
    );
    assert!(!concrete.is_fixed());
    assert!(!painting.get_entity().is_removed());
    assert!(!frame.get_entity().is_removed());
    armed.store(false, Ordering::SeqCst);
    fixture.finish().await;
}

fn assert_tracked_item(frame: &dyn EntityBase, item: &'static Item) {
    use pumpkin_protocol::{
        codec::item_stack_seralizer::ItemStackSerializer, java::client::play::Metadata,
    };
    let mut expected = Vec::new();
    Metadata::new(
        pumpkin_data::tracked_data::item_frame::ITEM,
        ItemStackSerializer::from(ItemStack::new(1, item)),
    )
    .write(&mut expected, &CURRENT_MC_VERSION)
    .unwrap();
    expected.push(255);
    assert_eq!(
        frame
            .get_entity()
            .synched_data
            .pack_dirty_for_version(&CURRENT_MC_VERSION)
            .unwrap()
            .as_ref(),
        expected,
        "tracked ITEM disagrees with the newest committed frame item",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_reentrant_frame_insertion_keeps_latest_metadata() {
    reentrant_frame_write(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_reentrant_frame_extraction_keeps_latest_metadata() {
    reentrant_frame_write(true).await;
}

async fn reentrant_frame_write(on_extract: bool) {
    let fixture = Fixture::new();
    fixture.set_block(SUPPORT, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
    let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    let armed = Arc::new(AtomicBool::new(!on_extract));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut stack = ItemStack::new(1, &Item::DIAMOND);
    stack.set_data_component(FrameCloneReentry {
        component: CustomDataImpl {
            data: NbtCompound::new(),
        },
        frame: Arc::downgrade(&frame),
        armed: armed.clone(),
        calls: calls.clone(),
        replacement: Some(&Item::EMERALD),
        when_empty: Some(on_extract),
        gate: None,
    });
    frame.get_entity().synched_data.clear_dirty_for_test();
    let target = frame.clone();
    let player = fixture.player.player.clone();
    let server = fixture.server.clone();
    let runtime = tokio::runtime::Handle::current();
    let (done_send, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        let concrete = target.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
        concrete.set_item(stack, false);
        if on_extract {
            target.get_entity().synched_data.clear_dirty_for_test();
            armed.store(true, Ordering::SeqCst);
            attack(&player, &server, target.as_ref());
        }
        done_send.send(()).unwrap();
    });
    done.recv_timeout(DEADLINE).unwrap();
    worker.join().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(concrete.get_item().item, &Item::EMERALD);
    assert_tracked_item(frame.as_ref(), &Item::EMERALD);
    assert_eq!(fixture.drops(&Item::DIAMOND), u32::from(on_extract));
    assert_eq!(fixture.drops(&Item::EMERALD), 0);
    assert!(!frame.get_entity().is_removed());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_filled_frame_hit_races_allowed_and_cancelled_support() {
    for cancel_support in [false, true] {
        let fixture = Fixture::new();
        fixture.set_block(SUPPORT, &Block::STONE);
        let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
        let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
        let (entered_send, entered) = mpsc::channel();
        let (release, receive) = mpsc::channel();
        let gate = Arc::new(CloneGate {
            first: AtomicBool::new(false),
            entered: entered_send,
            release: Mutex::new(receive),
        });
        let mut stack = ItemStack::new(1, &Item::DIAMOND);
        stack.set_data_component(FrameCloneReentry {
            component: CustomDataImpl {
                data: NbtCompound::new(),
            },
            frame: Arc::downgrade(&frame),
            armed: Arc::new(AtomicBool::new(false)),
            calls: Arc::new(AtomicUsize::new(0)),
            replacement: None,
            when_empty: None,
            gate: Some(gate.clone()),
        });
        concrete.set_item(stack, false);
        let recorder = Arc::new(BreakRecorder {
            cancel: AtomicU8::new(u8::from(cancel_support)),
            events: Mutex::new(Vec::new()),
        });
        fixture
            .server
            .plugin_manager
            .register::<HangingBreakEvent, _>(recorder.clone(), EventPriority::Normal, true);
        gate.first.store(true, Ordering::SeqCst);
        let player = fixture.player.player.clone();
        let server = fixture.server.clone();
        let target = frame.clone();
        let runtime = tokio::runtime::Handle::current();
        let (done_send, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _runtime = runtime.enter();
            attack(&player, &server, target.as_ref());
            done_send.send(()).unwrap();
        });
        entered.recv_timeout(DEADLINE).unwrap();
        fixture.set_block(SUPPORT, &Block::AIR);
        fixture.tick(101);
        assert_eq!(frame.get_entity().is_removed(), !cancel_support);
        assert_eq!(fixture.drops(&Item::DIAMOND), u32::from(!cancel_support));
        release.send(()).unwrap();
        done.recv_timeout(DEADLINE).unwrap();
        worker.join().unwrap();
        assert!(concrete.get_item().is_empty());
        assert_eq!(
            fixture.drops(&Item::DIAMOND),
            1,
            "filled-frame hit/support duplicated or lost contents"
        );
        recorder.cancel.store(0, Ordering::Relaxed);
        attack(&fixture.player.player, &fixture.server, frame.as_ref());
        fixture.tick(101);
        assert_eq!(fixture.drops(&Item::DIAMOND), 1);
        assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_explosion_and_support_drop_filled_frame_once() {
    use crate::world::explosion::{BlockInteraction, Explosion};
    let fixture = Fixture::new();
    fixture.set_block(SUPPORT, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
    frame
        .cast_any()
        .downcast_ref::<ItemFrameEntity>()
        .unwrap()
        .set_item(ItemStack::new(1, &Item::DIAMOND), false);
    let (entered_send, entered) = mpsc::channel();
    let (release, receive) = mpsc::channel();
    let gate = Arc::new(BreakGate {
        first: AtomicBool::new(true),
        entered: entered_send,
        release: Mutex::new(receive),
        cancel_first: false,
        calls: Mutex::new(Vec::new()),
        by_entity_calls: Mutex::new(Vec::new()),
    });
    fixture
        .server
        .plugin_manager
        .register::<HangingBreakEvent, _>(gate, EventPriority::Normal, true);
    let world = fixture.world.clone();
    let position = frame.get_entity().pos.load();
    let runtime = tokio::runtime::Handle::current();
    let (done_send, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        Explosion::new(2.0, position, BlockInteraction::Keep).explode(&world);
        done_send.send(()).unwrap();
    });
    entered.recv_timeout(DEADLINE).unwrap();
    fixture.set_block(SUPPORT, &Block::AIR);
    fixture.tick(101);
    assert!(frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    release.send(()).unwrap();
    done.recv_timeout(DEADLINE).unwrap();
    worker.join().unwrap();
    fixture.tick(101);
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    fixture.finish().await;
}
