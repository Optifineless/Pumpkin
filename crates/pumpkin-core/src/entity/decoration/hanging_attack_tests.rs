#![expect(
    clippy::unwrap_used,
    reason = "Focused regressions use valid packet fixtures and bounded callback gates"
)]

use super::{hanging_test_support::Fixture, item_frame::ItemFrameEntity};
use crate::{
    entity::{EntityBase, player::Player},
    net::ClientPlatform,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        hanging::{HangingBreakByEntityEvent, HangingBreakEvent},
    },
    server::Server,
};
use pumpkin_data::{
    Block, BlockDirection,
    data_component::DataComponent,
    data_component_impl::{CustomDataImpl, DataComponentImpl},
    item::Item,
    item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
    painting_variant::PaintingVariant,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    ClientPacket, RawPacket,
    java::server::play::{SAttack, SInteract},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{GameMode, Hand, math::position::BlockPos};
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const SUPPORT: BlockPos = BlockPos::new(8, 65, 10);
const DEADLINE: Duration = Duration::from_secs(5);

fn dispatch(player: &Arc<Player>, server: &Arc<Server>, packet: &impl ClientPacket, id: i32) {
    let mut bytes = Vec::new();
    packet
        .write_packet_data(&mut bytes, &CURRENT_MC_VERSION)
        .unwrap();
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java hanging regression requires a Java player");
    };
    client
        .handle_play_packet(
            player,
            server,
            &RawPacket {
                id,
                payload: bytes.into(),
            },
        )
        .unwrap();
    assert!(
        !client.is_closed(),
        "the real attack packet closed the client"
    );
}

fn attack(player: &Arc<Player>, server: &Arc<Server>, entity: &dyn EntityBase) {
    player.last_attacked_ticks.store(100, Ordering::Relaxed);
    dispatch(
        player,
        server,
        &SAttack {
            entity_id: entity.get_entity().entity_id.into(),
        },
        SAttack::to_id(CURRENT_MC_VERSION),
    );
}

fn painting(fixture: &Fixture) -> Arc<dyn EntityBase> {
    fixture.set_block(SUPPORT, &Block::STONE);
    fixture.place(
        &Item::PAINTING,
        SUPPORT,
        BlockDirection::North,
        Some(PaintingVariant::Alban),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_java_painting_attack_uses_survival_and_creative_attacker() {
    for (mode, expected_drops) in [(GameMode::Survival, 1), (GameMode::Creative, 0)] {
        let fixture = Fixture::new();
        let painting = painting(&fixture);
        if mode == GameMode::Creative {
            assert!(fixture.player.player.set_gamemode(mode));
        }
        assert!(!painting.get_entity().is_removed());
        assert_eq!(fixture.drops(&Item::PAINTING), 0);
        attack(&fixture.player.player, &fixture.server, painting.as_ref());
        assert!(
            painting.get_entity().is_removed(),
            "{mode:?} attack was ignored"
        );
        assert_eq!(
            fixture.drops(&Item::PAINTING),
            expected_drops,
            "painting drop attributed to the victim instead of the {mode:?} attacker"
        );
        attack(&fixture.player.player, &fixture.server, painting.as_ref());
        fixture.tick(102);
        assert_eq!(fixture.drops(&Item::PAINTING), expected_drops);
        fixture.finish().await;
    }
}

#[derive(Debug, PartialEq)]
struct BreakSnapshot {
    by_entity: bool,
    hanging_id: i32,
    remover_id: Option<i32>,
    already_removed: bool,
}

struct BreakRecorder {
    cancel: AtomicU8,
    events: Mutex<Vec<BreakSnapshot>>,
}

impl EventHandler<HangingBreakEvent> for BreakRecorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut HangingBreakEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push(BreakSnapshot {
            by_entity: false,
            hanging_id: event.entity.get_entity().entity_id,
            remover_id: event
                .remover
                .as_ref()
                .map(|entity| entity.get_entity().entity_id),
            already_removed: event.entity.get_entity().is_removed(),
        });
        event.cancelled = self.cancel.load(Ordering::Relaxed) == 1;
        Box::pin(async {})
    }
}

