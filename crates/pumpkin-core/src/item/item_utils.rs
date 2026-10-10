use crate::entity::player::Player;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;
use pumpkin_inventory::screen_handler::InventoryPlayer;

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
