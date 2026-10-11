use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_protocol::MultiVersionJavaPacket;
use pumpkin_util::math::vector3::Vector3;

use super::{
    EntityBase,
    review_tests::{finish_trip, tick_trip},
};
use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{BoxFuture, EventHandler, EventPriority, server::packet::PacketSentEvent},
    server::Server,
};

struct PairingInterrupt {
    traveler: Arc<dyn EntityBase>,
    applied: Arc<AtomicBool>,
}

impl EventHandler<PacketSentEvent> for PairingInterrupt {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id
            == pumpkin_protocol::java::client::play::CHeadRot::to_id(
                pumpkin_data::packet::CURRENT_MC_VERSION,
            )
            && !self.applied.swap(true, Ordering::SeqCst)
        {
            let entity = self.traveler.get_entity();
            entity.teleport(
                Vector3::new(10.0, 82.0, 11.0),
                Some(42.0),
                Some(15.0),
                &entity.world.load_full(),
            );
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mob_pairing_callback_cannot_overwrite_newer_teleport_after_membership_commit() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    let observer = fixture.player_with_packet_sink("portal-observer");
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
    let traveler = fixture.mob(&EntityType::COW);
    let applied = Arc::new(AtomicBool::new(false));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(PairingInterrupt {
                traveler: traveler.clone(),
                applied: applied.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    tick_trip(&fixture, traveler.as_ref(), destination.clone());
    finish_trip(traveler.get_entity()).await;
    // Invalidation frees admission immediately; drain the real task before observing its end.
    fixture.server.shutdown().await;
    assert!(
        applied.load(Ordering::SeqCst),
        "actual tracker pairing callback did not run"
    );
    assert!(Arc::ptr_eq(
        &traveler.get_entity().world.load_full(),
        &destination
    ));
    assert_eq!(
        traveler.get_entity().pos.load(),
        Vector3::new(10.0, 82.0, 11.0)
    );
    assert_eq!(traveler.get_entity().yaw.load(), 42.0);
    assert_eq!(traveler.get_entity().pitch.load(), 15.0);
    assert!(
        destination
            .get_entity_by_id(traveler.get_entity().entity_id)
            .is_some()
    );
    assert!(
        fixture
            .world()
            .get_entity_by_id(traveler.get_entity().entity_id)
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_mob_rider_stays_unmounted_in_source_with_original_membership() {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    let root = fixture.mob(&EntityType::COW);
    let rider = fixture.mob(&EntityType::COW);
    let original = rider.get_entity().pos.load();
    let collision = crate::entity::r#type::from_type(
        &EntityType::COW,
        original,
        &destination,
        rider.get_entity().entity_uuid,
    );
    destination.add_entity_silent(collision.clone());
    root.get_entity().add_passenger(root.clone(), rider.clone());
    tick_trip(&fixture, root.as_ref(), destination.clone());
    finish_trip(root.get_entity()).await;
    fixture.server.shutdown().await;
    assert!(Arc::ptr_eq(
        &root.get_entity().world.load_full(),
        &destination
    ));
    assert!(Arc::ptr_eq(&rider.get_entity().world.load_full(), &source));
    assert_eq!(rider.get_entity().pos.load(), original);
    assert!(Arc::ptr_eq(
        &source
            .get_entity_by_id(rider.get_entity().entity_id)
            .unwrap(),
        &rider
    ));
    assert!(Arc::ptr_eq(
        &destination
            .get_entity_by_id(collision.get_entity().entity_id)
            .unwrap(),
        &collision
    ));
    assert!(
        !rider.get_entity().has_vehicle(),
        "rejected rider kept a vehicle from a different world"
    );
    assert!(
        !root
            .get_entity()
            .has_passenger(rider.get_entity().entity_id)
    );
}
#[derive(Clone, Copy)]
enum PairingRiderAction {
    InterruptVehicle,
    Save,
    MountRider,
}

struct PairingRiderSave {
    root: Arc<dyn EntityBase>,
    rider: Arc<crate::entity::player::Player>,
    applied: Arc<AtomicBool>,
    saved: Arc<std::sync::Mutex<Option<pumpkin_nbt::compound::NbtCompound>>>,
    action: PairingRiderAction,
    replacement: Arc<dyn EntityBase>,
}

impl EventHandler<PacketSentEvent> for PairingRiderSave {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        use pumpkin_protocol::ser::NetworkReadExt;
        if event.packet_id
            != pumpkin_protocol::java::client::play::CHeadRot::to_id(
                pumpkin_data::packet::CURRENT_MC_VERSION,
            )
            || (&event.payload[..]).get_var_int().unwrap().0 != self.root.get_entity().entity_id
            || self.applied.swap(true, Ordering::SeqCst)
        {
            return Box::pin(async {});
        }
        Box::pin(async move {
            let entity = self.root.get_entity();
            assert!(!Arc::ptr_eq(&self.rider.world(), &entity.world.load_full()));
            if matches!(self.action, PairingRiderAction::InterruptVehicle) {
                entity.teleport(
                    Vector3::new(10.0, 82.0, 11.0),
                    Some(42.0),
                    Some(15.0),
                    &entity.world.load_full(),
                );
            } else if matches!(self.action, PairingRiderAction::MountRider) {
                self.replacement
                    .get_entity()
                    .add_passenger(self.replacement.clone(), self.rider.clone());
            } else {
                server
                    .player_data_storage
                    .save_all_players(server)
                    .await
                    .unwrap();
                // Read the actual flushed file independently while the live UUID session remains held.
                let reader = pumpkin_world::data::player_data::PlayerDataStorage::new(
                    self.rider
                        .world()
                        .level
                        .level_folder
                        .root_folder
                        .join("players/data"),
                    true,
                );
                let (exists, saved) = reader.load_player_data(&self.rider.gameprofile.id).unwrap();
                assert!(exists);
                *self.saved.lock().unwrap() = Some(saved);
            }
        })
    }
}

struct PairingRiderFixture {
    fixture: DeathTestWorld,
    root: Arc<dyn EntityBase>,
    rider: Arc<crate::entity::player::Player>,
    destination: Arc<crate::world::World>,
    applied: Arc<AtomicBool>,
    saved: Arc<std::sync::Mutex<Option<pumpkin_nbt::compound::NbtCompound>>>,
    replacement: Arc<dyn EntityBase>,
    source_lists: Arc<std::sync::Mutex<Vec<Vec<i32>>>>,
}

async fn pairing_source_observer(fixture: &DeathTestWorld) -> Arc<crate::entity::player::Player> {
    let watcher = fixture.player_with_packet_sink("pairing-source-observer");
    let java = watcher.client.java().unwrap();
    java.player.store(Arc::new(Some(watcher.clone())));
    java.version
        .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21);
    let source = fixture.world();
    super::tests::save_destination(&source, true).await;
    watcher
        .teleport_world(source, Vector3::new(1.0, 80.0, 1.0), None, None)
        .await;
    watcher
}

async fn pairing_rider_fixture(
    action: PairingRiderAction,
    source_observer: bool,
) -> PairingRiderFixture {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    super::rider_tests::destination_observer(&fixture, &destination).await;
    let source_watcher = if source_observer {
        Some(pairing_source_observer(&fixture).await)
    } else {
        None
    };
    let root = fixture.mob(&EntityType::COW);
    let replacement = fixture.mob(&EntityType::COW);
    let rider = fixture.player_with_packet_sink("pairing-rider");
    rider
        .client
        .java()
        .unwrap()
        .player
        .store(Arc::new(Some(rider.clone())));
    let (session, _) = fixture
        .server
        .player_data_storage
        .load_data(&rider.gameprofile.id)
        .await
        .unwrap();
    *rider.storage_session.lock().unwrap() = Some(session);
    let source_lists = Arc::new(std::sync::Mutex::new(Vec::new()));
    if let Some(observer) = source_watcher {
        let tracked = fixture
            .world()
            .entity_tracker
            .get_tracked_entity(root.get_entity().entity_id)
            .unwrap();
        assert!(Arc::ptr_eq(&tracked.entity, &root));
        assert!(
            tracked.seen_by.contains(&observer.gameprofile.id),
            "source observer never actually paired the vehicle"
        );
        fixture
            .server
            .plugin_manager
            .register::<PacketSentEvent, _>(
                Arc::new(super::rider_tests::PassengerUpdates {
                    observer,
                    vehicle_id: root.get_entity().entity_id,
                    lists: source_lists.clone(),
                }),
                EventPriority::Normal,
                true,
            );
    }
    root.get_entity().add_passenger(root.clone(), rider.clone());
    let applied = Arc::new(AtomicBool::new(false));
    let saved = Arc::new(std::sync::Mutex::new(None));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(PairingRiderSave {
                root: root.clone(),
                rider: rider.clone(),
                applied: applied.clone(),
                saved: saved.clone(),
                action,
                replacement: replacement.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    if source_observer {
        assert_eq!(
            *source_lists.lock().unwrap(),
            vec![vec![rider.entity_id()]],
            "source observer never received the original riding relationship"
        );
    }
    tick_trip(&fixture, root.as_ref(), destination.clone());
    finish_trip(root.get_entity()).await;
    PairingRiderFixture {
        fixture,
        root,
        rider,
        destination,
        applied,
        saved,
        replacement,
        source_lists,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vehicle_pairing_newer_teleport_keeps_source_rider_logout_free_of_foreign_root_vehicle() {
    let PairingRiderFixture {
        fixture,
        root,
        rider,
        destination,
        applied,
        ..
    } = pairing_rider_fixture(PairingRiderAction::InterruptVehicle, false).await;
    fixture.server.shutdown().await;
    assert!(
        applied.load(Ordering::SeqCst),
        "actual vehicle pairing callback did not run"
    );
    assert!(Arc::ptr_eq(
        &root.get_entity().world.load_full(),
        &destination
    ));
    assert_eq!(root.get_entity().pos.load(), Vector3::new(10.0, 82.0, 11.0));
    assert!(Arc::ptr_eq(&rider.world(), &fixture.world()));
    assert!(
        fixture
            .world()
            .get_player_by_id(rider.entity_id())
            .is_some()
    );
    assert!(destination.get_player_by_id(rider.entity_id()).is_none());
    let linked_before_logout = rider.get_entity().has_vehicle();
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        rider.finish_java_connection(&fixture.server),
    )
    .await
    .unwrap();
    let (_session, saved) = fixture
        .server
        .player_data_storage
        .load_data(&rider.gameprofile.id)
        .await
        .unwrap();
    let saved = saved.unwrap();
    assert_eq!(
        saved.get_string("Dimension"),
        Some(Dimension::OVERWORLD.minecraft_name)
    );
    assert!(
        saved.get_compound("RootVehicle").is_none(),
        "pairing interruption saved a foreign RootVehicle on real logout"
    );
    assert!(
        !linked_before_logout,
        "pairing interruption retained the cross-world riding link"
    );
    assert!(!root.get_entity().has_passenger(rider.entity_id()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vehicle_pairing_save_cannot_capture_source_rider_with_foreign_root_vehicle() {
    let PairingRiderFixture {
        fixture,
        root,
        rider,
        destination,
        applied,
        saved,
        ..
    } = pairing_rider_fixture(PairingRiderAction::Save, false).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        rider.client.java().unwrap().await_tasks(),
    )
    .await
    .unwrap();
    fixture.server.shutdown().await;
    assert!(
        applied.load(Ordering::SeqCst),
        "actual vehicle pairing callback did not run"
    );
    let saved = saved.lock().unwrap().take().unwrap();
    assert_eq!(
        saved.get_string("Dimension"),
        Some(Dimension::OVERWORLD.minecraft_name)
    );
    assert!(
        saved.get_compound("RootVehicle").is_none(),
        "vehicle publication callback flushed a foreign RootVehicle while rider remained in source"
    );
    assert!(Arc::ptr_eq(&rider.world(), &destination));
    assert!(
        rider
            .get_entity()
            .get_vehicle()
            .is_some_and(|vehicle| Arc::ptr_eq(&vehicle, &root)),
        "successful vehicle publication lost the riding relationship"
    );
    assert!(root.get_entity().has_passenger(rider.entity_id()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vehicle_pairing_new_mount_keeps_rider_in_source_on_new_vehicle() {
    let PairingRiderFixture {
        fixture,
        root,
        rider,
        destination,
        applied,
        replacement,
        ..
    } = pairing_rider_fixture(PairingRiderAction::MountRider, false).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        rider.client.java().unwrap().await_tasks(),
    )
    .await
    .unwrap();
    fixture.server.shutdown().await;
    assert!(applied.load(Ordering::SeqCst));
    assert!(Arc::ptr_eq(
        &root.get_entity().world.load_full(),
        &destination
    ));
    assert!(
        Arc::ptr_eq(&rider.world(), &fixture.world()),
        "old passenger continuation replaced the callback's newer mount world"
    );
    assert!(Arc::ptr_eq(
        &rider.get_entity().get_vehicle().unwrap(),
        &replacement
    ));
    assert!(replacement.get_entity().has_passenger(rider.entity_id()));
    assert!(!root.get_entity().has_passenger(rider.entity_id()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_observer_loses_rider_before_cross_world_vehicle_publication() {
    let PairingRiderFixture {
        fixture,
        rider,
        applied,
        source_lists,
        ..
    } = pairing_rider_fixture(PairingRiderAction::InterruptVehicle, true).await;
    fixture.server.shutdown().await;
    assert!(applied.load(Ordering::SeqCst));
    assert_eq!(
        *source_lists.lock().unwrap(),
        vec![vec![rider.entity_id()], vec![]],
        "the actual source observer did not receive the empty list before vehicle departure"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_vehicle_publication_restores_source_riding_relationship() {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    let root = fixture.mob(&EntityType::COW);
    let rider = fixture.player_with_packet_sink("rejected-vehicle-rider");
    root.get_entity().add_passenger(root.clone(), rider.clone());
    let collision = crate::entity::r#type::from_type(
        &EntityType::COW,
        root.get_entity().pos.load(),
        &destination,
        root.get_entity().entity_uuid,
    );
    destination.add_entity_silent(collision);
    tick_trip(&fixture, root.as_ref(), destination.clone());
    finish_trip(root.get_entity()).await;
    fixture.server.shutdown().await;
    assert!(Arc::ptr_eq(&root.get_entity().world.load_full(), &source));
    assert!(Arc::ptr_eq(&rider.world(), &source));
    assert!(
        rider
            .get_entity()
            .get_vehicle()
            .is_some_and(|vehicle| Arc::ptr_eq(&vehicle, &root)),
        "rejected vehicle lost its source riding relationship"
    );
    assert!(root.get_entity().has_passenger(rider.entity_id()));
    assert!(source.get_player_by_id(rider.entity_id()).is_some());
    assert!(destination.get_player_by_id(rider.entity_id()).is_none());
}
