use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::player::player_gamemode_change::PlayerGamemodeChangeEvent,
};
use pumpkin_data::packet::clientbound::play as outgoing;
use pumpkin_protocol::{VarInt, java::server::play::SClientCommand, ser::NetworkReadExt};
use pumpkin_util::GameMode;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

struct RemoveDuringSpectatorChange {
    player_id: i32,
    removed: AtomicBool,
    calls: AtomicUsize,
    resume: Mutex<Option<oneshot::Receiver<()>>>,
}

impl EventHandler<PlayerGamemodeChangeEvent> for RemoveDuringSpectatorChange {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerGamemodeChangeEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() != self.player_id {
            return Box::pin(async {});
        }
        assert_eq!(event.previous_gamemode, GameMode::Survival);
        assert_eq!(event.new_gamemode, GameMode::Spectator);
        assert!(event.player.living_entity.health.load() > 0.0);
        self.calls.fetch_add(1, Ordering::Relaxed);
        let resume = self.resume.lock().unwrap().take().unwrap();
        Box::pin(async move {
            event.player.remove().await;
            self.removed.store(true, Ordering::Release);
            tokio::time::timeout(Duration::from_secs(60), resume)
                .await
                .unwrap()
                .unwrap();
        })
    }
}

fn republishes(packet: &Bytes, player: &Player) -> bool {
    let mut bytes = packet.as_ref();
    match bytes.get_var_int().unwrap().0 {
        id if id == outgoing::ADD_ENTITY.to_id(CURRENT_MC_VERSION) => {
            bytes.get_var_int().unwrap().0 == player.entity_id()
        }
        id if id == outgoing::PLAYER_INFO_UPDATE.to_id(CURRENT_MC_VERSION) => {
            let flags = bytes[0];
            bytes = &bytes[1..];
            flags & 1 != 0
                && bytes.get_var_int().unwrap().0 == 1
                && bytes.get_uuid().unwrap() == player.gameprofile.id
        }
        _ => false,
    }
}

fn is_absent(scenario: &Scenario, player: &Arc<Player>) -> bool {
    scenario.server.worlds.load().iter().all(|world| {
        !world.contains_player_instance(player)
            && world
                .entity_tracker
                .get_tracked_entity(player.entity_id())
                .is_none()
    })
}

async fn removed_hardcore_respawn_cannot_publish(cross_world: bool) {
    let scenario = Scenario::new_with_hardcore(true).await;
    let session = scenario.join(Uuid::new_v4(), "HardcoreRespawn").await;
    let observer = scenario.join(Uuid::new_v4(), "HardcoreObserver").await;
    scenario.command("gamemode spectator HardcoreObserver");
    let dimension = if cross_world {
        super::transfer::transfer(&scenario, &observer, "the_nether").await;
        Dimension::THE_NETHER
    } else {
        Dimension::OVERWORLD
    };
    scenario.command("kill HardcoreRespawn");
    assert_eq!(session.player.living_entity.health.load(), 0.0);
    session
        .player
        .set_respawn_point(dimension.clone(), BlockPos::new(7, 64, 9), 90.0, 0.0, true);
    let (resume, receiver) = oneshot::channel();
    let hook = Arc::new(RemoveDuringSpectatorChange {
        player_id: session.player.entity_id(),
        removed: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        resume: Mutex::new(Some(receiver)),
    });
    scenario
        .server
        .plugin_manager
        .register(hook.clone(), EventPriority::Normal, true);

    // ServerGamePacketListenerImpl.handleClientCommand -> PlayerList.respawn -> setGameMode.
    session.send(
        &scenario.server,
        &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
    );
    scenario
        .until("hardcore mode callback completed player removal", || {
            hook.removed.load(Ordering::Acquire)
        })
        .await;
    assert_eq!(session.player.world().dimension, dimension);
    assert_eq!(hook.calls.load(Ordering::Relaxed), 1);
    assert!(
        !session.client().is_closed(),
        "public removal keeps the socket open"
    );
    assert!(is_absent(&scenario, &session.player));
    observer.packets();
    resume.send(()).unwrap();
    scenario
        .until("removed hardcore respawn completed", || {
            scenario.server.tasks.is_empty()
        })
        .await;
    let absent = is_absent(&scenario, &session.player);
    let observer_add = observer
        .packets()
        .iter()
        .any(|packet| republishes(packet, &session.player));
    scenario.stop().await;
    assert!(
        absent,
        "hardcore callback republished a removed player in the tracker (post-cleanup observer add={observer_add})"
    );
    assert!(
        !observer_add,
        "hardcore callback sent a post-cleanup observer add"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardcore_respawn_review_same_world_callback_removal_cannot_publish() {
    run(removed_hardcore_respawn_cannot_publish(false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardcore_respawn_review_cross_world_callback_removal_cannot_publish() {
    run(removed_hardcore_respawn_cannot_publish(true)).await;
}
