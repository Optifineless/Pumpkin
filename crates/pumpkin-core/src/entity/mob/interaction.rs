use super::Mob;
use crate::{
    entity::player::Player,
    net::java::play::hand_use_result::{HandMutation, hand_slot, write_back_hand_item},
};
use pumpkin_data::{game_event::GameEvent, item_stack::ItemStack};
use pumpkin_util::Hand;

/// Retains the actual source hand while species interactions invoke synchronous callbacks.
pub struct MobInteraction {
    hand: Hand,
    source_slot: usize,
    before: ItemStack,
}

impl MobInteraction {
    pub fn new(player: &Player, input: &ItemStack, hand: Hand) -> Self {
        Self {
            hand,
            source_slot: hand_slot(player, hand),
            before: input.clone(),
        }
    }

    /// `Mob.interact` emits `ENTITY_INTERACT` after a successful species interaction.
    pub fn finish<M: Mob + ?Sized>(&self, mob: &M, player: &Player, input: &ItemStack) -> bool {
        write_back_hand_item(
            player,
            self.hand,
            self.source_slot,
            &self.before,
            input,
            HandMutation::ItemUse,
        );
        let entity = &mob.get_mob_entity().living_entity.entity;
        entity
            .world
            .load()
            .emit_game_event(GameEvent::EntityInteract.name(), entity.pos.load());
        true
    }
}
