use super::*;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_respawn::PlayerRespawnEvent,
    },
    world::respawn_test_support::{self as test_support, Phase, Step},
};
use pumpkin_data::packet::clientbound::play as outgoing;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{VarInt, java::server::play::SClientCommand, ser::NetworkReadExt};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{mpsc, oneshot};

const RESPAWN_POSITION: Vector3<f64> = Vector3::new(7.5, 64.1, 9.5);

async fn new_scenario() -> Scenario {
    tokio::time::timeout(Duration::from_secs(60), Scenario::new())
        .await
        .unwrap()
}

fn prepare_respawn(scenario: &Scenario, session: &Session) {
    session
        .player
        .get_entity()
        .set_pos(Vector3::new(11.5, 64.0, 11.5));
    scenario.command(&format!("kill {}", session.player.gameprofile.name));
    assert_eq!(session.player.living_entity.health.load(), 0.0);
    session.player.set_respawn_point(
        Dimension::THE_NETHER,
        BlockPos::new(7, 64, 9),
        90.0,
        0.0,
        true,
    );
}

fn request_respawn(scenario: &Scenario, session: &Session) {
    session.send(
        &scenario.server,
        &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
    );
}

async fn step(
    scenario: &Scenario,
    steps: &mut mpsc::UnboundedReceiver<Step>,
    phase: Phase,
) -> Step {
    let step = scenario
        .drive("respawn admission gate", steps.recv())
        .await
        .unwrap();
    assert_eq!(step.phase, phase);
    step
}

fn competing_session(world: &Arc<World>, old: &Session) -> Arc<Player> {
    // Install the competing session at the admission boundary, after its initial UUID scan.
    let profile = old.player.gameprofile.clone();
    let (client, _packets) = captured_client(profile.clone());
    let competitor = Arc::new(Player::new(
        Arc::new(ClientPlatform::Java(client)),
        profile,
        PlayerConfig::default(),
        world,
        pumpkin_util::GameMode::Survival,
    ));
    world.players.rcu(|players| {
        let mut players = (**players).clone();
        players.push(competitor.clone());
        players
    });
    competitor
}

fn assert_absent(scenario: &Scenario, session: &Session) {
    for world in scenario.server.worlds.load().iter() {
        assert!(!world.contains_player_instance(&session.player));
        assert!(
            world
                .entity_tracker
                .get_tracked_entity(session.player.entity_id())
                .is_none(),
            "disconnected respawn was republished in the entity tracker"
        );
    }
}

struct PauseRespawnEvent {
    player_id: i32,
    reached: AtomicBool,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

impl EventHandler<PlayerRespawnEvent> for PauseRespawnEvent {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() != self.player_id {
            return Box::pin(async {});
        }
        assert_eq!(event.respawned_world.dimension, Dimension::THE_NETHER);
        assert_eq!(event.position, RESPAWN_POSITION);
        let release = self.release.lock().unwrap().take();
        self.reached.store(true, Ordering::Release);
        Box::pin(async move {
            if let Some(release) = release {
                tokio::time::timeout(Duration::from_secs(60), release)
                    .await
                    .unwrap()
                    .unwrap();
            }
        })
    }
}

