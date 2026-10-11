use super::*;
use crate::world::scoreboard::ScoreboardObjective;
use pumpkin_protocol::java::client::play::RenderType;
use pumpkin_util::text::TextComponent;

struct ReenterOwner {
    border: bool,
    calls: AtomicUsize,
}

impl EventHandler<PacketSentEvent> for ReenterOwner {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        let id = if self.border {
            pumpkin_data::packet::clientbound::play::SET_BORDER_SIZE.0
        } else {
            pumpkin_data::packet::clientbound::play::SET_OBJECTIVE.0
        };
        if event.packet_id == id {
            let world = event.player.world();
            if self.border {
                assert_eq!(world.worldborder.try_lock().unwrap().new_diameter, 128.0);
            } else {
                assert!(
                    world
                        .scoreboard
                        .try_lock()
                        .unwrap()
                        .get_objective("direct")
                        .is_some()
                );
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

async fn direct_mutation(border: bool) {
    let fixture = Fixture::new();
    fixture.translating();
    let hook = Arc::new(ReenterOwner {
        border,
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
    // Public owner APIs, deliberately without a manually created GuardEffects batch.
    if border {
        let mut owner = fixture.world.worldborder.lock().unwrap();
        owner.set_diameter(&fixture.world, 64.0, None);
        owner.set_diameter(&fixture.world, 128.0, None);
        assert_eq!(hook.calls.load(Ordering::Relaxed), 0);
        drop(owner);
        assert_eq!(hook.calls.load(Ordering::Relaxed), 2);
    } else {
        let mut owner = fixture.world.scoreboard.lock().unwrap();
        owner.add_objective(
            &fixture.world,
            ScoreboardObjective::new(
                "direct",
                TextComponent::text("direct"),
                RenderType::Integer,
                None,
                "dummy",
            ),
        );
        assert_eq!(hook.calls.load(Ordering::Relaxed), 0);
        drop(owner);
        assert_eq!(hook.calls.load(Ordering::Relaxed), 1);
    }
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_direct_border_mutation_delivers_after_unlock() {
    direct_mutation(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_direct_scoreboard_mutation_delivers_after_unlock() {
    direct_mutation(false).await;
}
