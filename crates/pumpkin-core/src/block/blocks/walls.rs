use crate::block::{
    BlockBehaviour, GetStateForNeighborUpdateArgs, OnPlaceArgs, PathComputationType,
};
use crate::world::World;
use pumpkin_data::block_properties::EastWall;
use pumpkin_data::block_properties::HorizontalFacing;
use pumpkin_data::block_properties::NorthWall;
use pumpkin_data::block_properties::SouthWall;
use pumpkin_data::block_properties::WestWall;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, tag};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;

type FenceGateProperties = pumpkin_data::block_properties::OakFenceGateLikeProperties;
type WallProperties = pumpkin_data::block_properties::ResinBrickWallLikeProperties;

#[pumpkin_block_from_tag("minecraft:walls")]
pub struct WallBlock;

impl BlockBehaviour for WallBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut wall_props = WallProperties::default(args.block);
        wall_props.waterlogged = args.replacing.water_source();

        compute_wall_state(wall_props, args.world, args.block, args.position)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let wall_props = WallProperties::from_state_id(args.state_id);
        compute_wall_state(wall_props, args.world, args.block, args.position)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

pub fn compute_wall_state(
    mut wall_props: WallProperties,
    world: &World,
    block: &Block,
    block_pos: &BlockPos,
) -> BlockStateId {
    let block_above_pos = block_pos.up();
    let (block_above, block_above_state) = world.get_block_and_state(&block_above_pos);

    for direction in HorizontalFacing::all() {
        let other_block_pos = block_pos.offset(direction.to_offset());
        let (other_block, other_block_state) = world.get_block_and_state(&other_block_pos);

        let connected = is_connected(block, direction, other_block, other_block_state);

        let shape = if connected {
            let raise = is_wall_side_covered(block_above_state, &block_above_pos, direction);
            if raise {
                WallShape::Tall
            } else {
                WallShape::Low
            }
        } else {
            WallShape::None
        };

        match direction {
            HorizontalFacing::North => wall_props.north = shape.into(),
            HorizontalFacing::South => wall_props.south = shape.into(),
            HorizontalFacing::East => wall_props.east = shape.into(),
            HorizontalFacing::West => wall_props.west = shape.into(),
        }
    }

    wall_props.up = should_raise_post(wall_props, block_above, block_above_state, &block_above_pos);
    wall_props.to_state_id(block)
}

fn is_connected(
    block: &Block,
    direction: HorizontalFacing,
    other_block: &Block,
    other_block_state: &BlockState,
) -> bool {
    let solid_face_connection = !super::fences::is_exception_for_connection(other_block)
        && ((other_block_state.is_solid() && other_block_state.is_full_cube())
            || other_block_state.is_side_solid(BlockDirection::from_cardinal_direction(
                direction.opposite(),
            )));
    let mut connected = other_block == block
        || solid_face_connection
        || other_block.has_tag(&tag::Block::MINECRAFT_WALLS)
        || other_block == &Block::IRON_BARS
        || other_block.has_tag(&tag::Block::C_GLASS_PANES);

    // fence gates do not pass is_side_solid check
    if !connected && other_block.has_tag(&tag::Block::MINECRAFT_FENCE_GATES) {
        let fence_props = FenceGateProperties::from_state_id(other_block_state.id);
        if fence_props.facing == direction.rotate_clockwise()
            || fence_props.facing == direction.rotate_counter_clockwise()
        {
            connected = true;
        }
    }
    connected
}

/// Mirrors `WallBlock.updateSides` and `WallBlock.isCovered` using collision faces.
fn is_wall_side_covered(
    block_above_state: &BlockState,
    block_above_pos: &BlockPos,
    direction: HorizontalFacing,
) -> bool {
    // WallBlock.updateSides tests the top footprint of each tall side arm.
    let side_test = match direction {
        HorizontalFacing::North => NORTH_SIDE_TEST_SHAPE,
        HorizontalFacing::South => SOUTH_SIDE_TEST_SHAPE,
        HorizontalFacing::East => EAST_SIDE_TEST_SHAPE,
        HorizontalFacing::West => WEST_SIDE_TEST_SHAPE,
    };
    collision_face_covers(block_above_state, block_above_pos, &side_test)
}

const NORTH_SIDE_TEST_SHAPE: BoundingBox =
    BoundingBox::new_array([7.0 / 16.0, 0.0, 0.0], [9.0 / 16.0, 1.0, 9.0 / 16.0]);
const SOUTH_SIDE_TEST_SHAPE: BoundingBox =
    BoundingBox::new_array([7.0 / 16.0, 0.0, 7.0 / 16.0], [9.0 / 16.0, 1.0, 1.0]);
