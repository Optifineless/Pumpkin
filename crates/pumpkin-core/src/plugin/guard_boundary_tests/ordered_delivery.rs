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
