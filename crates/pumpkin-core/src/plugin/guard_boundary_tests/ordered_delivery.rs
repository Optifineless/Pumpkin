use super::*;
use pumpkin_protocol::ser::NetworkReadExt;
use std::sync::Barrier;

#[derive(Clone, Copy)]
enum Change {
    Border,
    Score,
    Advancement,
}

impl Change {
    fn prepare(self, fixture: &mut Fixture) {
        match self {
            Self::Border => (),
            Self::Score => fixture.run("scoreboard objectives add ordered dummy"),
            Self::Advancement => {
                fixture.run("advancement grant @s only minecraft:story/root");
                fixture.player.player.flush_advancements(false);
            }
        }
        fixture.player.take_packets();
    }
    fn mutate(
        self,
        server: &Arc<Server>,
        player: &Arc<crate::entity::player::Player>,
        first: bool,
    ) {
        let command = match (self, first) {
            (Self::Border, true) => "worldborder set 64",
            (Self::Border, false) => "worldborder set 128",
            (Self::Score, true) => "scoreboard players set value ordered 64",
            (Self::Score, false) => "scoreboard players set value ordered 128",
            (Self::Advancement, true) => "advancement revoke @s only minecraft:story/root",
            (Self::Advancement, false) => "advancement grant @s only minecraft:story/root",
        };
        server
            .command_dispatcher
            .load()
            .execute_input(command, &player.get_command_source(server))
            .unwrap();
        if matches!(self, Self::Advancement) {
            player.flush_advancements(false);
        }
    }
    fn id(self) -> i32 {
        match self {
            Self::Border => pumpkin_data::packet::clientbound::play::SET_BORDER_SIZE.0,
            Self::Score => pumpkin_data::packet::clientbound::play::SET_SCORE.0,
            Self::Advancement => pumpkin_data::packet::clientbound::play::UPDATE_ADVANCEMENTS.0,
        }
    }
    fn values(self, packets: Vec<bytes::Bytes>) -> Vec<i32> {
        packets
            .into_iter()
            .filter_map(|packet| {
                let mut reader = packet.as_ref();
                if reader.get_var_int().unwrap().0 != self.id() {
                    return None;
                }
                Some(match self {
                    Self::Border => reader.get_f64_be().unwrap() as i32,
                    Self::Score => {
                        assert_eq!(reader.get_str().unwrap().as_ref(), "value");
                        assert_eq!(reader.get_str().unwrap().as_ref(), "ordered");
                        reader.get_var_int().unwrap().0
                    }
                    Self::Advancement => {
                        assert!(!reader.get_bool().unwrap());
                        let added = reader.get_var_int().unwrap().0;
                        if added == 0 {
                            assert!(reader.get_var_int().unwrap().0 > 0);
                        }
                        i32::from(added > 0)
                    }
                })
            })
            .collect()
    }
    fn expected(self) -> Vec<i32> {
        if matches!(self, Self::Advancement) {
            vec![0, 1]
        } else {
            vec![64, 128]
        }
    }
}