const EAST_SIDE_TEST_SHAPE: BoundingBox =
    BoundingBox::new_array([7.0 / 16.0, 0.0, 7.0 / 16.0], [1.0, 1.0, 9.0 / 16.0]);
const WEST_SIDE_TEST_SHAPE: BoundingBox =
    BoundingBox::new_array([0.0, 0.0, 7.0 / 16.0], [9.0 / 16.0, 1.0, 9.0 / 16.0]);
const POST_TEST_SHAPE: BoundingBox =
    BoundingBox::new_array([7.0 / 16.0, 0.0, 7.0 / 16.0], [9.0 / 16.0, 1.0, 9.0 / 16.0]);

fn should_raise_post(
    wall_props: WallProperties,
    block_above: &Block,
    block_above_state: &BlockState,
    block_above_pos: &BlockPos,
) -> bool {
    if block_above.has_tag(&tag::Block::MINECRAFT_WALLS)
        && WallProperties::from_state_id(block_above_state.id).up
    {
        return true;
    }

    let north_none = wall_props.north == NorthWall::None;
    let south_none = wall_props.south == SouthWall::None;
    let east_none = wall_props.east == EastWall::None;
    let west_none = wall_props.west == WestWall::None;
    let has_corner = north_none && south_none && east_none && west_none
        || north_none != south_none
        || east_none != west_none;
    if has_corner {
        return true;
    }

    let has_high_wall = wall_props.north == NorthWall::Tall && wall_props.south == SouthWall::Tall
        || wall_props.east == EastWall::Tall && wall_props.west == WestWall::Tall;
    if has_high_wall {
        return false;
    }

    block_above.has_tag(&tag::Block::MINECRAFT_WALL_POST_OVERRIDE)
        || collision_face_covers(block_above_state, block_above_pos, &POST_TEST_SHAPE)
}

fn collision_face_covers(
    covering_state: &BlockState,
    covering_pos: &BlockPos,
    test_shape: &BoundingBox,
) -> bool {
    let downward_face: Vec<_> = covering_state
        .get_block_collision_shapes_at(covering_pos)
        .filter(|shape| shape.min.y == 0.0 && shape.max.y > 0.0)
        .collect();
    rectangle_is_covered(test_shape, &downward_face)
}

fn rectangle_is_covered(test: &BoundingBox, covers: &[BoundingBox]) -> bool {
    let mut x_edges = vec![test.min.x, test.max.x];
    let mut z_edges = vec![test.min.z, test.max.z];
    for cover in covers {
        let min_x = cover.min.x.max(test.min.x);
        let max_x = cover.max.x.min(test.max.x);
        let min_z = cover.min.z.max(test.min.z);
        let max_z = cover.max.z.min(test.max.z);
        if min_x < max_x && min_z < max_z {
            x_edges.extend([min_x, max_x]);
            z_edges.extend([min_z, max_z]);
        }
    }
    x_edges.sort_by(f64::total_cmp);
    x_edges.dedup_by(|left, right| left.total_cmp(right).is_eq());
    z_edges.sort_by(f64::total_cmp);
    z_edges.dedup_by(|left, right| left.total_cmp(right).is_eq());

    x_edges.windows(2).all(|x| {
        z_edges.windows(2).all(|z| {
            let x = f64::midpoint(x[0], x[1]);
            let z = f64::midpoint(z[0], z[1]);
            covers.iter().any(|cover| {
                cover.min.x < x && x < cover.max.x && cover.min.z < z && z < cover.max.z
            })
        })
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WallShape {
    None,
    Low,
    Tall,
}

impl From<WallShape> for NorthWall {
    fn from(value: WallShape) -> Self {
        match value {
            WallShape::None => Self::None,
            WallShape::Low => Self::Low,
            WallShape::Tall => Self::Tall,
        }
    }
}

impl From<WallShape> for SouthWall {
    fn from(value: WallShape) -> Self {
        match value {
            WallShape::None => Self::None,
            WallShape::Low => Self::Low,
            WallShape::Tall => Self::Tall,
        }
    }
}

impl From<WallShape> for EastWall {
    fn from(value: WallShape) -> Self {
        match value {
            WallShape::None => Self::None,
            WallShape::Low => Self::Low,
            WallShape::Tall => Self::Tall,
        }
    }
}

impl From<WallShape> for WestWall {
    fn from(value: WallShape) -> Self {
        match value {
            WallShape::None => Self::None,
            WallShape::Low => Self::Low,
            WallShape::Tall => Self::Tall,
        }
    }
}
