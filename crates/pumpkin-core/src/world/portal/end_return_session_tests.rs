#![expect(clippy::unwrap_used, reason = "Bounded real portal session regression")]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use futures::future::BoxFuture;
use pumpkin_data::{Block, block_properties::RespawnAnchorLikeProperties, dimension::Dimension};
use pumpkin_util::version::JavaMinecraftVersion;
use pumpkin_world::world::BlockFlags;
use tokio::sync::oneshot;

use super::end_return_test_support::{DEADLINE, Fixture, PERSONAL, contact, load_personal};
use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{EventHandler, EventPriority, api::events::server::packet::PacketSentEvent},
    server::Server,
};

struct WinGameGate {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    resume: Mutex<Option<oneshot::Receiver<()>>>,
    calls: AtomicUsize,
    expired: AtomicBool,
}

impl EventHandler<PacketSentEvent> for WinGameGate {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id != pumpkin_data::packet::clientbound::play::GAME_EVENT.0
            || event.payload.first() != Some(&4)
        {
            return Box::pin(async {});
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        let entered = self.entered.lock().unwrap().take();
        let resume = self.resume.lock().unwrap().take();
        Box::pin(async move {
            if let Some(entered) = entered {
                let _ = entered.send(());
            }
            if let Some(resume) = resume {
                let completed = tokio::time::timeout(DEADLINE, resume).await;
                self.expired
                    .store(!matches!(completed, Ok(Ok(()))), Ordering::Relaxed);
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_session_replaced_during_end_destination() {
    let fixture = Fixture::new().await;
    let mut props = RespawnAnchorLikeProperties::default(&Block::RESPAWN_ANCHOR);
    props.charges = 3;
    fixture.nether.set_block_state(
        &PERSONAL,
        props.to_state_id(&Block::RESPAWN_ANCHOR),
        BlockFlags::FORCE_STATE,
    );
    let mut client = fixture.returning_player();
    let old = client.player.clone();
    load_personal(&old, &Dimension::THE_NETHER, false);
    // The established multiversion packet hook receives raw 26.3 packets before translation.
    client.client().set_player(old.clone());
    client
        .client()
        .version
        .store(JavaMinecraftVersion::V_1_21_2);
    let initial = old.position();
    let (entered, suspended) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    let gate = Arc::new(WinGameGate {
        entered: Mutex::new(Some(entered)),
        resume: Mutex::new(Some(resumed)),
        calls: AtomicUsize::new(0),
        expired: AtomicBool::new(false),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(gate.clone(), EventPriority::Normal, true);
    contact(&fixture.server, old.as_ref());
    tokio::time::timeout(DEADLINE, suspended)
        .await
        .unwrap()
        .unwrap();
    // WinGame runs inside the actual async selector, before its saved-spawn lookup.
    // Replace the source session while that selector is suspended at the real packet callback.
    let removed = tokio::time::timeout(DEADLINE, fixture.end.remove_player(&old, false))
        .await
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&removed, &old));
    let mut replacement_client = TestPlayer::with_profile(&fixture.end, old.gameprofile.clone());
    let replacement = replacement_client.player.clone();
    replacement.get_entity().set_pos(initial);
    resume.send(()).unwrap();
    fixture.server.tasks.close();
    tokio::time::timeout(DEADLINE, fixture.server.tasks.wait())
        .await
        .unwrap();
    // Drain every queued operation before assertions, including a stale transfer on an unfixed tree.
    client.client().close();
    let old_java = old.client.java().unwrap();
    let replacement_java = replacement.client.java().unwrap();
    let drained = async {
        old_java.await_tasks().await;
        replacement_java.await_tasks().await;
    };
    tokio::time::timeout(
        DEADLINE,
        replacement_client.with_outgoing_writer(client.with_outgoing_writer(drained)),
    )
    .await
    .unwrap();
    assert_eq!(gate.calls.load(Ordering::Relaxed), 1);
    assert!(
        !gate.expired.load(Ordering::Relaxed),
        "packet callback must resume through its explicit gate"
    );
    assert!(Arc::ptr_eq(&old.world(), &fixture.end));
    assert_eq!(old.position(), initial);
    assert!(
        Arc::ptr_eq(&replacement.world(), &fixture.end),
        "old portal destination must not transfer the replacement session"
    );
    assert_eq!(replacement.position(), initial);
    assert!(
        fixture
            .end
            .get_player_by_uuid(old.gameprofile.id)
            .is_some_and(|current| Arc::ptr_eq(&current, &replacement))
    );
    assert!(
        fixture
            .overworld
            .get_player_by_uuid(old.gameprofile.id)
            .is_none()
    );
    assert!(
        fixture
            .nether
            .get_player_by_uuid(old.gameprofile.id)
            .is_none()
    );
    assert_eq!(
        RespawnAnchorLikeProperties::from_state_id(fixture.nether.get_block_state_id(&PERSONAL))
            .charges,
        3
    );
    fixture.finish().await;
}
