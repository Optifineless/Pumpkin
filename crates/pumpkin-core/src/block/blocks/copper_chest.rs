use crate::block::GetStateForNeighborUpdateArgs;
use pumpkin_data::HorizontalFacingExt;
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{ChestLikeProperties, ChestType},
    tag::{self, Taggable},
};

/// Preserves the complete block entity across copper-chest variants.
// CopperChestBlock.shouldChangedStateKeepBlockEntity.
pub fn should_changed_state_keep_block_entity(old: &Block, new: &Block) -> bool {
    old.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
        && new.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
}

// CopperChestBlock.updateShape first applies ChestBlock.updateShape, then follows its partner.
pub(super) fn update_shape(args: &GetStateForNeighborUpdateArgs<'_>) -> BlockStateId {
    let neighbor = Block::from_state_id(args.neighbor_state_id);
    let mut props = ChestLikeProperties::from_state_id(args.state_id);
    if props.waterlogged {
        args.world.schedule_fluid_tick(
            &pumpkin_data::fluid::Fluid::FLOWING_WATER,
            *args.position,
            crate::block::fluid::water::WATER_FLOW_SPEED,
            pumpkin_world::tick::TickPriority::Normal,
        );
    }
    let direction = |props: &ChestLikeProperties| match props.r#type {
        ChestType::Left => props.facing.rotate_clockwise().to_block_direction(),
        ChestType::Single | ChestType::Right => {
            props.facing.rotate_counter_clockwise().to_block_direction()
        }
    };
    if neighbor.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS) {
        let other = ChestLikeProperties::from_state_id(args.neighbor_state_id);
        if matches!(
            args.direction,
            BlockDirection::North
                | BlockDirection::South
                | BlockDirection::East
                | BlockDirection::West
        ) {
            if props.r#type == ChestType::Single
                && other.r#type != ChestType::Single
                && props.facing == other.facing
                && direction(&other) == args.direction.opposite()
            {
                props.r#type = if other.r#type == ChestType::Left {
                    ChestType::Right
                } else {
                    ChestType::Left
                };
            }
        } else if direction(&props) == args.direction {
            props.r#type = ChestType::Single;
        }
        if props.r#type != ChestType::Single && direction(&props) == args.direction {
            return props.to_state_id(neighbor);
        }
    } else if direction(&props) == args.direction {
        props.r#type = ChestType::Single;
    }
    props.to_state_id(args.block)
}
