use std::sync::Arc;

use pumpkin_data::block_properties::{BlockProperties, ChestLikeProperties, ChestType};
use pumpkin_inventory::{Inventory, double::DoubleInventory};
use pumpkin_util::math::position::BlockPos;

use super::{chest::ChestBlockEntity, trapped_chest::TrappedChestBlockEntity};
use crate::world::World;

// HopperBlockEntity.getBlockContainer uses ChestBlock.getContainer for both transfers.
pub(super) fn get_container_at(world: &World, pos: &BlockPos) -> Option<Arc<dyn Inventory>> {
    let entity = world.get_block_entity(pos)?;
    let inventory = entity.clone().get_inventory()?;
    // HopperBlockEntity.getBlockContainer only asks ChestBlock for chest entities.
    if !entity.as_any().is::<ChestBlockEntity>() && !entity.as_any().is::<TrappedChestBlockEntity>()
    {
        return Some(inventory);
    }
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
    // DoubleBlockCombiner.combineWithNeigbour only combines matching, opposite halves.
    let (partner_block, partner_state) = world.get_block_and_state(&partner);
    if partner_block != block {
        return Some(inventory);
    }
    let partner_props = ChestLikeProperties::from_state_id(partner_state.id);
    if partner_props.facing != props.facing
        || partner_props.r#type == ChestType::Single
        || partner_props.r#type == props.r#type
    {
        return Some(inventory);
    }
    let Some(partner_entity) = world.get_block_entity(&partner) else {
        return Some(inventory);
    };
    if partner_entity.resource_location() != entity.resource_location() {
        return Some(inventory);
    }
    let Some(other) = partner_entity.get_inventory() else {
        return Some(inventory);
    };
    // DoubleBlockCombiner orders RIGHT/FIRST before LEFT/SECOND.
    Some(if props.r#type == ChestType::Right {
        DoubleInventory::new(inventory, other)
    } else {
        DoubleInventory::new(other, inventory)
    })
}

// HopperBlockEntity.tryTakeInItemFromSlot/tryMoveInItem operate on the actual container.
// The closure only mutates the stack; callers must send packets and fire events afterwards.
pub(super) fn with_inventory_slot<T>(
    inventory: &dyn Inventory,
    slot: usize,
    update: impl FnOnce(&mut pumpkin_data::item_stack::ItemStack) -> T,
) -> T {
    if let Some(double) = inventory.as_any().downcast_ref::<DoubleInventory>() {
        let (half, slot) = double.inventory_for_slot(slot);
        return with_inventory_slot(half, slot, update);
    }
    if let Some(chest) = inventory.as_any().downcast_ref::<ChestBlockEntity>() {
        let mut items = chest
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return update(&mut items[slot]);
    }
    if let Some(chest) = inventory.as_any().downcast_ref::<TrappedChestBlockEntity>() {
        let mut items = chest
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return update(&mut items[slot]);
    }
    if let Some(hopper) = inventory
        .as_any()
        .downcast_ref::<super::hopper::HopperBlockEntity>()
    {
        let mut items = hopper
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return update(&mut items[slot]);
    }
    let mut stack = inventory.get_stack(slot);
    let before = stack.clone();
    let result = update(&mut stack);
    if !stack.are_equal(&before) {
        inventory.set_stack(slot, stack);
    }
    result
}
