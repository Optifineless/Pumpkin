use std::sync::Arc;

use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::CustomNameImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

pub struct NameTagItem;

impl ItemMetadata for NameTagItem {
    fn ids() -> Box<[u16]> {
        [Item::NAME_TAG.id].into()
    }
}

impl ItemBehaviour for NameTagItem {
    fn use_on_entity(&self, item: &mut ItemStack, player: &Player, entity: Arc<dyn EntityBase>) {
        let base = entity.get_entity();
        if base.entity_type.saveable
            && let Some(name) = item.get_data_component::<CustomNameImpl>()
        {
            // NameTagItem.interactLivingEntity makes a named mob persistent, including for caps.
            base.set_custom_name(name.name.clone());
            if let Some(mob) = entity.get_mob() {
                mob.get_mob_entity()
                    .persistence_required
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            item.decrement_unless_creative(player.gamemode.load(), 1);
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