async fn overtaking(change: Change) {
    let mut fixture = Fixture::new();
    change.prepare(&mut fixture);
    let committed = Arc::new(Barrier::new(2));
    let delivered = Arc::new(Barrier::new(2));
    let server = fixture.server.clone();
    let player = fixture.player.player.clone();
    let first_committed = committed.clone();
    let second_delivered = delivered.clone();
    let worker = std::thread::spawn(move || {
        // Deliberately delay the first operation's production effect batch after mutation.
        let effects = crate::world::guard_effects::GuardEffects::new();
        change.mutate(&server, &player, true);
        first_committed.wait();
        second_delivered.wait();
        drop(effects);
    });
    committed.wait();
    change.mutate(&fixture.server, &fixture.player.player, false);
    delivered.wait();
    worker.join().unwrap();
    assert_eq!(
        change.values(fixture.player.take_packets()),
        change.expected()
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_border_delivery_cannot_overtake_mutation_order() {
    overtaking(Change::Border).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_score_delivery_cannot_overtake_mutation_order() {
    overtaking(Change::Score).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_advancement_delivery_cannot_overtake_mutation_order() {
    overtaking(Change::Advancement).await;
}

struct MutateDuringPacket {
    change: Change,
    calls: AtomicUsize,
}
impl EventHandler<PacketSentEvent> for MutateDuringPacket {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == self.change.id() && self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            // A callback commits and flushes again before its triggering packet is enqueued.
            self.change.mutate(server, &event.player, false);
        }
        Box::pin(async {})
    }
}

async fn reentrant(change: Change) {
    let mut fixture = Fixture::new();
    change.prepare(&mut fixture);
    fixture.translating();
    let hook = Arc::new(MutateDuringPacket {
        change,
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    change.mutate(&fixture.server, &fixture.player.player, true);
    assert_eq!(hook.calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        change.values(fixture.player.take_packets()),
        change.expected()
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_border_callback_reentry_preserves_packet_order() {
    reentrant(Change::Border).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_score_callback_reentry_preserves_packet_order() {
    reentrant(Change::Score).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_advancement_callback_reentry_preserves_packet_order() {
    reentrant(Change::Advancement).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_abandoned_batch_does_not_block_later_border_update() {
    let mut fixture = Fixture::new();
    Change::Border.prepare(&mut fixture);
    let aborted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _effects = crate::world::guard_effects::GuardEffects::new();
        fixture.run("worldborder set 64");
        panic!("abort operation before its effect batch dispatches");
    }));
    assert!(aborted.is_err());
    fixture.run("worldborder set 128");
    assert_eq!(
        Change::Border.values(fixture.player.take_packets()),
        vec![128]
    );
    fixture.close().await;
}

struct CancellationDelivery {
    change: Change,
    delivered: std::sync::mpsc::Sender<()>,
}

impl EventHandler<PacketSentEvent> for CancellationDelivery {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == self.change.id() {
            assert!(!std::thread::panicking());
            match self.change {
                Change::Border => drop(event.player.world().worldborder.try_lock().unwrap()),
                Change::Score => drop(event.player.world().scoreboard.try_lock().unwrap()),
                Change::Advancement => drop(event.player.advancements.try_lock().unwrap()),
            }
            self.delivered.send(()).unwrap();
        }
        Box::pin(async {})
    }
}

async fn cancel_with_ready_successor(change: Change) {
    let mut fixture = Fixture::new();
    change.prepare(&mut fixture);
    fixture.translating();
    let (send, receive) = std::sync::mpsc::channel();
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(CancellationDelivery {
                change,
                delivered: send,
            }),
            EventPriority::Normal,
            true,
        );
    let committed = Arc::new(Barrier::new(2));
    let successor_ready = Arc::new(Barrier::new(2));
    let server = fixture.server.clone();
    let player = fixture.player.player.clone();
    let first_committed = committed.clone();
    let second_ready = successor_ready.clone();
    let worker = std::thread::spawn(move || {
        let aborted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _effects = crate::world::guard_effects::GuardEffects::new();
            change.mutate(&server, &player, true);
            first_committed.wait();
            second_ready.wait();
            panic!("abort the unready queue head after its successor becomes ready");
        }));
        assert!(aborted.is_err());
    });
    committed.wait();
    change.mutate(&fixture.server, &fixture.player.player, false);
    assert!(
        fixture
            .player
            .take_packets()
            .into_iter()
            .all(|packet| { packet.as_ref().get_var_int().unwrap().0 != change.id() })
    );
    successor_ready.wait();
    worker.join().unwrap();
    // Timeout only bounds a broken implementation; barriers establish the interleaving.
    receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let expected = *change.expected().last().unwrap();
    let packet = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.player.receive_packet(),
    )
    .await
    .unwrap()
    .unwrap();
    let packets = change.values(vec![packet]);
    assert_eq!(packets, vec![expected]);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_border_cancellation_delivers_already_ready_successor() {
    cancel_with_ready_successor(Change::Border).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_score_cancellation_delivers_already_ready_successor() {
    cancel_with_ready_successor(Change::Score).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_advancement_cancellation_delivers_already_ready_successor() {
    cancel_with_ready_successor(Change::Advancement).await;
}

struct CommitThenPanic {
    change: Change,
    calls: AtomicUsize,
    committed: AtomicUsize,
}

impl EventHandler<PacketSentEvent> for CommitThenPanic {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == self.change.id() && self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            self.change.mutate(server, &event.player, false);
            self.committed.store(1, Ordering::Release);
            panic!("one-shot native packet callback after successor commit");
        }
        Box::pin(async {})
    }
}

async fn dispatch_unwind_preserves_ready_successor(change: Change) {
    let mut fixture = Fixture::new();
    change.prepare(&mut fixture);
    fixture.translating();
    let hook = Arc::new(CommitThenPanic {
        change,
        calls: AtomicUsize::new(0),
        committed: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        change.mutate(&fixture.server, &fixture.player.player, true);
    }));
    assert!(
        failed.is_err(),
        "The actual native pairing callback did not unwind"
    );
    assert_eq!(
        hook.committed.load(Ordering::Acquire),
        1,
        "The successor mutation did not complete before the callback unwound"
    );
    // No third mutation may be needed to deliver the already committed successor.
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(packet) = fixture.player.receive_packet().await {
            let values = change.values(vec![packet]);
            if !values.is_empty() {
                return values;
            }
        }
        Vec::new()
    })
    .await;
    let values = received.unwrap_or_default();
    let calls = hook.calls.load(Ordering::Relaxed);
    fixture.close().await;
    assert_eq!(
        values,
        vec![*change.expected().last().unwrap()],
        "Dispatch unwind stranded an already ready committed successor"
    );
    assert_eq!(
        calls, 2,
        "The failed first effect was replayed or its successor was omitted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_border_dispatch_unwind_delivers_ready_successor() {
    dispatch_unwind_preserves_ready_successor(Change::Border).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_score_dispatch_unwind_delivers_ready_successor() {
    dispatch_unwind_preserves_ready_successor(Change::Score).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_advancement_dispatch_unwind_delivers_ready_successor() {
    dispatch_unwind_preserves_ready_successor(Change::Advancement).await;
}

struct PanicOncePerBroadcast {
    calls: AtomicUsize,
}

impl EventHandler<PacketSentEvent> for PanicOncePerBroadcast {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == Change::Border.id() && self.calls.fetch_add(1, Ordering::AcqRel) == 0
        {
            assert!(!std::thread::panicking());
            drop(event.player.world().worldborder.try_lock().unwrap());
            panic!("one-shot actual border packet callback");
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_outer_batch_unwind_keeps_committed_sibling() {
    let mut fixture = Fixture::new();
    Change::Border.prepare(&mut fixture);
    fixture.translating();
    let hook = Arc::new(PanicOncePerBroadcast {
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let effects = crate::world::guard_effects::GuardEffects::new();
        Change::Border.mutate(&fixture.server, &fixture.player.player, true);
        Change::Border.mutate(&fixture.server, &fixture.player.player, false);
        assert_eq!(
            fixture.world.worldborder.lock().unwrap().new_diameter,
            128.0
        );
        assert_eq!(hook.calls.load(Ordering::Acquire), 0);
        drop(effects);
    }));
    assert!(
        failed.is_err(),
        "The first actual border callback did not unwind"
    );
    let values = Change::Border.values(fixture.player.take_packets());
    let calls = hook.calls.load(Ordering::Acquire);
    fixture.close().await;
    assert_eq!(
        values,
        vec![128],
        "Dispatch unwind discarded a later committed sibling in the same outer batch"
    );
    assert_eq!(
        calls, 2,
        "The failed packet was replayed or its sibling omitted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_broadcast_unwind_keeps_remaining_recipient() {
    let mut fixture = Fixture::new();
    Change::Border.prepare(&mut fixture);
    fixture.translating();
    let mut observer = TestPlayer::new(&fixture.world);
    fixture.world.players.store(Arc::new(vec![
        fixture.player.player.clone(),
        observer.player.clone(),
    ]));
    observer.client().set_player(observer.player.clone());
    observer
        .client()
        .version
        .store(JavaMinecraftVersion::V_1_21_2);
    fixture.player.take_packets();
    observer.take_packets();
    let recipients: Vec<_> = fixture
        .world
        .players
        .load()
        .iter()
        .map(|p| p.gameprofile.id)
        .collect();
    assert_eq!(
        recipients.len(),
        2,
        "The actual broadcast must target two clients"
    );
    let hook = Arc::new(PanicOncePerBroadcast {
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Change::Border.mutate(&fixture.server, &fixture.player.player, false);
    }));
    assert!(
        failed.is_err(),
        "The first actual recipient callback did not unwind"
    );
    assert_eq!(
        fixture.world.worldborder.lock().unwrap().new_diameter,
        128.0
    );
    let owner_values = Change::Border.values(fixture.player.take_packets());
    let observer_values = Change::Border.values(observer.take_packets());
    let (first, second) = if recipients[0] == fixture.player.player.gameprofile.id {
        (owner_values, observer_values)
    } else {
        assert_eq!(recipients[0], observer.player.gameprofile.id);
        (observer_values, owner_values)
    };
    let calls = hook.calls.load(Ordering::Acquire);
    observer.client().close();
    fixture.close().await;
    assert!(first.is_empty(), "The failed first recipient was replayed");
    assert_eq!(
        second,
        vec![128],
        "One recipient's native callback panic discarded the other committed notification"
    );
    assert_eq!(
        calls, 2,
        "Both actual recipient sends must be attempted once"
    );
}
