use crate::entity::player::Player;
use crate::net::java::play::hand_use_result::{HandMutation, hand_slot, write_back_hand_item};
use pumpkin_data::data_component_impl::UseRemainderImpl;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::Hand;

/// Gives extra items to the player, dropping Survival overflow and discarding Creative overflow.
pub fn give_or_drop(player: &Player, mut stack: ItemStack) {
    player.inventory().insert_stack_anywhere(&mut stack);
    // Inventory.add discards remaining items and reports success for infinite materials.
    if !stack.is_empty() && !player.has_infinite_materials() {
        player
            .world()
            .drop_stack(&player.position().to_block_pos(), stack);
    }
}

/// Consumes mob food and applies `USE_REMAINDER` before later interaction callbacks.
/// Handless species keep their existing detached consumption until they carry the used hand.
pub(crate) fn use_player_item(player: &Player, input: &mut ItemStack, hand: Option<Hand>) {
    let Some(hand) = hand else {
        input.decrement_unless_creative(player.gamemode.load(), 1);
        return;
    };
    // Mob.usePlayerItem snapshots the component before consuming; it does not apply USE_COOLDOWN.
    let before = input.clone();
    let source_slot = hand_slot(player, hand);
    input.decrement_unless_creative(player.gamemode.load(), 1);
    if !player.has_infinite_materials()
        && input.item_count < before.item_count
        && let Some(extra) = before
            .get_data_component::<UseRemainderImpl>()
            .and_then(UseRemainderImpl::create)
    {
        if input.is_empty() {
            *input = extra;
        } else {
            // Insertion can merge into the consumed stack, so publish it first.
            write_back_hand_item(
                player,
                hand,
                source_slot,
                &before,
                input,
                HandMutation::ItemUse,
            );
            give_or_drop(player, extra);
            *input = player.inventory().get_stack(source_slot);
            player.sync_hand_slot(source_slot, input.clone());
        }
    }
    write_back_hand_item(
        player,
        hand,
        source_slot,
        &before,
        input,
        HandMutation::ItemUse,
    );
}

/// Converts one input item into an output without writing the source hand.
/// The caller persists `input`; creative duplicate suppression matches ItemUtils.createFilledResult.
pub fn create_filled_result(
    input: &mut ItemStack,
    player: &Player,
    output: ItemStack,
    limit_creative_stack_size: bool,
) {
    if limit_creative_stack_size && player.has_infinite_materials() {
        let inventory = player.inventory();
        if !(0..inventory.size()).any(|slot| {
            inventory
                .get_stack(slot)
                .are_items_and_components_equal(&output)
        }) {
            let mut output = output;
            inventory.insert_stack_anywhere(&mut output);
        }
        return;
    }
    input.decrement_unless_creative(player.gamemode.load(), 1);
    if input.is_empty() {
        *input = output;
    } else {
        give_or_drop(player, output);
    }
}
