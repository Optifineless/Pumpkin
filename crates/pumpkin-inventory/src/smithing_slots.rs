use crate::{
    Inventory,
    slot::{NormalSlot, Slot},
};
use pumpkin_data::{
    item_stack::ItemStack,
    recipes::{RECIPES_SMITHING_TRANSFORM, RECIPES_SMITHING_TRIM},
};
use std::sync::Arc;

pub struct SmithingInputSlot(NormalSlot);
impl SmithingInputSlot {
    pub(crate) fn new(inventory: Arc<dyn Inventory>, index: usize) -> Self {
        Self(NormalSlot::new(inventory, index))
    }
}
impl Slot for SmithingInputSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.0.get_inventory()
    }
    fn get_index(&self) -> usize {
        self.0.get_index()
    }
    fn set_id(&self, index: usize) {
        self.0.set_id(index);
    }
    fn mark_dirty(&self) {
        self.0.mark_dirty();
    }
    fn can_insert(&self, stack: &ItemStack) -> bool {
        // SmithingMenu.createInputSlotDefinitions reads RecipePropertySet's ingredient unions.
        RECIPES_SMITHING_TRIM
            .iter()
            .any(|r| [&r.template, &r.base, &r.addition][self.get_index()].match_item(stack.item))
            || RECIPES_SMITHING_TRANSFORM.iter().any(|r| {
                [&r.template, &r.base, &r.addition][self.get_index()].match_item(stack.item)
            })
    }
}
