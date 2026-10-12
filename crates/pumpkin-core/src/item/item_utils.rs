use crate::entity::mob::interaction::MobInteraction;
use crate::entity::player::Player;
use crate::net::java::play::hand_use_result::{
    HandMutation, try_commit_hand_slot, try_write_back_hand_item,
};
use pumpkin_data::{data_component_impl::UseRemainderImpl, item_stack::ItemStack};
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

/// Consumes mob food and applies `USE_REMAINDER` before later interaction callbacks.
/// Handless species keep detached consumption until their callback carries the interaction.
pub(crate) fn use_player_item(
    player: &Player,
    input: &mut ItemStack,
    interaction: Option<&MobInteraction>,
) -> bool {
    let Some(interaction) = interaction else {
        let before = input.clone();
        input.decrement_unless_creative(player.gamemode.load(), 1);
        return !input.are_equal(&before);
    };
    // Mob.usePlayerItem consumes from the hand captured by Mob.interact; it does not apply USE_COOLDOWN.
    let before_use = input.clone();
    let mut consumed = input.clone();
    consumed.decrement_unless_creative(player.gamemode.load(), 1);
    let extra = if !player.has_infinite_materials() && consumed.item_count < before_use.item_count {
        before_use
            .get_data_component::<UseRemainderImpl>()
            .and_then(UseRemainderImpl::create)
    } else {
        None
    };
    let after = if consumed.is_empty() {
        extra.as_ref().unwrap_or(&consumed)
    } else {
        &consumed
    };
    let give_extra = extra.is_some() && !consumed.is_empty();
    let committed = if give_extra {
        // Mob.usePlayerItem consumes before dispatching handleExtraItemsCreatedOnUse.
        try_commit_hand_slot(
            player,
            interaction.source_slot(),
            interaction.before(),
            after,
        )
    } else {
        try_write_back_hand_item(
            player,
            interaction.hand(),
            interaction.source_slot(),
            interaction.before(),
            after,
            HandMutation::ItemUse,
        )
    };
    if !committed {
        return false;
    }
    *input = after.clone();
    if let Some(extra) = extra.filter(|_| give_extra) {
        // Publish only after the remainder has been inserted or dropped.
        give_or_drop(player, extra);
        let committed_stack = player.inventory().get_stack(interaction.source_slot());
        player.sync_hand_slot(interaction.source_slot(), committed_stack);
    }
    true
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

#[cfg(test)]
#[path = "item_utils_tests.rs"]
mod tests;