impl EventHandler<HangingBreakByEntityEvent> for BreakRecorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut HangingBreakByEntityEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push(BreakSnapshot {
            by_entity: true,
            hanging_id: event.entity.get_entity().entity_id,
            remover_id: Some(event.remover.get_entity().entity_id),
            already_removed: event.entity.get_entity().is_removed(),
        });
        event.cancelled = self.cancel.load(Ordering::Relaxed) == 2;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_java_attack_hanging_events_keep_payload_order_and_cancellation() {
    for cancelled_stage in [1, 2] {
        let fixture = Fixture::new();
        let painting = painting(&fixture);
        let recorder = Arc::new(BreakRecorder {
            cancel: AtomicU8::new(cancelled_stage),
            events: Mutex::new(Vec::new()),
        });
        fixture
            .server
            .plugin_manager
            .register::<HangingBreakEvent, _>(recorder.clone(), EventPriority::Normal, true);
        fixture
            .server
            .plugin_manager
            .register::<HangingBreakByEntityEvent, _>(
                recorder.clone(),
                EventPriority::Normal,
                true,
            );
        attack(&fixture.player.player, &fixture.server, painting.as_ref());
        assert!(
            !painting.get_entity().is_removed(),
            "cancelled break removed painting"
        );
        assert_eq!(fixture.drops(&Item::PAINTING), 0);
        let expected = |by_entity| BreakSnapshot {
            by_entity,
            hanging_id: painting.get_entity().entity_id,
            remover_id: Some(fixture.player.player.entity_id()),
            already_removed: false,
        };
        let mut events = vec![expected(false)];
        if cancelled_stage == 2 {
            events.push(expected(true));
        }
        assert_eq!(*recorder.events.lock().unwrap(), events);

        recorder.events.lock().unwrap().clear();
        recorder.cancel.store(0, Ordering::Relaxed);
        attack(&fixture.player.player, &fixture.server, painting.as_ref());
        assert!(
            painting.get_entity().is_removed(),
            "cancellation retained a stale break claim"
        );
        assert_eq!(fixture.drops(&Item::PAINTING), 1);
        assert_eq!(
            *recorder.events.lock().unwrap(),
            vec![expected(false), expected(true)]
        );
        fixture.finish().await;
    }
}

struct BreakGate {
    first: AtomicBool,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    cancel_first: bool,
    calls: Mutex<Vec<Option<i32>>>,
    by_entity_calls: Mutex<Vec<bool>>,
}

struct FixFrameOnBreak(AtomicBool);

