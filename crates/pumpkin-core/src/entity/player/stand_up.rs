use pumpkin_data::{
    block_properties::OakTrapdoorLikeProperties,
    entity::EntityType,
    tag::{self, Taggable},
};
use pumpkin_util::math::{
    boundingbox::{BoundingBox, EntityDimensions},
    position::BlockPos,
    vector3::Vector3,
    wrap_degrees_f64,
};

use crate::world::{World, natural_spawner::is_block_dangerous};

mod inputs;

const BORDER_EPSILON: f64 = 1.0e-5f32 as f64;

// RespawnAnchorBlock.RESPAWN_HORIZONTAL_OFFSETS, in vanilla search order.
const ANCHOR_OFFSETS: [(i32, i32); 8] = [
    (0, -1),
    (-1, 0),
    (0, 1),
    (1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

pub(super) fn anchor_stand_up(world: &World, pos: BlockPos) -> Option<Vector3<f64>> {
    // RespawnAnchorBlock.findStandUpPosition exhausts the safe pass first.
    for check_dangerous in [true, false] {
        for dy in [0, -1, 1] {
            if let Some(position) =
                stand_up_at_offsets(world, pos.add(0, dy, 0), &ANCHOR_OFFSETS, check_dangerous)
            {
                return Some(position);
            }
        }
        if let Some(position) = safe_dismount(world, pos.up(), check_dangerous) {
            return Some(position);
        }
    }
    None
}

fn stand_up_at_offsets(
    world: &World,
    pos: BlockPos,
    offsets: &[(i32, i32)],
    check_dangerous: bool,
) -> Option<Vector3<f64>> {
    offsets
        .iter()
        .find_map(|&(x, z)| safe_dismount(world, pos.add(x, 0, z), check_dangerous))
}

// DismountHelper.findSafeDismountLocation: floor collision is required on both passes.
pub(super) fn safe_dismount(
    world: &World,
    pos: BlockPos,
    check_dangerous: bool,
) -> Option<Vector3<f64>> {
    let kind = &EntityType::PLAYER;
    let mut inputs = inputs::Inputs::default();
    let state = inputs.read(world, pos)?;
    if check_dangerous && is_block_dangerous(kind, state) {
        return None;
    }
    let mut floor = non_climbable_shape_height(world, pos, &mut inputs)?;
    if floor == f64::NEG_INFINITY {
        let below = non_climbable_shape_height(world, pos.down(), &mut inputs)?;
        floor = if below >= 1.0 {
            below - 1.0
        } else {
            f64::NEG_INFINITY
        };
    }
    if !floor.is_finite() || floor >= 1.0 {
        return None;
    }
    if check_dangerous && floor <= 0.0 && is_block_dangerous(kind, inputs.read(world, pos.down())?)
    {
        return None;
    }
    if state
        .id
        .to_block()
        .has_tag(&tag::Block::MINECRAFT_INVALID_SPAWN_INSIDE)
        || inputs
            .read(world, pos.up())?
            .id
            .to_block()
            .has_tag(&tag::Block::MINECRAFT_INVALID_SPAWN_INSIDE)
    {
        return None;
    }
    let position = pos.to_f64().add_raw(0.0, floor, 0.0);
    let dimensions = EntityDimensions::new(kind.width(), kind.height(), kind.eye_height);
    let bounds = BoundingBox::new_from_pos(position.x, position.y, position.z, &dimensions);
    for cell in BlockPos::iterate(
        bounds.min_block_pos().add(-1, -1, -1),
        bounds.max_block_pos().add(1, 1, 1),
    ) {
        if !world.is_in_height_limit(cell.0.y) {
            continue;
        }
        let state = inputs.read(world, cell)?;
        let mut collision = false;
        world.for_each_collision_shape(state, &cell, None, &mut |shape| {
            collision |= shape.at_pos(cell).intersects(&bounds);
        });
        if collision {
            return None;
        }
    }
    let inside_border = {
        let border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // WorldBorder.isWithinBounds(AABB) includes a body flush with the upper edge.
        border.contains(bounds.min.x, bounds.min.z)
            && border.contains(bounds.max.x - BORDER_EPSILON, bounds.max.z - BORDER_EPSILON)
    };
    (inside_border && inputs.is_current(world)).then_some(position)
}

fn non_climbable_shape_height(
    world: &World,
    pos: BlockPos,
    inputs: &mut inputs::Inputs,
) -> Option<f64> {
    let state = inputs.read(world, pos)?;
    let block = state.id.to_block();
    // DismountHelper.nonClimbableShape excludes ladders and open trapdoors as support.
    if block.has_tag(&tag::Block::MINECRAFT_CLIMBABLE)
        || (block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS)
            && OakTrapdoorLikeProperties::from_state_id(state.id).open)
    {
        return Some(f64::NEG_INFINITY);
    }
    let mut height = f64::NEG_INFINITY;
    world.for_each_collision_shape(state, &pos, None, &mut |shape| {
        height = height.max(shape.max.y);
    });
    Some(height)
}

pub(super) fn look_at_yaw(position: Vector3<f64>, spawn: BlockPos) -> f32 {
    // ServerPlayer.RespawnPosAngle.calculateLookAtYaw.
    let direction = (spawn.to_f64() - position).normalize();
    wrap_degrees_f64(direction.z.atan2(direction.x).to_degrees() - 90.0) as f32
}
