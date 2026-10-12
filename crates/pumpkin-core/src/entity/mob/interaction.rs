use crate::{
    entity::player::Player,
    net::java::play::hand_use_result::{HandMutation, hand_slot, try_write_back_hand_item},
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
    pub(crate) fn new(player: &Player, input: &ItemStack, hand: Hand) -> Self {
        Self {
            hand,
            source_slot: hand_slot(player, hand),
            before: input.clone(),
        }
    }

    /// Consumes one item through vanilla's `Mob.usePlayerItem` using this interaction's snapshot.
    pub fn use_player_item(&mut self, player: &Player, input: &mut ItemStack) -> bool {
        let committed = crate::item::item_utils::use_player_item(player, input, Some(self));
        if committed {
            self.before = input.clone();
        }
        committed
    }

    /// Persists the interaction result to its captured hand if no callback replaced it.
    pub(crate) fn finish(&self, player: &Player, input: &ItemStack) -> bool {
        try_write_back_hand_item(
            player,
            self.hand,
            self.source_slot,
            &self.before,
            input,
            HandMutation::ItemUse,
        )
    }

    /// Returns the hand Minecraft used to begin this interaction.
    #[must_use]
    pub const fn hand(&self) -> Hand {
        self.hand
    }

    pub(crate) const fn source_slot(&self) -> usize {
        self.source_slot
    }

    pub(crate) const fn before(&self) -> &ItemStack {
        &self.before
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
