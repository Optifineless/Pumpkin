use super::{BlockEntity, ChestBlockEntity, ChestTypeExt, World};
use crate::block::entities::trapped_chest::TrappedChestBlockEntity;
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{BlockProperties, ChestLikeProperties, ChestType},
    block_transformer::AXE,
    tag::{self, Taggable},
    world::WorldEvent,
};
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

// CopperChestBlock.chestCanConnectTo broadens placement candidates, not inventory composition.
pub(super) fn chest_can_connect_to(block: &Block, neighbor: &Block) -> bool {
    block == neighbor
        || (block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
            && neighbor.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
            && ChestLikeProperties::handles_block_id(neighbor.id))
}

fn unwax_block(block: &Block) -> &Block {
    // Resolve HoneycombItem's reverse mapping from the generated wax-off transform.
    AXE.transform(
        block,
        block.default_state.id,
        BlockDirection::Up,
        &|_, _, _| block.id.to_block(),
    )
    .filter(|result| result.entry.particle == Some(WorldEvent::ParticlesWaxOff))
    .map_or(block, |result| result.target_block)
}

// CopperChestBlock.getStateForPlacement -> getLeastOxidizedChestOfConnectedBlocks.
pub(super) fn get_least_oxidized_chest_of_connected_blocks(
    state: BlockStateId,
    world: &World,
    position: &BlockPos,
) -> BlockStateId {
    let block = Block::from_state_id(state);
    if !block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS) {
        return state;
    }
    let properties = ChestLikeProperties::from_state_id(state);
    let direction = match properties.r#type {
        ChestType::Single => return state,
        ChestType::Left => properties.facing.rotate_clockwise(),
        ChestType::Right => properties.facing.rotate_counter_clockwise(),
    };
    let neighbor = world.get_block(&position.offset(direction.to_offset()));
    if !chest_can_connect_to(block, neighbor) {
        return state;
    }
    let unwaxed = unwax_block(block);
    let neighbor_unwaxed = unwax_block(neighbor);
    let (Some(age), Some(neighbor_age)) = (
        crate::block::blocks::weathering_copper::get_weather_state(unwaxed),
        crate::block::blocks::weathering_copper::get_weather_state(neighbor_unwaxed),
    ) else {
        return state;
    };
    let mixed_wax = (unwaxed != block) != (neighbor_unwaxed != neighbor);
    let target = match (age <= neighbor_age, mixed_wax) {
        (true, false) => block,
        (false, false) => neighbor,
        (true, true) => unwaxed,
        (false, true) => neighbor_unwaxed,
    };
    properties.to_state_id(target)
}

// DoubleBlockCombiner.combineWithNeigbour validates the same pair for every reader.
pub(super) fn valid_partner(
    world: &World,
    position: &BlockPos,
    state: BlockStateId,
    entity: &Arc<dyn BlockEntity>,
) -> Option<(BlockPos, Arc<dyn BlockEntity>)> {
    if !entity.as_any().is::<ChestBlockEntity>() && !entity.as_any().is::<TrappedChestBlockEntity>()
    {
        return None;
    }
    let block = Block::from_state_id(state);
    if !ChestLikeProperties::handles_block_id(block.id) {
        return None;
    }
    let properties = ChestLikeProperties::from_state_id(state);
    let direction = match properties.r#type {
        ChestType::Single => return None,
        ChestType::Left => properties.facing.rotate_clockwise(),
        ChestType::Right => properties.facing.rotate_counter_clockwise(),
    };
    let neighbor_pos = position.offset(direction.to_offset());
    let (neighbor, neighbor_state) = world.get_block_and_state_id(&neighbor_pos);
    if neighbor != block {
        return None;
    }
    let other = ChestLikeProperties::from_state_id(neighbor_state);
    if other.facing != properties.facing || other.r#type != properties.r#type.opposite() {
        return None;
    }
    let neighbor_entity = world.get_block_entity(&neighbor_pos)?;
    if neighbor_entity.resource_location() != entity.resource_location() {
        return None;
    }
    Some((neighbor_pos, neighbor_entity))
}