impl EventHandler<HangingBreakEvent> for FixFrameOnBreak {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut HangingBreakEvent,
    ) -> BoxFuture<'a, ()> {
        if self.0.load(Ordering::Relaxed) {
            event
                .entity
                .cast_any()
                .downcast_ref::<ItemFrameEntity>()
                .unwrap()
                .set_fixed(true);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_java_attack_rechecks_fixed_changed_by_break_callback() {
    let fixture = Fixture::new();
    fixture.set_block(SUPPORT, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
    let handler = Arc::new(FixFrameOnBreak(AtomicBool::new(true)));
    fixture
        .server
        .plugin_manager
        .register::<HangingBreakEvent, _>(handler.clone(), EventPriority::Normal, true);
    attack(&fixture.player.player, &fixture.server, frame.as_ref());
    let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    assert!(
        concrete.is_fixed(),
        "the real attack did not reach the callback"
    );
    assert!(
        !frame.get_entity().is_removed(),
        "callback made frame Fixed before removal"
    );
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    fixture.set_block(SUPPORT, &Block::AIR);
    fixture.tick(102);
    assert!(!frame.get_entity().is_removed());

    handler.0.store(false, Ordering::Relaxed);
    concrete.set_fixed(false);
    attack(&fixture.player.player, &fixture.server, frame.as_ref());
    assert!(frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    fixture.finish().await;
}

impl EventHandler<HangingBreakByEntityEvent> for BreakGate {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut HangingBreakByEntityEvent,
    ) -> BoxFuture<'a, ()> {
        self.by_entity_calls
            .lock()
            .unwrap()
            .push(event.entity.get_entity().is_removed());
        Box::pin(async {})
    }
}

impl EventHandler<HangingBreakEvent> for BreakGate {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut HangingBreakEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.lock().unwrap().push(
            event
                .remover
                .as_ref()
                .map(|entity| entity.get_entity().entity_id),
        );
        if self.first.swap(false, Ordering::SeqCst) {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv_timeout(DEADLINE).unwrap();
            event.cancelled = self.cancel_first;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_simultaneous_java_hit_and_support_retirement_drop_once() {
    for cancel_first in [false, true] {
        let fixture = Fixture::new();
        let painting = painting(&fixture);
        let (entered_send, entered) = mpsc::channel();
        let (release, release_receive) = mpsc::channel();
        let gate = Arc::new(BreakGate {
            first: AtomicBool::new(true),
            entered: entered_send,
            release: Mutex::new(release_receive),
            cancel_first,
            calls: Mutex::new(Vec::new()),
            by_entity_calls: Mutex::new(Vec::new()),
        });
        fixture
            .server
            .plugin_manager
            .register::<HangingBreakEvent, _>(gate.clone(), EventPriority::Normal, true);
        fixture
            .server
            .plugin_manager
            .register::<HangingBreakByEntityEvent, _>(gate.clone(), EventPriority::Normal, true);
        let player = fixture.player.player.clone();
        let server = fixture.server.clone();
        let target = painting.clone();
        let runtime = tokio::runtime::Handle::current();
        let (done_send, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _runtime = runtime.enter();
            attack(&player, &server, target.as_ref());
            done_send.send(()).unwrap();
        });
        let reached_gate = entered.recv_timeout(DEADLINE);
        if reached_gate.is_err() {
            let _ = release.send(());
        }
        assert!(
            reached_gate.is_ok(),
            "real Java attack never reached hanging break event"
        );
        fixture.set_block(SUPPORT, &Block::AIR);
        fixture.tick(102);
        assert!(
            painting.get_entity().is_removed(),
            "support retirement was blocked by a callback"
        );
        assert_eq!(fixture.drops(&Item::PAINTING), 1);
        release.send(()).unwrap();
        done.recv_timeout(DEADLINE).unwrap();
        worker.join().unwrap();
        fixture.tick(102);
        assert!(painting.get_entity().is_removed());
        assert_eq!(
            fixture.drops(&Item::PAINTING),
            1,
            "concurrent retirement duplicated painting"
        );
        assert_eq!(
            *gate.calls.lock().unwrap(),
            vec![Some(fixture.player.player.entity_id()), None]
        );
        assert!(
            gate.by_entity_calls.lock().unwrap().is_empty(),
            "resumed Java attack emitted a stale by-entity event after support removal"
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_java_frame_interaction_and_attacks_drop_contents_once() {
    let fixture = Fixture::new();
    fixture.set_block(SUPPORT, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::DIAMOND));
    dispatch(
        &fixture.player.player,
        &fixture.server,
        &SInteract {
            entity_id: frame.get_entity().entity_id.into(),
            r#type: 2.into(),
            target_position: None,
            hand: Some(0.into()),
            sneaking: false,
        },
        SInteract::to_id(CURRENT_MC_VERSION),
    );
    let hanging = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    assert_eq!(hanging.get_item().item, &Item::DIAMOND);
    assert!(fixture.player.player.inventory().held_item().is_empty());
    attack(&fixture.player.player, &fixture.server, frame.as_ref());
    assert!(!frame.get_entity().is_removed());
    assert!(hanging.get_item().is_empty());
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    attack(&fixture.player.player, &fixture.server, frame.as_ref());
    assert!(frame.get_entity().is_removed());
    attack(&fixture.player.player, &fixture.server, frame.as_ref());
    fixture.set_block(SUPPORT, &Block::AIR);
    fixture.tick(102);
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    fixture.finish().await;
}

struct FrameCloneReentry {
    component: CustomDataImpl,
    frame: Weak<dyn EntityBase>,
    armed: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    replacement: Option<&'static Item>,
}

impl DataComponentImpl for FrameCloneReentry {
    fn write_data(&self) -> NbtTag {
        self.component.write_data()
    }

    fn clone_dyn(&self) -> Box<dyn DataComponentImpl> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let frame = self.frame.upgrade().unwrap();
            let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
            if let Some(item) = self.replacement {
                concrete.set_item(ItemStack::new(1, item), false);
            } else {
                concrete.set_fixed(true);
            }
        }
        Box::new(Self {
            component: self.component.clone(),
            frame: self.frame.clone(),
            armed: self.armed.clone(),
            calls: self.calls.clone(),
            replacement: self.replacement,
        })
    }

    fn equal(&self, other: &dyn DataComponentImpl) -> bool {
        self.component.equal(other)
    }

    fn get_enum() -> DataComponent {
        DataComponent::CustomData
    }

    fn get_self_enum(&self) -> DataComponent {
        Self::get_enum()
    }

    fn to_dyn(self) -> Box<dyn DataComponentImpl> {
        Box::new(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        &self.component
    }

    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        &mut self.component
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_java_attack_component_clone_reenters_frame_without_guard() {
    for replacement in [None, Some(&Item::EMERALD)] {
        let fixture = Fixture::new();
        fixture.set_block(SUPPORT, &Block::STONE);
        let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
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
            replacement,
        });
        let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
        concrete.set_item(stack, false);
        armed.store(true, Ordering::SeqCst);

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
        assert!(
            done.recv_timeout(DEADLINE).is_ok(),
            "real Java attack deadlocked when component clone reentered the frame setter"
        );
        worker.join().unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(concrete.is_fixed(), replacement.is_none());
        assert!(!frame.get_entity().is_removed());
        assert_eq!(
            concrete.get_item().item,
            replacement.unwrap_or(&Item::DIAMOND)
        );
        assert_eq!(fixture.drops(&Item::DIAMOND), 0);
        assert_eq!(fixture.drops(&Item::EMERALD), 0);
        assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
        fixture.finish().await;
    }
}
