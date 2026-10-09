use crate::block::{BlockBehaviour, GetStateForNeighborUpdateArgs, OnPlaceArgs, PlacedArgs};
use crate::entity::EntityBase;
use pumpkin_data::block_properties::{WhiteBannerLikeProperties, WhiteWallBannerProperties};
use pumpkin_data::{Block, BlockDirection, BlockStateId, FacingExt, HorizontalFacingExt};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockAccessor;

use crate::block::entities::banner::BannerBlockEntity;
use std::sync::Arc;

#[pumpkin_block_from_tag("minecraft:banners")]
pub struct BannerBlock;

impl BlockBehaviour for BannerBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let entity = BannerBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(entity));
        }
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut directions = args
            .player
            .get_entity()
            .get_entity_facing_order()
            .map(|d| d.to_block_direction());
        // BlockPlaceContext.getNearestLookingDirections prioritizes the clicked support side.
        if args.position != &args.use_item_on.position
            && let Some(index) = directions.iter().position(|d| *d == args.direction)
        {
            directions[..=index].rotate_right(1);
        }
        let Some(direction) = select_support(directions, |direction| {
            has_support(args.world, args.position, direction)
        }) else {
            return BlockStateId::AIR;
        };
        if direction.is_horizontal() {
            let Some(color) = args
                .block
                .name
                .strip_suffix("_wall_banner")
                .or_else(|| args.block.name.strip_suffix("_banner"))
            else {
                return BlockStateId::AIR;
            };
            let Some(wall_block) = Block::from_name(&format!("{color}_wall_banner")) else {
                return BlockStateId::AIR;
            };
            let mut props = WhiteWallBannerProperties::default(wall_block);
            props.facing = direction.opposite().to_cardinal_direction();
            return props.to_state_id(wall_block);
        }
        let mut props = WhiteBannerLikeProperties::default(args.block);
        props.rotation = args.player.get_entity().get_flipped_rotation_16();
        props.to_state_id(args.block)
    }

    fn can_place_at(&self, args: crate::block::CanPlaceAtArgs<'_>) -> bool {
        // The registry validates the item before on_place selects its standing or wall state.
        if args.player.is_some() && args.use_item_on.is_some() {
            return has_support(args.block_accessor, args.position, BlockDirection::Down)
                || BlockDirection::horizontal().iter().any(|d| {
                    has_support(args.block_accessor, args.position, d.to_block_direction())
                });
        }
        has_support(
            args.block_accessor,
            args.position,
            support_direction(args.block, args.state.id),
        )
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        // BannerBlock/WallBannerBlock.updateShape reads current support in canSurvive.
        if args.direction == support_direction(args.block, args.state_id)
            && !has_support(args.world, args.position, args.direction)
        {
            BlockStateId::AIR
        } else {
            args.state_id
        }
    }
}

fn select_support(
    directions: [BlockDirection; 6],
    has_support: impl Fn(BlockDirection) -> bool,
) -> Option<BlockDirection> {
    // WallBannerBlock computes its first valid horizontal state before the item
    // chooses between that state and the standing banner in direction order.
    let wall = directions
        .iter()
        .copied()
        .find(|d| d.is_horizontal() && has_support(*d));
    for direction in directions {
        if direction == BlockDirection::Down {
            if has_support(direction) {
                return Some(direction);
            }
        } else if direction.is_horizontal() && wall.is_some() {
            return wall;
        }
    }
    None
}

fn support_direction(block: &Block, state_id: BlockStateId) -> BlockDirection {
    if block.name.ends_with("_wall_banner") {
        WhiteWallBannerProperties::from_state_id(state_id)
            .facing
            .to_block_direction()
            .opposite()
    } else {
        BlockDirection::Down
    }
}

fn has_support(world: &dyn BlockAccessor, position: &BlockPos, direction: BlockDirection) -> bool {
    let state = world.get_block_state(&position.offset(direction.to_offset()));
    state.is_solid()
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlockDirection::{Down, East, North, South, Up, West};

    #[test]
    fn floor_or_wall_follows_placement_order() {
        let supports = |d| matches!(d, Down | North);
        assert_eq!(
            select_support([Down, North, South, East, West, Up], supports),
            Some(Down)
        );
        assert_eq!(
            select_support([North, Down, South, East, West, Up], supports),
            Some(North)
        );
    }

    #[test]
    fn wall_candidate_is_computed_before_item_selects_variant() {
        // The first horizontal direction has no support, but Vanilla still uses
        // the wall state found later rather than the intervening floor candidate.
        assert_eq!(
            select_support([North, Down, South, East, West, Up], |d| matches!(
                d,
                Down | South
            )),
            Some(South)
        );
    }

    #[test]
    fn ceiling_is_ignored_and_missing_support_rejects_placement() {
        let directions = [Up, North, Down, South, East, West];
        assert_eq!(select_support(directions, |d| d == Up), None);
        assert_eq!(select_support(directions, |_| false), None);
        assert_eq!(select_support(directions, |d| d == Down), Some(Down));
    }
}

#[cfg(test)]
#[path = "banner_support_tests.rs"]
mod support_tests;
