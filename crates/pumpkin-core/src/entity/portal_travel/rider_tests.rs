use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_protocol::{MultiVersionJavaPacket, ser::NetworkReadExt};
use pumpkin_util::math::vector3::Vector3;

use super::{
    EntityBase,
    review_tests::{finish_trip, tick_trip},
};
use crate::{
    entity::{death_test_world::DeathTestWorld, player::Player},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_change_world::PlayerChangeWorldEvent,
    },
    server::Server,
    world::World,
};

struct RiderGate {
    ready: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    resume: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl EventHandler<PlayerChangeWorldEvent> for RiderGate {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let ready = self.ready.lock().unwrap().take();
            if let Some(ready) = ready {
                ready.send(()).unwrap();
                let resume = self.resume.lock().await.take().unwrap();
                let _ = resume.await;
            }
        })
    }
}

type RiderFixture = (
    DeathTestWorld,
    Arc<dyn EntityBase>,
    Arc<Player>,
    Arc<World>,
    tokio::sync::oneshot::Sender<()>,
);

pub(super) struct PassengerUpdates {
    pub(super) observer: Arc<Player>,
    pub(super) vehicle_id: i32,
    pub(super) lists: Arc<Mutex<Vec<Vec<i32>>>>,
}

impl EventHandler<crate::plugin::server::packet::PacketSentEvent> for PassengerUpdates {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::server::packet::PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if Arc::ptr_eq(&event.player, &self.observer)
            && event.packet_id
                == pumpkin_protocol::java::client::play::CSetPassengers::to_id(
                    pumpkin_data::packet::CURRENT_MC_VERSION,
                )
        {
            let mut payload = &event.payload[..];
            if payload.get_var_int().unwrap().0 == self.vehicle_id {
                let count = payload.get_var_int().unwrap().0;
                let ids = (0..count)
                    .map(|_| payload.get_var_int().unwrap().0)
                    .collect();
                self.lists.lock().unwrap().push(ids);
            }
        }
        Box::pin(async {})
    }
}

async fn gated_rider() -> RiderFixture {
    observed_gated_rider(None).await
}

pub(super) async fn destination_observer(
    fixture: &DeathTestWorld,
    destination: &Arc<World>,
) -> Arc<Player> {
    let observer = fixture.player_with_packet_sink("rider-destination-observer");
    let java = observer.client.java().unwrap();
    java.player.store(Arc::new(Some(observer.clone())));
    java.version
        .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21);
    observer
        .teleport_world(
            destination.clone(),
            Vector3::new(1.0, 80.0, 1.0),
            None,
            None,
        )
        .await;
    observer
}

