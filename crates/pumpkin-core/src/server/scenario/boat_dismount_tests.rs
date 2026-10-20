use super::*;
use crate::{
    entity::{Entity, EntityBase},
    plugin::{
        EventHandler, EventPriority,
        api::events::{
            entity::entity_dismount::EntityDismountEvent, vehicle::vehicle_exit::VehicleExitEvent,
        },
    },
    server::Server,
    world::World,
};
use futures::future::BoxFuture;
use pumpkin_data::{
    Block,
    block_properties::{OakSlabProperties, SlabType},
    entity::{EntityPose, EntityType},
};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{ActionType, SInteract, SPlayerInput},
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const MAIN_HAND: VarInt = VarInt(0);

fn set_block(world: &Arc<World>, pos: BlockPos, block: &Block) {
    set_state(world, pos, block.default_state.id);
}

fn set_state(world: &Arc<World>, pos: BlockPos, state_id: pumpkin_data::BlockStateId) {
    world.set_block_state(&pos, state_id, BlockFlags::FORCE_STATE);
    assert_eq!(
        world.get_block_state_id(&pos),
        state_id,
        "fixture block state at {pos:?}"
    );
}

fn board(
    scenario: &Scenario,
    player: &Session,
    vehicle: &Arc<dyn EntityBase>,
    center: Vector3<f64>,
    passenger_yaw: f32,
) {
    player.player.get_entity().set_pos(center);
    player.player.get_entity().yaw.store(passenger_yaw);
    player.send(
        &scenario.server,
        &SInteract {
            entity_id: vehicle.get_entity().entity_id.into(),
            r#type: VarInt(ActionType::Interact as i32),
            hand: Some(MAIN_HAND),
            target_position: None,
            sneaking: false,
        },
    );
    assert!(
        player.player.get_entity().has_vehicle(),
        "ordinary interact did not board the vehicle"
    );
    assert!(
        vehicle
            .get_entity()
            .has_passenger(player.player.entity_id())
    );
}

fn actual_dismount(
    scenario: &Scenario,
    player: &Session,
    vehicle_type: &'static EntityType,
    center: Vector3<f64>,
    passenger_yaw: f32,
) -> (Arc<dyn EntityBase>, Vector3<f64>) {
    let world = player.player.world();
    let vehicle = Scenario::mob(&world, vehicle_type, center);
    // Make the vehicle's yaw differ from each passenger direction.
    vehicle.get_entity().yaw.store(17.0);
    board(scenario, player, &vehicle, center, passenger_yaw);
    player.send(
        &scenario.server,
        &SPlayerInput {
            input: SPlayerInput::SNEAK,
        },
    );
    let position = player.player.get_entity().pos.load();
    assert!(!player.player.get_entity().has_vehicle());
    assert!(
        !vehicle
            .get_entity()
            .has_passenger(player.player.entity_id())
    );
    let passenger = player.player.get_entity();
    let bounds = passenger.bounding_box.load();
    assert!(
        world.is_space_empty(bounds),
        "dismount placed the passenger inside solid blocks at {position:?} with bounds {bounds:?}"
    );
    player.confirm_teleport(&scenario.server);
    (vehicle, position)
}

fn assert_position_close(actual: Vector3<f64>, expected: Vector3<f64>, reason: &str) {
    assert!(
        (actual.x - expected.x).abs() < 1.0e-4
            && (actual.y - expected.y).abs() < 1.0e-4
            && (actual.z - expected.z).abs() < 1.0e-4,
        "{reason}: got {actual:?}, expected {expected:?}"
    );
}

fn above_vehicle(vehicle: &dyn EntityBase, center: Vector3<f64>) -> Vector3<f64> {
    Vector3::new(
        center.x,
        vehicle.get_entity().bounding_box.load().max.y,
        center.z,
    )
}

