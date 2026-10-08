use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::RecipesImpl;
use pumpkin_data::item::Item;

pub struct KnowledgeBookItem;
pub struct DiscFragmentItem;

impl ItemMetadata for KnowledgeBookItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::KNOWLEDGE_BOOK.id])
    }
}

impl ItemBehaviour for KnowledgeBookItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    // KnowledgeBookItem.use reads and consumes player.getItemInHand(hand).
    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
        let mut stack = player.inventory().get_stack_in_hand(hand);
        let _recipes = stack.get_data_component::<RecipesImpl>();
        stack.decrement_unless_creative(player.gamemode.load(), 1);
        player.inventory().set_stack_in_hand(hand, stack);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ItemMetadata for DiscFragmentItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::DISC_FRAGMENT_5.id])
    }
}

impl ItemBehaviour for DiscFragmentItem {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