async fn observed_gated_rider(lists: Option<Arc<Mutex<Vec<Vec<i32>>>>>) -> RiderFixture {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .create_world("portal-rider".to_owned(), Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    let observer = if lists.is_some() {
        Some(destination_observer(&fixture, &destination).await)
    } else {
        None
    };
    let root = fixture.mob(&EntityType::COW);
    if let Some(observer) = &observer {
        fixture
            .server
            .plugin_manager
            .register::<crate::plugin::server::packet::PacketSentEvent, _>(
                Arc::new(PassengerUpdates {
                    observer: observer.clone(),
                    vehicle_id: root.get_entity().entity_id,
                    lists: lists.unwrap(),
                }),
                EventPriority::Normal,
                true,
            );
    }
    let player = fixture.player_with_packet_sink("portal-rider");
    player
        .client
        .java()
        .unwrap()
        .player
        .store(Arc::new(Some(player.clone())));
    let (session, _) = fixture
        .server
        .player_data_storage
        .load_data(&player.gameprofile.id)
        .await
        .unwrap();
    *player.storage_session.lock().unwrap() = Some(session);
    root.get_entity()
        .add_passenger(root.clone(), player.clone());
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(
            Arc::new(RiderGate {
                ready: Mutex::new(Some(ready)),
                resume: tokio::sync::Mutex::new(Some(gate)),
            }),
            EventPriority::Normal,
            true,
        );
    tick_trip(&fixture, root.as_ref(), destination.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    finish_trip(root.get_entity()).await;
    assert!(Arc::ptr_eq(
        &root.get_entity().world.load_full(),
        &destination
    ));
    assert!(Arc::ptr_eq(&player.world(), &fixture.world()));
    assert_eq!(
        destination.players.load().len(),
        usize::from(observer.is_some())
    );
    if let Some(observer) = observer {
        let tracked = destination
            .entity_tracker
            .get_tracked_entity(root.get_entity().entity_id)
            .unwrap();
        assert!(Arc::ptr_eq(&tracked.entity, &root));
        assert!(tracked.seen_by.contains(&observer.gameprofile.id));
    }
    (fixture, root, player, destination, resume)
}

async fn drain_rider(player: &Player) {
    tokio::time::timeout(
        Duration::from_secs(60),
        player.client.java().unwrap().await_tasks(),
    )
    .await
    .unwrap();
}

async fn assert_gate_cancelled(resume: &tokio::sync::oneshot::Sender<()>) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !resume.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_player_rider_holds_destination_admission_and_restores_mount_after_arrival() {
    let (fixture, root, player, destination, resume) = gated_rider().await;
    assert!(
        fixture.server.unload_world("portal-rider").await.is_err(),
        "queued rider lost its destination admission"
    );
    assert!(!destination.level.cancel_token.is_cancelled());
    assert!(!player.get_entity().has_vehicle());
    assert!(!root.get_entity().has_passenger(player.entity_id()));
    resume.send(()).unwrap();
    drain_rider(&player).await;
    assert!(Arc::ptr_eq(&player.world(), &destination));
    assert!(destination.get_player_by_id(player.entity_id()).is_some());
    assert!(
        fixture
            .world()
            .get_player_by_id(player.entity_id())
            .is_none()
    );
    assert!(Arc::ptr_eq(
        &player.get_entity().get_vehicle().unwrap(),
        &root
    ));
    assert!(root.get_entity().has_passenger(player.entity_id()));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_player_rider_logout_cancels_callback_without_saving_foreign_root_vehicle() {
    let (fixture, root, player, destination, resume) = gated_rider().await;
    tokio::time::timeout(
        Duration::from_secs(60),
        player.finish_java_connection(&fixture.server),
    )
    .await
    .unwrap();
    assert_gate_cancelled(&resume).await;
    assert!(
        fixture
            .world()
            .get_player_by_id(player.entity_id())
            .is_none()
    );
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    assert!(!root.get_entity().has_passenger(player.entity_id()));
    let (_session, saved) = fixture
        .server
        .player_data_storage
        .load_data(&player.gameprofile.id)
        .await
        .unwrap();
    let saved = saved.unwrap();
    assert_eq!(
        saved.get_string("Dimension"),
        Some(Dimension::OVERWORLD.minecraft_name)
    );
    assert!(
        saved.get_compound("RootVehicle").is_none(),
        "logout saved a vehicle from a different dimension"
    );
    fixture.server.unload_world("portal-rider").await.unwrap();
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_player_rider_newer_teleport_cancels_callback_and_releases_admission() {
    let (fixture, root, player, destination, resume) = gated_rider().await;
    let newer = Vector3::new(10.0, 82.0, 11.0);
    player.request_teleport(newer, 42.0, 15.0);
    drain_rider(&player).await;
    assert_gate_cancelled(&resume).await;
    assert!(Arc::ptr_eq(&player.world(), &fixture.world()));
    assert_eq!(player.position(), newer);
    assert!(!player.get_entity().has_vehicle());
    assert!(!root.get_entity().has_passenger(player.entity_id()));
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    fixture.server.unload_world("portal-rider").await.unwrap();
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_player_rider_new_mount_cancels_callback_and_keeps_new_vehicle() {
    let (fixture, root, player, destination, resume) = gated_rider().await;
    let replacement = fixture.mob(&EntityType::COW);
    replacement
        .get_entity()
        .add_passenger(replacement.clone(), player.clone());
    drain_rider(&player).await;
    assert_gate_cancelled(&resume).await;
    assert!(Arc::ptr_eq(&player.world(), &fixture.world()));
    assert!(Arc::ptr_eq(
        &player.get_entity().get_vehicle().unwrap(),
        &replacement
    ));
    assert!(!root.get_entity().has_passenger(player.entity_id()));
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    fixture.server.unload_world("portal-rider").await.unwrap();
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_player_rider_logout_sends_empty_passengers_to_existing_destination_observer() {
    let lists = Arc::new(Mutex::new(Vec::new()));
    let (fixture, root, player, destination, resume) =
        observed_gated_rider(Some(lists.clone())).await;
    assert_eq!(
        *lists.lock().unwrap(),
        vec![Vec::<i32>::new()],
        "destination pairing advertised a pending source rider",
    );
    tokio::time::timeout(
        Duration::from_secs(60),
        player.finish_java_connection(&fixture.server),
    )
    .await
    .unwrap();
    assert_gate_cancelled(&resume).await;
    assert!(!root.get_entity().has_passenger(player.entity_id()));
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    assert!(lists.lock().unwrap().last().unwrap().is_empty());
    for observer in destination.players.load().iter() {
        observer.client.java().unwrap().close();
    }
    fixture.server.shutdown().await;
}
