use pumpkin_data::{entity::EntityType, fluid::Fluid};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

use crate::{
    entity::{Entity, EntityPose},
    world::World,
};

const ESCAPE_CLEARANCE: f32 = 1.0E-5;
const SQRT_OF_TWO: f32 = std::f32::consts::SQRT_2;
const DISMOUNT_POSES: [EntityPose; 3] = [
    EntityPose::Standing,
    EntityPose::Crouching,
    EntityPose::Swimming,
];

const BOAT_ENTITY_IDS: &[u16] = &[
    EntityType::ACACIA_BOAT.id,
    EntityType::ACACIA_CHEST_BOAT.id,
    EntityType::BAMBOO_RAFT.id,
    EntityType::BAMBOO_CHEST_RAFT.id,
    EntityType::BIRCH_BOAT.id,
    EntityType::BIRCH_CHEST_BOAT.id,
    EntityType::CHERRY_BOAT.id,
    EntityType::CHERRY_CHEST_BOAT.id,
    EntityType::DARK_OAK_BOAT.id,
    EntityType::DARK_OAK_CHEST_BOAT.id,
    EntityType::JUNGLE_BOAT.id,
    EntityType::JUNGLE_CHEST_BOAT.id,
    EntityType::MANGROVE_BOAT.id,
    EntityType::MANGROVE_CHEST_BOAT.id,
    EntityType::OAK_BOAT.id,
    EntityType::OAK_CHEST_BOAT.id,
    EntityType::PALE_OAK_BOAT.id,
    EntityType::PALE_OAK_CHEST_BOAT.id,
    EntityType::POPLAR_BOAT.id,
    EntityType::POPLAR_CHEST_BOAT.id,
    EntityType::SPRUCE_BOAT.id,
    EntityType::SPRUCE_CHEST_BOAT.id,
];

/// Returns a boat-specific destination, leaving other vehicle policies untouched.
pub(in crate::entity) fn dismount_position_if_boat(
    vehicle: &Entity,
    passenger: &Entity,
    world: &World,
    vehicle_box: BoundingBox,
    fallback: Vector3<f64>,
) -> Option<Vector3<f64>> {
    if !BOAT_ENTITY_IDS.contains(&vehicle.entity_type.id) {
        return None;
    }

    Some(dismount_position(
        vehicle,
        passenger,
        world,
        vehicle_box,
        fallback,
    ))
}

// AbstractBoat.getDismountLocationForPassenger; the fallback is Entity's exact above-vehicle location.
fn dismount_position(
    vehicle: &Entity,
    passenger: &Entity,
    world: &World,
    vehicle_box: BoundingBox,
    fallback: Vector3<f64>,
) -> Vector3<f64> {
    let vehicle_pos = vehicle.pos.load();
    let vehicle_width = vehicle.entity_dimension.load().width * SQRT_OF_TWO;
    let passenger_width = passenger.entity_dimension.load().width;
    let distance =
        (f64::from(vehicle_width) + f64::from(passenger_width) + f64::from(ESCAPE_CLEARANCE)) / 2.0;

    // Java computes the angle and trigonometric components as floats before scaling in Vec3.
    let radians = passenger.yaw.load() * (std::f32::consts::PI / 180.0);
    let direction_x = -radians.sin();
    let direction_z = radians.cos();
    let scale = direction_x.abs().max(direction_z.abs());
    let target_x = vehicle_pos.x + f64::from(direction_x) * distance / f64::from(scale);
    let target_z = vehicle_pos.z + f64::from(direction_z) * distance / f64::from(scale);

    let target_block = BlockPos(Vector3::new(
        target_x.floor() as i32,
        vehicle_box.max.y.floor() as i32,
        target_z.floor() as i32,
    ));
    let below_target = BlockPos(Vector3::new(
        target_block.0.x,
        target_block.0.y - 1,
        target_block.0.z,
    ));

    if is_water_at(world, &below_target) {
        return fallback;
    }

    let mut targets = Vec::with_capacity(2);
    let target_floor = world.get_dismount_height(&target_block);
    if is_valid_floor(target_floor) {
        targets.push(Vector3::new(
            target_x,
            f64::from(target_block.0.y) + target_floor,
            target_z,
        ));
    }

    let below_floor = world.get_dismount_height(&below_target);
    if is_valid_floor(below_floor) {
        targets.push(Vector3::new(
            target_x,
            f64::from(below_target.0.y) + below_floor,
            target_z,
        ));
    }

    for pose in DISMOUNT_POSES {
        let mut dimensions = Entity::get_entity_dimensions(pose);
        dimensions.width = passenger.entity_dimension.load().width;
        for target in &targets {
            let bounds = BoundingBox::new_from_pos(target.x, target.y, target.z, &dimensions);
            if can_dismount_to(world, bounds) {
                passenger.set_pose(pose);
                return *target;
            }
        }
    }

    fallback
}

fn is_water_at(world: &World, pos: &BlockPos) -> bool {
    let fluid = world.get_fluid_and_fluid_state(pos).0;
    fluid.id == Fluid::WATER.id || fluid.id == Fluid::FLOWING_WATER.id
}

fn is_valid_floor(height: f64) -> bool {
    !height.is_infinite() && height < 1.0
}

fn can_dismount_to(world: &World, bounds: BoundingBox) -> bool {
    if !world.is_space_empty(bounds) {
        return false;
    }

    let border = world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    border.contains(bounds.min.x, bounds.min.z) && border.contains(bounds.max.x, bounds.max.z)
}
