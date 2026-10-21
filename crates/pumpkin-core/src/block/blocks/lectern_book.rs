//! LecternBlock.placeBook commits payment and the observed terrain before publication.

use super::{LecternBlock, LecternBlockEntity, LecternLikeProperties, UseWithItemArgs};
use pumpkin_data::BlockStateId;
use pumpkin_inventory::{Inventory, inventory::slot_transaction, screen_handler::InventoryPlayer};

pub(super) fn place_book(args: &mut UseWithItemArgs<'_>, observed: BlockStateId) -> bool {
    if args.item_stack.is_empty() {
        return false;
    }
    let Some(block_entity) = args.world.get_block_entity(args.position) else {
        return false;
    };
    if !args.world.is_live_block_entity(&block_entity) {
        return false;
    }
    let Some(lectern) = block_entity.as_any().downcast_ref::<LecternBlockEntity>() else {
        return false;
    };
    let inventory = args.player.inventory();
    let Some(mutation) = args.world.block_entity_mutation_permit(*args.position) else {
        return false;
    };
    let owners = slot_transaction::ordered_owners(&[inventory.as_ref() as &dyn Inventory, lectern]);
    let guards = slot_transaction::lock_owners(&owners);
    if !owners.iter().all(|owner| owner.is_live()) || !lectern.get_stack(0).is_empty() {
        return false;
    }
    let Some(source) = (0..inventory.size()).find(|slot| {
        let current = inventory.get_stack(*slot);
        current.uid == args.item_stack.uid && current.are_equal(args.item_stack)
    }) else {
        return false;
    };
    let mut properties = LecternLikeProperties::from_state_id(observed);
    properties.powered = false;
    properties.has_book = true;
    let Some(effects) = args.world.prepare_block_state_from_observed(
        *args.position,
        observed,
        properties.to_state_id(args.block),
    ) else {
        return false;
    };
    let mut receipt = if args.player.has_infinite_materials() {
        args.item_stack.copy_with_count(1)
    } else {
        slot_transaction::withdraw(inventory.as_ref(), source, args.item_stack, 1)
    };
    slot_transaction::deposit(lectern, 0, &mut receipt, 1);
    debug_assert!(receipt.is_empty());
    *args.item_stack = inventory.get_stack(source);
    drop(guards);
    drop(mutation);
    lectern.on_slot_changed(0);
    args.player
        .inventory_changed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    args.player.sync_hand_slot(source, args.item_stack.clone());
    effects();
    LecternBlock::update_neighbors_below(args.world, args.position);
    true
}
