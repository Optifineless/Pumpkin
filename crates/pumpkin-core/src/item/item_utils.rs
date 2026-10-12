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

/// Claims one item from the observed hand before publishing a use effect.
/// Returns paid stock for cancellation refunds, or an empty receipt for Creative use.
// Throwable item use consumes the same getItemInHand snapshot in vanilla's serial tick.
pub(crate) fn claim_item_in_hand(
    player: &Player,
    hand: pumpkin_util::Hand,
    expected: &ItemStack,
) -> Option<ItemStack> {
    if expected.is_empty() {
        return None;
    }
    let mode = player.gamemode.load();
    let mut replacement = expected.clone();
    replacement.decrement_unless_creative(mode, 1);
    player
        .inventory()
        .exchange_stack_in_hand(hand, expected, replacement)
        .then(|| {
            if mode == pumpkin_util::GameMode::Creative {
                ItemStack::EMPTY.clone()
            } else {
                expected.copy_with_count(1)
            }
        })
}

/// Delivers the result of an admitted one-item hand claim without replacing concurrent edits.
// ItemUtils.createFilledResult replaces the last paid input, otherwise adds the output.
pub(crate) fn finish_claimed_result(
    player: &Player,
    hand: pumpkin_util::Hand,
    expected: &ItemStack,
    receipt: &ItemStack,
    output: ItemStack,
    creative_output: bool,
) {
    let inventory = player.inventory();
    if receipt.is_empty()
        && (!creative_output
            || (0..inventory.size()).any(|slot| {
                inventory
                    .get_stack(slot)
                    .are_items_and_components_equal(&output)
            }))
    {
        return;
    }
    let slot = match hand {
        pumpkin_util::Hand::Right => inventory.get_selected_slot() as usize,
        pumpkin_util::Hand::Left => {
            pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT
        }
    };
    let current = inventory.get_stack(slot);
    let delivered = !receipt.is_empty()
        && expected.item_count == 1
        && current.is_empty()
        && pumpkin_inventory::inventory::slot_transaction::exchange(
            inventory.as_ref(),
            slot,
            &current,
            output.clone(),
        );
    if !delivered {
        give_or_drop(player, output);
    }
    player
        .inventory_changed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    player.sync_hand_slot(slot, inventory.get_stack(slot));
}
