use super::{
    JavaClient,
    hand_use_result::{hand_slot, trigger_item_used_on_block},
};
use crate::{
    entity::player::Player,
    plugin::api::events::player::player_item_consume::PlayerItemConsumeEvent, server::Server,
};
use pumpkin_data::{
    BlockStateId,
    data_component_impl::{ConsumableImpl, FoodImpl},
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

impl JavaClient {
    // Consumable.startConsuming shares the air-use hunger and active-use admission.
    pub(super) fn start_consuming(player: &Arc<Player>, hand: Hand, stack: &ItemStack) -> bool {
        if stack
            .get_data_component::<FoodImpl>()
            .is_some_and(|food| !player.can_eat(food.can_always_eat))
        {
            return false;
        }
        if stack.get_max_use_time() == 0 {
            player
                .living_entity
                .consume_instantly(player.as_ref(), hand, stack);
        } else {
            player
                .living_entity
                .set_active_hand(hand, stack.clone(), stack.get_max_use_time());
        }
        true
    }

    pub(super) fn consume_after_refused_placement(
        player: &Arc<Player>,
        server: &Arc<Server>,
        hand: Hand,
        source_slot: usize,
        stack: &ItemStack,
        position: BlockPos,
        state_before: BlockStateId,
    ) {
        if stack.get_data_component::<ConsumableImpl>().is_none()
            || !source_is_current(player, hand, source_slot, stack)
        {
            return;
        }
        let mut event =
            PlayerItemConsumeEvent::new(player.clone(), stack.item.registry_key.to_string());
        server.plugin_manager.fire_blocking(server, &mut event);
        if event.cancelled
            || !source_is_current(player, hand, source_slot, stack)
            || !crate::entity::item_use::item_use_allowed(stack, |group| {
                player.is_on_cooldown(group)
            })
        {
            return;
        }
        if !Self::start_consuming(player, hand, stack) {
            return;
        }
        trigger_item_used_on_block(player, position, stack, state_before);
        // Advancement and packet callbacks may replace the hand after admission.
        if !source_is_current(player, hand, source_slot, stack) {
            clear_admitted_use(player, hand, stack);
        }
    }
}

fn clear_admitted_use(player: &Player, hand: Hand, stack: &ItemStack) {
    let living = &player.living_entity;
    let active = *living
        .active_hand
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let saved = living
        .item_in_use
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    // Preserve a callback's newer use, as with the existing release-use snapshot.
    if active == Some(hand)
        && saved.is_some_and(|saved| saved.uid == stack.uid && saved.are_equal(stack))
        && living
            .item_use_time
            .load(std::sync::atomic::Ordering::Relaxed)
            <= stack.get_max_use_time()
    {
        living.clear_active_hand();
    }
}

fn source_is_current(player: &Player, hand: Hand, source_slot: usize, stack: &ItemStack) -> bool {
    let inventory = player.inventory();
    let owner = inventory.slot_transaction(source_slot);
    let _guard = owner.lock();
    let current = inventory.get_stack(source_slot);
    owner.is_live()
        && hand_slot(player, hand) == source_slot
        && !current.is_empty()
        && current.uid == stack.uid
        && current.are_equal(stack)
}

#[cfg(test)]
mod tests;
