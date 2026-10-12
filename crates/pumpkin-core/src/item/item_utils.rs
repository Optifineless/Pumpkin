use crate::entity::player::Player;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;
use pumpkin_inventory::screen_handler::InventoryPlayer;

/// Gives extra items to the player, dropping anything their inventory cannot hold.
pub fn give_or_drop(player: &Player, mut stack: ItemStack) {
    player.inventory().insert_stack_anywhere(&mut stack);
    if !stack.is_empty() {
        player
            .world()
            .drop_stack(&player.position().to_block_pos(), stack);
    }
}

/// Commits a held item into an empty container slot before publishing inventory changes.
// LecternBlock.placeBook / ChiseledBookShelfBlock.addBook / JukeboxBlock.useItemOn consume one.
pub fn place_held_item(
    input: &mut ItemStack,
    player: &Player,
    destination: &dyn Inventory,
    slot: usize,
) -> bool {
    transfer_held_item(input, player, destination, slot, true)
}

/// Commits one held item into a compatible container slot before publishing changes.
// DecoratedPotBlock.useItemOn permits merging identical item components.
pub fn insert_held_item(
    input: &mut ItemStack,
    player: &Player,
    destination: &dyn Inventory,
    slot: usize,
) -> bool {
    transfer_held_item(input, player, destination, slot, false)
}

fn transfer_held_item(
    input: &mut ItemStack,
    player: &Player,
    destination: &dyn Inventory,
    slot: usize,
    empty_only: bool,
) -> bool {
    let inventory = player.inventory();
    let owners = pumpkin_inventory::inventory::slot_transaction::ordered_owners(&[
        inventory.as_ref() as &dyn Inventory,
        destination,
    ]);
    let guards = pumpkin_inventory::inventory::slot_transaction::lock_owners(&owners);
    if !owners.iter().all(|owner| owner.is_live()) {
        return false;
    }
    let Some(source) = (0..inventory.size()).find(|index| {
        let current = inventory.get_stack(*index);
        current.uid == input.uid && current.are_equal(input)
    }) else {
        return false;
    };
    let target = destination.get_stack(slot);
    let maximum = input
        .get_max_stack_size()
        .min(destination.get_max_count_per_stack());
    if input.is_empty()
        || (!target.is_empty()
            && (empty_only
                || !target.are_items_and_components_equal(input)
                || target.item_count >= maximum))
    {
        return false;
    }
    let mut receipt = if player.has_infinite_materials() {
        input.copy_with_count(1)
    } else {
        pumpkin_inventory::inventory::slot_transaction::withdraw(
            inventory.as_ref(),
            source,
            input,
            1,
        )
    };
    pumpkin_inventory::inventory::slot_transaction::deposit(
        destination,
        slot,
        &mut receipt,
        maximum,
    );
    debug_assert!(receipt.is_empty());
    *input = inventory.get_stack(source);
    drop(guards);
    destination.on_slot_changed(slot);
    player
        .inventory_changed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    player.sync_hand_slot(source, input.clone());
    true
}

/// Commits one filled result and its source consumption before publishing drops or packets.
// ItemUtils.createFilledResult: a remaining input sends the output to Inventory.add.
pub fn create_filled_result(
    input: &mut ItemStack,
    player: &Player,
    mut output: ItemStack,
    limit_creative_stack_size: bool,
) -> bool {
    let inventory = player.inventory();
    let owner = inventory.slot_transaction(0);
    let guard = owner.lock();
    let Some(source) = (0..inventory.size()).find(|slot| {
        let current = inventory.get_stack(*slot);
        current.uid == input.uid && current.are_equal(input)
    }) else {
        return false;
    };
    if !owner.is_live() {
        return false;
    }
    if limit_creative_stack_size && player.has_infinite_materials() {
        if !(0..inventory.size()).any(|slot| {
            inventory
                .get_stack(slot)
                .are_items_and_components_equal(&output)
        }) {
            inventory.insert_stack_anywhere(&mut output);
        }
        drop(guard);
        return true;
    }
    let expected = input.clone();
    input.decrement_unless_creative(player.gamemode.load(), 1);
    if input.is_empty() {
        *input = output;
        output = ItemStack::EMPTY.clone();
    }
    let committed = pumpkin_inventory::inventory::slot_transaction::exchange(
        inventory.as_ref(),
        source,
        &expected,
        input.clone(),
    );
    debug_assert!(committed);
    if !output.is_empty() {
        inventory.insert_stack_anywhere(&mut output);
    }
    *input = inventory.get_stack(source);
    drop(guard);
    player
        .inventory_changed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    player.sync_hand_slot(source, input.clone());
    if !output.is_empty() {
        player
            .world()
            .drop_stack(&player.position().to_block_pos(), output);
    }
    true
}
