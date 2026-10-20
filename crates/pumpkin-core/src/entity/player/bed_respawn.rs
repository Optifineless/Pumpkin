use pumpkin_data::{
    block_properties::HorizontalFacing,
    tag::{self, Taggable},
};
use pumpkin_util::math::{cos, position::BlockPos, sin, vector3::Vector3};

use crate::{entity::player::stand_up, world::World};

pub(super) fn find_bed_spawn_position(
    world: &World,
    bed_pos: BlockPos,
    facing: HorizontalFacing,
    yaw: f32,
) -> Option<Vector3<f64>> {
    let (forward_x, forward_z) = match facing {
        HorizontalFacing::North => (0, -1),
        HorizontalFacing::South => (0, 1),
        HorizontalFacing::West => (-1, 0),
        HorizontalFacing::East => (1, 0),
    };
    let (right_x, right_z) = (-forward_z, forward_x);
    let (side_x, side_z) =
        if right_x as f32 * -sin(yaw.to_radians()) + right_z as f32 * cos(yaw.to_radians()) > 0.0 {
            (-right_x, -right_z)
        } else {
            (right_x, right_z)
        };

    // AbstractBedBlock.bedSurroundStandUpOffsets, in target-version order.
    let surround_offsets = [
        (side_x, side_z),
        (side_x - forward_x, side_z - forward_z),
        (side_x - forward_x * 2, side_z - forward_z * 2),
        (-forward_x * 2, -forward_z * 2),
        (-side_x - forward_x * 2, -side_z - forward_z * 2),
        (-side_x - forward_x, -side_z - forward_z),
        (-side_x, -side_z),
        (-side_x + forward_x, -side_z + forward_z),
        (forward_x, forward_z),
        (side_x + forward_x, side_z + forward_z),
    ];
    let above_offsets = [(0, 0), (-forward_x, -forward_z)];
    let bunk_bed = world
        .get_block(&bed_pos.down())
        .has_tag(&tag::Block::MINECRAFT_BEDS);

    // AbstractBedBlock tries every safe candidate before its permissive pass.
    for check_dangerous in [true, false] {
        if let Some(position) =
            find_stand_up_position_at_offset(world, bed_pos, &surround_offsets, check_dangerous)
        {
            return Some(position);
        }
        if bunk_bed
            && let Some(position) = find_stand_up_position_at_offset(
                world,
                bed_pos.down(),
                &surround_offsets,
                check_dangerous,
            )
        {
            return Some(position);
        }
        if let Some(position) =
            find_stand_up_position_at_offset(world, bed_pos, &above_offsets, check_dangerous)
        {
            return Some(position);
        }
    }

    None
}

fn find_stand_up_position_at_offset(
    world: &World,
    pos: BlockPos,
    offsets: &[(i32, i32)],
    check_dangerous: bool,
) -> Option<Vector3<f64>> {
    offsets.iter().find_map(|&(offset_x, offset_z)| {
        stand_up::safe_dismount(world, pos.add(offset_x, 0, offset_z), check_dangerous)
    })
}