#[tokio::test]
async fn respawn_owner_review_disconnect_in_post_transfer_hook_saves_matching_dimension_and_pos() {
    run(async {
        let scenario = new_scenario().await;
        let session = scenario.join(Uuid::new_v4(), "SavedRespawn").await;
        prepare_respawn(&scenario, &session);
        let (release, receiver) = oneshot::channel();
        let hook = Arc::new(PauseRespawnEvent {
            player_id: session.player.entity_id(),
            reached: AtomicBool::new(false),
            release: Mutex::new(Some(receiver)),
        });
        scenario
            .server
            .plugin_manager
            .register(hook.clone(), EventPriority::Normal, true);
        request_respawn(&scenario, &session);
        scenario
            .until("post-transfer respawn event", || {
                hook.reached.load(Ordering::Acquire)
            })
            .await;
        scenario.leave(&session).await;
        let (saved_session, saved) = scenario
            .drive(
                "load disconnected respawn save",
                scenario
                    .server
                    .player_data_storage
                    .load_data(&session.player.gameprofile.id),
            )
            .await
            .unwrap();
        let saved = saved.unwrap();
        assert_eq!(
            saved.get_string("Dimension"),
            Some(Dimension::THE_NETHER.minecraft_name)
        );
        assert_eq!(
            saved.get_list("Pos").unwrap(),
            &[
                NbtTag::Double(RESPAWN_POSITION.x),
                NbtTag::Double(RESPAWN_POSITION.y),
                NbtTag::Double(RESPAWN_POSITION.z),
            ],
            "disconnect saved destination dimension with the source death coordinates"
        );
        drop(saved_session);
        let _ = release.send(());
        scenario
            .until("disconnected respawn task", || {
                scenario.server.tasks.is_empty()
            })
            .await;
        assert_absent(&scenario, &session);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn respawn_owner_review_rejected_admission_preserves_source_chunk_stream() {
    run(async {
        let scenario = new_scenario().await;
        let session = scenario.join(Uuid::new_v4(), "RejectedRespawn").await;
        prepare_respawn(&scenario, &session);
        let source = session.player.world();
        let destination = scenario.world(&Dimension::THE_NETHER);
        let watched = session.player.watched_section.load();
        let held_tickets = *session.player.held_chunk_tickets.lock().unwrap();
        let tickets = source.level.chunk_loading.lock().unwrap().ticket.clone();
        let epoch = session.player.chunk_send_epoch.load(Ordering::Relaxed);
        let (sent, pending) = {
            let sender = session.player.chunk_sender.lock().unwrap();
            (sender.sent_chunks_count(), sender.pending_chunks.clone())
        };
        assert!(held_tickets.is_some());
        assert!(sent > 0, "source chunk stream must exist before rejection");
        session.packets();
        let (_registration, mut steps) = test_support::capture(&session.player);
        request_respawn(&scenario, &session);
        let admission = step(&scenario, &mut steps, Phase::BeforeAdmission).await;
        let competitor = competing_session(&destination, &session);
        admission.release.send(()).unwrap();
        step(&scenario, &mut steps, Phase::AdmissionRejected)
            .await
            .release
            .send(())
            .unwrap();
        scenario
            .until("rejected respawn task", || scenario.server.tasks.is_empty())
            .await;
        assert!(Arc::ptr_eq(&session.player.world(), &source));
        assert!(source.contains_player_instance(&session.player));
        assert!(
            source
                .entity_tracker
                .get_tracked_entity(session.player.entity_id())
                .is_some()
        );
        assert!(
            session.player.watched_section.load() == watched,
            "rejection erased source watches"
        );
        assert!(
            watched
                .all_chunks_within()
                .all(|pos| source.level.is_chunk_watched(&pos))
        );
        assert_eq!(
            *session.player.held_chunk_tickets.lock().unwrap(),
            held_tickets
        );
        assert_eq!(source.level.chunk_loading.lock().unwrap().ticket, tickets);
        assert_eq!(
            session.player.chunk_send_epoch.load(Ordering::Relaxed),
            epoch
        );
        let stream_after = {
            let sender = session.player.chunk_sender.lock().unwrap();
            (sender.sent_chunks_count(), sender.pending_chunks.clone())
        };
        assert_eq!(stream_after, (sent, pending));
        assert!(
            !session.packets().iter().any(|packet| {
                packet.as_ref().get_var_int().unwrap().0
                    == outgoing::UNLOAD_CHUNK.to_id(CURRENT_MC_VERSION)
            }),
            "rejection unloaded chunks still needed in the source world"
        );
        scenario
            .drive(
                "remove competing admission",
                destination.remove_player(&competitor, false),
            )
            .await;
        scenario.stop().await;
    })
    .await;
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

#[tokio::test]
async fn respawn_owner_review_disconnect_after_rejected_admission_cannot_publish_ghost() {
    run(async {
        let scenario = new_scenario().await;
        let session = scenario.join(Uuid::new_v4(), "GhostRespawn").await;
        let observer = scenario.join(Uuid::new_v4(), "GhostObserver").await;
        prepare_respawn(&scenario, &session);
        let destination = scenario.world(&Dimension::THE_NETHER);
        let (_registration, mut steps) = test_support::capture(&session.player);
        request_respawn(&scenario, &session);
        let admission = step(&scenario, &mut steps, Phase::BeforeAdmission).await;
        let competitor = competing_session(&destination, &session);
        admission.release.send(()).unwrap();
        let rejected = step(&scenario, &mut steps, Phase::AdmissionRejected).await;
        scenario.leave(&session).await;
        assert_absent(&scenario, &session);
        observer.packets();
        rejected.release.send(()).unwrap();
        scenario
            .until("rejected disconnected respawn task", || {
                scenario.server.tasks.is_empty()
            })
            .await;
        assert_absent(&scenario, &session);
        assert!(
            !observer
                .packets()
                .iter()
                .any(|packet| republishes(packet, &session.player)),
            "rollback sent a departed player back to a connected observer"
        );
        scenario
            .drive(
                "remove competing admission",
                destination.remove_player(&competitor, false),
            )
            .await;
        scenario.stop().await;
    })
    .await;
}
