use std::sync::Arc;

use pumpkin_data::block_properties::{BlockProperties, ChestLikeProperties, ChestType};
use pumpkin_inventory::{Inventory, double::DoubleInventory};
use pumpkin_util::math::position::BlockPos;

use crate::world::World;

// HopperBlockEntity.getBlockContainer uses ChestBlock.getContainer for both transfers.
pub(super) fn get_container_at(world: &World, pos: &BlockPos) -> Option<Arc<dyn Inventory>> {
    let inventory = world.get_block_entity(pos)?.get_inventory()?;
    let (block, state) = world.get_block_and_state(pos);
    if !ChestLikeProperties::handles_block_id(block.id) {
        return Some(inventory);
    }
    let props = ChestLikeProperties::from_state_id(state.id);
    let direction = match props.r#type {
        ChestType::Single => return Some(inventory),
        ChestType::Left => props.facing.rotate_clockwise(),
        ChestType::Right => props.facing.rotate_counter_clockwise(),
    };
    let partner = pos.offset(direction.to_offset());
    let Some(other) = world.get_block_entity(&partner).and_then(|entity| entity.get_inventory()) else {
        return Some(inventory);
    };
    // DoubleBlockCombiner orders RIGHT/FIRST before LEFT/SECOND.
    Some(if props.r#type == ChestType::Right {
        DoubleInventory::new(inventory, other)
    } else {
        DoubleInventory::new(other, inventory)
    })
}
