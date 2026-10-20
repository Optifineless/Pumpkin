#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real portal regressions bound completion and inspect admitted world identities"
)]

use super::{
    b1020d_g06_r2_tests::portal_frame,
    b1020d_g06_test_support::{DEADLINE, Fixture},
};
use crate::{
    entity::{EntityBase, r#type::from_type},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_change_world::PlayerChangeWorldEvent,
        vehicle::vehicle_update::VehicleUpdateEvent,
    },
    server::{Server, combat_test_support},
};
use pumpkin_data::{dimension::Dimension, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct PlayerTransfer {
    cancel: bool,
    observed: AtomicUsize,
    mount: Option<Arc<dyn EntityBase>>,
}

impl EventHandler<PlayerChangeWorldEvent> for PlayerTransfer {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        self.observed.fetch_add(1, Ordering::Relaxed);
        if let Some(mount) = &self.mount {
            mount
                .get_entity()
                .add_passenger(mount.clone(), event.player.clone() as Arc<dyn EntityBase>);
        }
        event.cancelled = self.cancel;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_portal_player_acceptance_and_cancellation_keep_world_local_riding() {
    for cancel in [false, true] {
        let mut fixture = Fixture::new();
        let destination = fixture.other_world(Dimension::THE_NETHER);
        portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
        portal_frame(&destination, BlockPos::new(1, 64, 1));
        let vessel = fixture.boat(&EntityType::OAK_BOAT);
        vessel.get_entity().set_pos(Vector3::new(9.0, 64.0, 8.5));
        vessel.get_entity().add_passenger(
            vessel.clone(),
            fixture.player.player.clone() as Arc<dyn EntityBase>,
        );
        assert!(fixture.player.player.get_entity().has_vehicle());
        let handler = Arc::new(PlayerTransfer {
            cancel,
            observed: AtomicUsize::new(0),
            mount: None,
        });
        fixture
            .server
            .plugin_manager
            .register::<PlayerChangeWorldEvent, _>(handler.clone(), EventPriority::Normal, true);
        fixture.tick(2);
        let server = fixture.server.clone();
        tokio::time::timeout(
            DEADLINE,
            fixture
                .player
                .with_outgoing_writer(combat_test_support::wait_for_tasks(&server)),
        )
        .await
        .expect("portal and existing player transfer must finish within their deadline");
        assert_eq!(handler.observed.load(Ordering::Relaxed), 1);
        assert!(vessel.get_entity().is_removed());
        let restored = destination
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .expect("root boat must arrive regardless of player callback cancellation");
        let player = fixture.player.player.get_entity();
        if cancel {
            assert!(Arc::ptr_eq(&player.world.load_full(), &fixture.world));
            assert!(
                !player.has_vehicle(),
                "cancelled player must not retain a cross-world mount"
            );
            assert!(restored.get_entity().passengers.lock().unwrap().is_empty());
            assert!(
                fixture
                    .world
                    .get_player_by_uuid(player.entity_uuid)
                    .is_some()
            );
            assert!(destination.get_player_by_uuid(player.entity_uuid).is_none());
        } else {
            assert!(Arc::ptr_eq(&player.world.load_full(), &destination));
            assert!(Arc::ptr_eq(&player.get_vehicle().unwrap(), &restored));
            assert!(restored.get_entity().has_passenger(player.entity_id));
            assert!(
                fixture
                    .world
                    .get_player_by_uuid(player.entity_uuid)
                    .is_none()
            );
            assert!(destination.get_player_by_uuid(player.entity_uuid).is_some());
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_destination_uuid_conflict_preserves_source_boat() {
    let fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
    portal_frame(&destination, BlockPos::new(1, 64, 1));
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    let origin = Vector3::new(9.0, 64.0, 8.5);
    vessel.get_entity().set_pos(origin);
    let existing = from_type(
        &EntityType::OAK_BOAT,
        origin,
        &destination,
        vessel.get_entity().entity_uuid,
    );
    destination.spawn_entity_non_save(existing.clone());
    fixture.tick(2);
    tokio::time::timeout(
        DEADLINE,
        combat_test_support::wait_for_tasks(&fixture.server),
    )
    .await
    .unwrap();
    assert!(vessel.get_entity().is_alive());
    assert_eq!(vessel.get_entity().pos.load(), origin);
    assert!(Arc::ptr_eq(
        &vessel.get_entity().world.load_full(),
        &fixture.world
    ));
    assert!(Arc::ptr_eq(
        &fixture
            .world
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .unwrap(),
        &vessel,
    ));
    assert!(Arc::ptr_eq(
        &destination
            .get_entity_by_uuid(existing.get_entity().entity_uuid)
            .unwrap(),
        &existing,
    ));
    assert_eq!(destination.entities.load().len(), 1);
    fixture.finish().await;
}

struct UnloadDestination(AtomicUsize);

impl EventHandler<VehicleUpdateEvent> for UnloadDestination {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _: &'a mut VehicleUpdateEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                tokio::time::timeout(DEADLINE, server.unload_world("minecraft:the_nether"))
                    .await
                    .unwrap()
                    .unwrap();
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_pending_portal_to_retired_world_preserves_source_boat() {
    let fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
    portal_frame(&destination, BlockPos::new(1, 64, 1));
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    let origin = Vector3::new(9.0, 64.0, 8.5);
    vessel.get_entity().set_pos(origin);
    fixture.tick(1);
    assert!(vessel.get_entity().portal_manager.lock().unwrap().is_some());
    let callback = Arc::new(UnloadDestination(AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<VehicleUpdateEvent, _>(callback.clone(), EventPriority::Normal, true);
    fixture.tick(1);
    tokio::time::timeout(
        DEADLINE,
        combat_test_support::wait_for_tasks(&fixture.server),
    )
    .await
    .unwrap();
    assert_eq!(callback.0.load(Ordering::Relaxed), 1);
    assert!(destination.level.cancel_token.is_cancelled());
    assert!(vessel.get_entity().is_alive());
    assert_eq!(vessel.get_entity().pos.load(), origin);
    assert!(Arc::ptr_eq(
        &vessel.get_entity().world.load_full(),
        &fixture.world
    ));
    assert!(
        destination
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .is_none()
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_container_passenger_keeps_one_inventory_owner_across_portal() {
    let fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
    portal_frame(&destination, BlockPos::new(1, 64, 1));
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    vessel.get_entity().set_pos(Vector3::new(9.0, 64.0, 8.5));
    let passenger = from_type(
        &EntityType::CHEST_MINECART,
        vessel.get_entity().pos.load(),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    let cart = passenger
        .cast_any()
        .downcast_ref::<super::minecart::MinecartEntity>()
        .unwrap();
    let retained_inventory = cart.container().unwrap().clone();
    retained_inventory.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    fixture.world.spawn_entity_non_save(passenger.clone());
    vessel
        .get_entity()
        .add_passenger(vessel.clone(), passenger.clone());
    fixture.tick(2);
    tokio::time::timeout(
        DEADLINE,
        combat_test_support::wait_for_tasks(&fixture.server),
    )
    .await
    .unwrap();
    let transferred = destination
        .get_entity_by_uuid(passenger.get_entity().entity_uuid)
        .expect("container passenger must reach the destination");
    assert!(Arc::ptr_eq(&transferred, &passenger));
    let cart = transferred
        .cast_any()
        .downcast_ref::<super::minecart::MinecartEntity>()
        .unwrap();
    let arrived_inventory = cart.container().unwrap();
    assert!(Arc::ptr_eq(arrived_inventory, &retained_inventory));
    let taken = retained_inventory.remove_stack_specific(0, 7);
    assert_eq!(taken.item_count, 7);
    assert_eq!(taken.item, &Item::DIAMOND);
    assert!(
        arrived_inventory.get_stack(0).is_empty(),
        "a retained pre-transfer inventory reference must not leave a second payable copy"
    );
    assert!(
        fixture
            .world
            .get_entity_by_uuid(passenger.get_entity().entity_uuid)
            .is_none()
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_player_transfer_callback_source_mount_cannot_cross_worlds() {
    let mut fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
    portal_frame(&destination, BlockPos::new(1, 64, 1));
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    vessel.get_entity().set_pos(Vector3::new(9.0, 64.0, 8.5));
    let other = fixture.boat(&EntityType::BAMBOO_RAFT);
    other.get_entity().set_pos(Vector3::new(13.0, 64.0, 13.0));
    vessel.get_entity().add_passenger(
        vessel.clone(),
        fixture.player.player.clone() as Arc<dyn EntityBase>,
    );
    let handler = Arc::new(PlayerTransfer {
        cancel: false,
        observed: AtomicUsize::new(0),
        mount: Some(other.clone()),
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(handler.clone(), EventPriority::Normal, true);
    fixture.tick(2);
    let server = fixture.server.clone();
    tokio::time::timeout(
        DEADLINE,
        fixture
            .player
            .with_outgoing_writer(combat_test_support::wait_for_tasks(&server)),
    )
    .await
    .unwrap();
    assert_eq!(handler.observed.load(Ordering::Relaxed), 1);
    let player = fixture.player.player.get_entity();
    assert!(Arc::ptr_eq(&player.world.load_full(), &destination));
    assert!(!other.get_entity().has_passenger(player.entity_id));
    if let Some(mount) = player.get_vehicle() {
        assert!(Arc::ptr_eq(
            &mount.get_entity().world.load_full(),
            &destination
        ));
        assert_eq!(
            mount.get_entity().entity_uuid,
            vessel.get_entity().entity_uuid
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_pending_destination_uuid_conflict_preserves_saved_entity() {
    let fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
    portal_frame(&destination, BlockPos::new(1, 64, 1));
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    let origin = Vector3::new(9.0, 64.0, 8.5);
    vessel.get_entity().set_pos(origin);
    let existing = from_type(
        &EntityType::OAK_BOAT,
        Vector3::new(2.0, 64.0, 1.5),
        &destination,
        vessel.get_entity().entity_uuid,
    );
    let saved_name = pumpkin_util::text::TextComponent::text("already saved destination boat");
    existing.get_entity().set_custom_name(saved_name.clone());
    let mut nbt = pumpkin_nbt::NbtCompound::new();
    existing.write_nbt(&mut nbt);
    let storage = tokio::time::timeout(
        DEADLINE,
        destination
            .level
            .get_entity_chunk(existing.get_entity().chunk_pos.load()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!storage.live.load(Ordering::Acquire));
    storage.data.lock().unwrap().push(nbt);
    fixture.tick(2);
    tokio::time::timeout(
        DEADLINE,
        combat_test_support::wait_for_tasks(&fixture.server),
    )
    .await
    .unwrap();
    assert!(vessel.get_entity().is_alive());
    assert_eq!(vessel.get_entity().pos.load(), origin);
    assert!(destination.make_chunk_entities_live(&storage, None));
    let saved = destination
        .get_entity_by_uuid(existing.get_entity().entity_uuid)
        .expect("subsequent activation must retain the conflicting saved destination entity");
    assert_eq!(
        *saved.get_entity().custom_name.load_full(),
        Some(saved_name)
    );
    assert_eq!(destination.entities.load().len(), 1);
    fixture.finish().await;
}

struct UnloadPlayerDestination(AtomicUsize);

impl EventHandler<PlayerChangeWorldEvent> for UnloadPlayerDestination {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::Relaxed);
            tokio::time::timeout(
                DEADLINE,
                server.unload_world(event.new_world.dimension.minecraft_name),
            )
            .await
            .unwrap()
            .unwrap();
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_player_approval_callback_destination_unload_preserves_both() {
    let mut fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(8, 64, 8));
    portal_frame(&destination, BlockPos::new(1, 64, 1));
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    vessel.get_entity().set_pos(Vector3::new(9.0, 64.0, 8.5));
    vessel.get_entity().add_passenger(
        vessel.clone(),
        fixture.player.player.clone() as Arc<dyn EntityBase>,
    );
    let callback = Arc::new(UnloadPlayerDestination(AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(callback.clone(), EventPriority::Normal, true);
    fixture.tick(2);
    let server = fixture.server.clone();
    tokio::time::timeout(
        DEADLINE,
        fixture
            .player
            .with_outgoing_writer(combat_test_support::wait_for_tasks(&server)),
    )
    .await
    .expect("player approval callback must retire its destination without waiting for transfer");
    assert_eq!(callback.0.load(Ordering::Relaxed), 1);
    assert!(destination.level.cancel_token.is_cancelled());
    let player = fixture.player.player.get_entity();
    assert!(Arc::ptr_eq(&player.world.load_full(), &fixture.world));
    assert!(!player.has_vehicle());
    assert!(
        fixture
            .world
            .get_player_by_uuid(player.entity_uuid)
            .is_some()
    );
    assert!(destination.get_player_by_uuid(player.entity_uuid).is_none());
    let reloaded = fixture
        .server
        .create_world("destination".into(), Dimension::THE_NETHER);
    let storage = tokio::time::timeout(
        DEADLINE,
        reloaded
            .level
            .get_entity_chunk(super::b1020d_g06_test_support::CHUNK),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(reloaded.make_chunk_entities_live(&storage, None));
    assert!(
        reloaded
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .is_some(),
        "retirement must save the arrived boat while refusing the player"
    );
    reloaded.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}
