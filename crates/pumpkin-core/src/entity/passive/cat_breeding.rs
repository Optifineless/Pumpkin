//! `Cat.getBreedOffspring` and the recipe-backed `DyeColor.getMixedColor` contract.

use super::CatEntity;
use crate::entity::{EntityBase, mob::Mob, passive::tamable::TamableAnimal};
use pumpkin_data::{data_component_impl::DyeImpl, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory,
    crafting::{
        crafting_inventory::CraftingInventory, crafting_screen_handler::match_crafting_recipe,
        recipe_provider::RecipeProvider,
    },
};
use rand::RngExt;
use std::sync::{LazyLock, atomic::Ordering::Relaxed};

// The item registry is generated and immutable. Build its component lookup once,
// including every item with a default DYE component, as vanilla's registry lookup does.
static DYE_ITEMS: LazyLock<Vec<(u8, &'static Item)>> = LazyLock::new(|| {
    (0..=u16::MAX)
        .filter_map(Item::from_id)
        .filter_map(|item| {
            ItemStack::static_new_java(1, item)
                .get_data_component::<DyeImpl>()
                .map(|dye| (dye.color.id(), item))
        })
        .collect()
});

impl CatEntity {
    pub(crate) fn inherit_breeding_data(&self, partner: &Self, offspring: &Self) {
        let source = if self.get_random().random() {
            self
        } else {
            partner
        };
        offspring.set_variant(source.variant.load(Relaxed));
        if self.is_tame() {
            offspring.set_tame(true, self.get_owner());
            offspring.set_collar_color(mixed_collar_color(self, partner));
        }
    }
}

fn mixed_collar_color(parent: &CatEntity, partner: &CatEntity) -> u8 {
    let first = parent.get_collar_color();
    let second = partner.get_collar_color();
    let world = parent.get_entity().world.load();
    let server = world.server.upgrade();
    let provider = server
        .as_ref()
        .map(|server| server.recipe_manager.as_ref() as &dyn RecipeProvider);
    let input = CraftingInventory::new(2, 1);
    for &(_, first_item) in DYE_ITEMS.iter().filter(|(color, _)| *color == first) {
        for &(_, second_item) in DYE_ITEMS.iter().filter(|(color, _)| *color == second) {
            input.set_stack(0, ItemStack::new(1, first_item));
            input.set_stack(1, ItemStack::new(1, second_item));
            if let Some(result) = match_crafting_recipe(&input, provider)
                && let Some(dye) = result.stack.get_data_component::<DyeImpl>()
            {
                return dye.color.id();
            }
        }
    }
    if parent.get_random().random() {
        first
    } else {
        second
    }
}