#[tokio::test]
async fn b1020d_g05_boat_dry_bank_uses_passenger_yaw() {
    run(async {
        let scenario = Scenario::new().await;
        let dry_rider = scenario.join(Uuid::new_v4(), "DryBank").await;
        let world = dry_rider.player.world();

        let dry_center = Vector3::new(8.3, 64.0, 10.5);
        set_block(&world, BlockPos::new(8, 63, 10), &Block::WATER);
        let (boat, landed) = actual_dismount(
            &scenario,
            &dry_rider,
            &EntityType::OAK_BOAT,
            dry_center,
            270.0,
        );
        assert_position_close(
            landed,
            Vector3::new(9.572_276_8, 64.0, 10.5),
            "passenger facing east should land on the dry stone bank despite vehicle yaw",
        );
        assert!(!dry_rider.player.get_entity().has_vehicle());
        assert!(
            !boat
                .get_entity()
                .has_passenger(dry_rider.player.entity_id())
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020d_g05_boat_open_water_keeps_above_vehicle_fallback() {
    run(async {
        let scenario = Scenario::new().await;
        let rider = scenario.join(Uuid::new_v4(), "OpenWater").await;
        let world = rider.player.world();
        let center = Vector3::new(8.3, 64.0, 10.5);
        set_block(&world, BlockPos::new(8, 63, 10), &Block::WATER);
        set_block(&world, BlockPos::new(9, 63, 10), &Block::WATER);

        let (boat, landed) =
            actual_dismount(&scenario, &rider, &EntityType::OAK_BOAT, center, 270.0);
        assert_position_close(
            landed,
            above_vehicle(boat.as_ref(), center),
            "open water below the passenger-facing target should retain the above-boat fallback",
        );
        scenario.stop().await;
    })
    .await;
}

struct CancelDismount(Arc<AtomicUsize>);

impl EventHandler<EntityDismountEvent> for CancelDismount {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityDismountEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

struct CancelVehicleExit(Arc<AtomicUsize>);

impl EventHandler<VehicleExitEvent> for CancelVehicleExit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut VehicleExitEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

fn assert_cardinal_dismounts(scenario: &Scenario, rider: &Session, world: &Arc<World>) {
    for pos in [
        BlockPos::new(9, 63, 10),
        BlockPos::new(7, 63, 10),
        BlockPos::new(8, 63, 11),
        BlockPos::new(8, 63, 9),
    ] {
        set_block(world, pos, &Block::STONE);
    }
    let center = Vector3::new(8.3, 64.0, 10.5);
    for (yaw, expected) in [
        (270.0, Vector3::new(9.572_276_8, 64.0, 10.5)),
        (90.0, Vector3::new(7.027_723_2, 64.0, 10.5)),
        (0.0, Vector3::new(8.3, 64.0, 11.772_276_8)),
        (180.0, Vector3::new(8.3, 64.0, 9.227_723_2)),
    ] {
        let (_, landed) = actual_dismount(scenario, rider, &EntityType::OAK_BOAT, center, yaw);
        assert_position_close(landed, expected, "boat escape should follow passenger yaw");
    }
}

fn assert_half_slab_landing(scenario: &Scenario, rider: &Session, world: &Arc<World>) {
    let center = Vector3::new(8.3, 64.0, 10.5);
    set_block(world, BlockPos::new(9, 63, 10), &Block::STONE);
    set_block(world, BlockPos::new(9, 64, 10), &Block::STONE_SLAB);
    assert_eq!(
        world.get_dismount_height(&BlockPos::new(9, 64, 10)),
        0.5,
        "fixture must expose the bottom slab's half-block floor"
    );
    assert!(world.is_space_empty(BoundingBox::new_from_pos(
        9.572_276_8,
        64.5,
        10.5,
        &Entity::get_entity_dimensions(EntityPose::Standing),
    )));
    let (_, landed) = actual_dismount(scenario, rider, &EntityType::OAK_BOAT, center, 270.0);
    assert_position_close(
        landed,
        Vector3::new(9.572_276_8, 64.5, 10.5),
        "a lower-half slab is a valid half-block floor",
    );
}

fn assert_top_slab_falls_back(scenario: &Scenario, rider: &Session, world: &Arc<World>) {
    let center = Vector3::new(8.7, 64.0, 10.5);
    set_block(world, BlockPos::new(7, 63, 10), &Block::STONE);
    let mut top_slab = OakSlabProperties::default(&Block::OAK_SLAB);
    top_slab.r#type = SlabType::Top;
    set_state(
        world,
        BlockPos::new(7, 64, 10),
        top_slab.to_state_id(&Block::OAK_SLAB),
    );
    let (vehicle, landed) = actual_dismount(scenario, rider, &EntityType::OAK_BOAT, center, 90.0);
    assert_position_close(
        landed,
        above_vehicle(vehicle.as_ref(), center),
        "a full-height top slab surface is invalid and must use the fallback",
    );
}

fn assert_crouching_fits_under_ceiling(scenario: &Scenario, rider: &Session, world: &Arc<World>) {
    let center = Vector3::new(12.3, 64.0, 10.5);
    set_block(world, BlockPos::new(13, 63, 10), &Block::STONE);
    let mut ceiling_slab = OakSlabProperties::default(&Block::OAK_SLAB);
    ceiling_slab.r#type = SlabType::Top;
    set_state(
        world,
        BlockPos::new(13, 65, 10),
        ceiling_slab.to_state_id(&Block::OAK_SLAB),
    );
    let (_, landed) = actual_dismount(scenario, rider, &EntityType::OAK_BOAT, center, 270.0);
    assert_position_close(
        landed,
        Vector3::new(13.572_276_8, 64.0, 10.5),
        "crouching should fit beneath a top-half slab after standing is blocked",
    );
    assert!(rider.player.get_entity().pose.load() == EntityPose::Crouching);
    assert!(world.is_space_empty(rider.player.get_entity().bounding_box.load()));
    set_block(world, BlockPos::new(13, 65, 10), &Block::AIR);
}

fn assert_obstructed_target_falls_back(scenario: &Scenario, rider: &Session, world: &Arc<World>) {
    let center = Vector3::new(4.3, 64.0, 10.5);
    set_block(world, BlockPos::new(5, 63, 10), &Block::STONE);
    set_block(world, BlockPos::new(5, 64, 10), &Block::STONE_SLAB);
    set_block(world, BlockPos::new(5, 65, 10), &Block::STONE);
    rider.player.get_entity().set_pose(EntityPose::Standing);
    let (vehicle, landed) = actual_dismount(scenario, rider, &EntityType::OAK_BOAT, center, 270.0);
    assert_position_close(
        landed,
        above_vehicle(vehicle.as_ref(), center),
        "an obstructed floor must fall back above the boat",
    );
    assert!(world.is_space_empty(rider.player.get_entity().bounding_box.load()));
}

fn set_test_world_border(world: &Arc<World>) {
    let mut border = world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    border.set_center(world, 0.0, 0.0);
    border.set_diameter(world, 30.5, None);
}

fn assert_border_target_falls_back(scenario: &Scenario, rider: &Session, world: &Arc<World>) {
    set_test_world_border(world);
    let center = Vector3::new(14.0, 64.0, 10.5);
    set_block(world, BlockPos::new(15, 63, 10), &Block::STONE);
    let (vehicle, landed) = actual_dismount(scenario, rider, &EntityType::OAK_BOAT, center, 270.0);
    assert_position_close(
        landed,
        above_vehicle(vehicle.as_ref(), center),
        "a passenger box outside the world border must use the fallback",
    );
    assert!(world.is_space_empty(rider.player.get_entity().bounding_box.load()));
}

#[tokio::test]
async fn b1020d_g05_boat_dismount_floor_pose_border_controls() {
    run(async {
        let scenario = Scenario::new().await;
        let rider = scenario.join(Uuid::new_v4(), "DismountControls").await;
        let world = rider.player.world();
        assert_cardinal_dismounts(&scenario, &rider, &world);
        assert_half_slab_landing(&scenario, &rider, &world);
        assert_top_slab_falls_back(&scenario, &rider, &world);
        assert_crouching_fits_under_ceiling(&scenario, &rider, &world);
        assert_obstructed_target_falls_back(&scenario, &rider, &world);
        assert_border_target_falls_back(&scenario, &rider, &world);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020d_g05_minecart_keeps_generic_dismount_destination() {
    run(async {
        let scenario = Scenario::new().await;
        let rider = scenario.join(Uuid::new_v4(), "MinecartControl").await;
        let center = Vector3::new(8.3, 64.0, 10.5);
        let world = rider.player.world();
        set_block(&world, BlockPos::new(7, 63, 10), &Block::STONE);
        let (_, landed) = actual_dismount(&scenario, &rider, &EntityType::MINECART, center, 270.0);
        assert_position_close(
            landed,
            Vector3::new(7.5, 64.0, 10.5),
            "minecart destinations must retain the generic vehicle-yaw search",
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g05_cancelled_vehicle_exit_keeps_rider_mounted() {
    run(async {
        let scenario = Scenario::new().await;
        let rider = scenario.join(Uuid::new_v4(), "CancelledDismount").await;
        let center = Vector3::new(20.3, 64.0, 28.5);
        let world = rider.player.world();
        let vehicle = Scenario::mob(&world, &EntityType::OAK_BOAT, center);
        vehicle.get_entity().yaw.store(17.0);
        let dismount_count = Arc::new(AtomicUsize::new(0));
        let exit_count = Arc::new(AtomicUsize::new(0));
        scenario
            .server
            .plugin_manager
            .register::<EntityDismountEvent, _>(
                Arc::new(CancelDismount(dismount_count.clone())),
                EventPriority::Lowest,
                true,
            );
        scenario
            .server
            .plugin_manager
            .register::<VehicleExitEvent, _>(
                Arc::new(CancelVehicleExit(exit_count.clone())),
                EventPriority::Lowest,
                true,
            );
        board(&scenario, &rider, &vehicle, center, 270.0);
        rider.send(
            &scenario.server,
            &SPlayerInput {
                input: SPlayerInput::SNEAK,
            },
        );
        assert_eq!(dismount_count.load(Ordering::Relaxed), 1);
        assert_eq!(exit_count.load(Ordering::Relaxed), 1);
        assert!(rider.player.get_entity().has_vehicle());
        assert!(vehicle.get_entity().has_passenger(rider.player.entity_id()));
        assert_eq!(
            rider.player.get_entity().pos.load(),
            center,
            "cancelled dismount must not commit the destination"
        );
        scenario.stop().await;
    })
    .await;
}
