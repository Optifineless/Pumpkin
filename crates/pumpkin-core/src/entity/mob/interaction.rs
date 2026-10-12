use crate::{
    entity::player::Player,
    net::java::play::hand_use_result::{HandMutation, hand_slot, write_back_hand_item},
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::Hand;

/// Mirrors vanilla `Mob.interact` by retaining the source hand while its item clone is mutated.
pub struct MobInteraction {
    hand: Hand,
    source_slot: usize,
    before: ItemStack,
}

impl MobInteraction {
    /// Captures the originating slot before a species interaction can call plugins.
    pub fn new(player: &Player, input: &ItemStack, hand: Hand) -> Self {
        Self {
            hand,
            source_slot: hand_slot(player, hand),
            before: input.clone(),
        }
    }

    /// Persists the interaction result to its captured hand if no callback replaced it.
    pub fn finish(&self, player: &Player, input: &ItemStack) -> bool {
        write_back_hand_item(
            player,
            self.hand,
            self.source_slot,
            &self.before,
            input,
            HandMutation::ItemUse,
        );
        true
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
