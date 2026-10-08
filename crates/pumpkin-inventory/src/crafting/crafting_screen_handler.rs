//! Crafting screen handler implementation.
//!
//! This module provides screen handlers for crafting mechanics:
//! - [`CraftingScreenHandler`] - Trait for crafting screen handlers
//! - [`CraftingTableScreenHandler`] - The 3x3 crafting table UI
//! - [`ResultSlot`] - The special result slot that shows crafted items
//!
//! # Recipe Matching
//!
//! Crafting recipes are matched against the items in the crafting grid.
//! The system supports:
//! - Shaped recipes (specific patterns)
//! - Shapeless recipes (any arrangement)
//! - Transmute recipes (upgrading items)
//! - Special recipes (like decorated pots)

use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use super::recipe_matching::recipe_matches;
use super::recipe_provider::{GenericRecipe, RecipeProvider};
use super::recipes::{RecipeFinderScreenHandler, RecipeInputInventory};
use crate::crafting::crafting_inventory::CraftingInventory;
use crate::player::player_inventory::PlayerInventory;
use crate::screen_handler::{
    InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour, ScreenHandlerListener,
};
use crate::slot::{NormalSlot, Slot};

use crate::inventory::Inventory;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::RECIPES_CRAFTING;
use pumpkin_data::screen::WindowType;
use pumpkin_data::statistic::StatisticCategory;
use pumpkin_protocol::codec::recipe::DynamicRecipe;

/// The result slot in a crafting screen.
pub struct ResultSlot {
    /// The crafting inventory (grid) that provides recipe input.
    pub inventory: Arc<dyn RecipeInputInventory>,
    /// Protocol ID for this slot (assigned by screen handler).
    pub id: AtomicU8,
    /// The cached result item stack.
    pub result: Arc<Mutex<ItemStack>>,
    /// Provider for dynamic recipes.
    pub recipe_provider: Option<Arc<dyn RecipeProvider>>,
}

pub struct RecipeResult {
    pub stack: ItemStack,
    pub remaining_items: Vec<ItemStack>,
}

#[must_use]
pub fn match_crafting_recipe(
    inventory: &dyn RecipeInputInventory,
    provider: Option<&dyn RecipeProvider>,
) -> Option<RecipeResult> {
    let mut count: usize = 0;
    let inventory_width = inventory.get_width();
    let mut top_x = 9;
    let mut top_y = 9;
    let mut bottom_x = 0;
    let mut bottom_y = 0;
    for i in 0..inventory.size() {
        let x = i % inventory_width;
        let y = i / inventory_width;
        let slot = inventory.get_stack(i);
        if !slot.is_empty() {
            top_x = top_x.min(x);
            top_y = top_y.min(y);
            bottom_x = bottom_x.max(x);
            bottom_y = bottom_y.max(y);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let input_width = bottom_x + 1 - top_x;
    let input_height = bottom_y + 1 - top_y;

    for recipe in RECIPES_CRAFTING {
        if let Some(result) = recipe_matches(
            GenericRecipe::Vanilla(recipe),
            input_height,
            input_width,
            top_x,
            top_y,
            count,
            inventory,
        ) {
            return Some(result);
        }
    }

    if let Some(provider) = provider {
        let dynamic = provider.get_dynamic_recipes();
        for recipe in &dynamic {
            if let DynamicRecipe::Crafting(crafting) = recipe
                && let Some(result) = recipe_matches(
                    GenericRecipe::Dynamic(crafting),
                    input_height,
                    input_width,
                    top_x,
                    top_y,
                    count,
                    inventory,
                )
            {
                return Some(result);
            }
        }
    }

    None
}

impl ResultSlot {
    pub fn new(
        inventory: Arc<dyn RecipeInputInventory>,
        provider: Option<Arc<dyn RecipeProvider>>,
    ) -> Self {
        Self {
            inventory,
            id: AtomicU8::new(0),
            result: Arc::new(Mutex::new(ItemStack::EMPTY.clone())),
            recipe_provider: provider,
        }
    }

    fn match_recipe(&self) -> Option<RecipeResult> {
        match_crafting_recipe(&*self.inventory, self.recipe_provider.as_deref())
    }

    fn refill_output(&self) -> ItemStack {
        let result = self
            .match_recipe()
            .map_or_else(|| ItemStack::EMPTY.clone(), |r| r.stack);
        *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = result.clone();
        result
    }
}

impl Slot for ResultSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }
    fn get_index(&self) -> usize {
        999
    }
    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }
    fn on_quick_move_crafted(&self, _stack: ItemStack, _stack_prev: ItemStack) {
        self.refill_output();
    }
    fn on_take_item(&self, player: &dyn InventoryPlayer, stack: &ItemStack) {
        player.increment_stat(
            StatisticCategory::Crafted,
            stack.item.id as i32,
            stack.item_count as i32,
        );
        // ResultSlot.onTake obtains remainders before consuming any ingredients.
        if let Some(recipe) = self.match_recipe() {
            for (i, mut remainder) in recipe.remaining_items.into_iter().enumerate() {
                self.inventory.remove_stack_specific(i, 1);
                let remaining = self.inventory.get_stack(i);
                if remainder.is_empty() {
                    continue;
                }
                if remaining.is_empty() {
                    self.inventory.set_stack(i, remainder);
                } else if remaining.are_items_and_components_equal(&remainder) {
                    remainder.increment(remaining.item_count);
                    self.inventory.set_stack(i, remainder);
                } else {
                    crate::screen_handler::offer_or_drop_stack(player, remainder);
                }
            }
        }
        self.mark_dirty();
        self.refill_output();
    }
    fn can_insert(&self, _stack: &ItemStack) -> bool {
        false
    }
    fn get_stack(&self) -> ItemStack {
        self.result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn get_cloned_stack(&self) -> ItemStack {
        self.result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn has_stack(&self) -> bool {
        !self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
    fn set_stack(&self, _stack: ItemStack) {
        self.refill_output();
    }
    fn set_stack_prev(&self, _stack: ItemStack, _previous_stack: ItemStack) {
        self.refill_output();
    }
    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }
    fn get_max_item_count(&self) -> u8 {
        let mut count = u8::MAX;
        for i in 0..self.inventory.size() {
            let slot = self.inventory.get_stack(i);
            if !slot.is_empty() {
                count = count.min(slot.item_count);
            }
        }
        count
    }
    fn take_stack(&self, _amount: u8) -> ItemStack {
        // ResultSlot.remove -> Slot.remove -> ResultContainer.removeItem takes the entire craft.
        std::mem::replace(
            &mut *self
                .result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            ItemStack::EMPTY.clone(),
        )
    }
}

impl ScreenHandlerListener for ResultSlot {
    fn on_slot_update(&self, screen_handler: &ScreenHandlerBehaviour, slot: u8, _stack: ItemStack) {
        if (0..=(self.inventory.get_width() * self.inventory.get_height()))
            .contains(&(slot as usize))
        {
            let result = self.refill_output();
            let next_revision = screen_handler.next_revision();
            if let Some(sync_handler) = screen_handler.sync_handler.as_ref() {
                sync_handler.update_slot(screen_handler, 0, &result, next_revision);
            }
        }
    }
}

pub trait CraftingScreenHandler<I: RecipeInputInventory>:
    RecipeFinderScreenHandler + ScreenHandler
{
    fn add_recipe_slots(
        &mut self,
        crafing_inventory: Arc<dyn RecipeInputInventory>,
        provider: Option<Arc<dyn RecipeProvider>>,
    ) {
        let result_slot = Arc::new(ResultSlot::new(crafing_inventory.clone(), provider));
        self.add_slot(result_slot.clone());
        let width = crafing_inventory.get_width();
        let height = crafing_inventory.get_height();
        for i in 0..width {
            for j in 0..height {
                let input_slot = NormalSlot::new(crafing_inventory.clone(), j + i * width);
                self.add_slot(Arc::new(input_slot));
            }
        }
        self.add_listener(result_slot);
    }
}

pub struct CraftingTableScreenHandler {
    behaviour: ScreenHandlerBehaviour,
    crafting_inventory: Arc<dyn RecipeInputInventory>,
}

impl CraftingTableScreenHandler {
    pub fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        provider: Option<Arc<dyn RecipeProvider>>,
    ) -> Self {
        let crafting_inventory: Arc<dyn RecipeInputInventory> =
            Arc::new(CraftingInventory::new(3, 3));
        let mut crafting_table_handler = Self {
            behaviour: ScreenHandlerBehaviour::new(sync_id, Some(WindowType::Crafting)),
            crafting_inventory: crafting_inventory.clone(),
        };
        crafting_table_handler.add_recipe_slots(crafting_inventory, provider);
        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();
        crafting_table_handler.add_player_slots(&player_inventory);
        crafting_table_handler
    }
}

impl RecipeFinderScreenHandler for CraftingTableScreenHandler {}

impl ScreenHandler for CraftingTableScreenHandler {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        &self.behaviour
    }
    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        &mut self.behaviour
    }
    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.default_on_closed(player);
        self.drop_inventory(player, self.crafting_inventory.clone());
        // InventoryMenu.removed / CraftingMenu.removed: recompute the cached result after clearing inputs.
        self.get_behaviour().slots[0].set_stack(ItemStack::EMPTY.clone());
    }
    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let slot = self.get_behaviour().slots[slot_index as usize].clone();
        if slot.has_stack() {
            let mut slot_stack = slot.get_stack();
            let stack_prev = slot_stack.clone();
            if slot_index == 0 {
                if !self.insert_item(&mut slot_stack, 10, 46, true) {
                    return ItemStack::EMPTY.clone();
                }
            } else if (1..=9).contains(&slot_index) {
                if !self.insert_item(&mut slot_stack, 10, 46, false) {
                    return ItemStack::EMPTY.clone();
                }
            } else if (10..46).contains(&slot_index) {
                if !self.insert_item(&mut slot_stack, 1, 10, false) {
                    if slot_index < 37 {
                        if !self.insert_item(&mut slot_stack, 37, 46, false) {
                            return ItemStack::EMPTY.clone();
                        }
                    } else if !self.insert_item(&mut slot_stack, 10, 37, false) {
                        return ItemStack::EMPTY.clone();
                    }
                }
            } else if !self.insert_item(&mut slot_stack, 10, 46, false) {
                return ItemStack::EMPTY.clone();
            }
            let stack = slot_stack.clone();
            drop(slot_stack);
            if stack.is_empty() {
                slot.set_stack_prev(ItemStack::EMPTY.clone(), stack_prev.clone());
            } else {
                // CraftingMenu.quickMoveStack mutates the source, including partial transfers.
                slot.set_stack(stack.clone());
            }
            if stack.item_count == stack_prev.item_count {
                return ItemStack::EMPTY.clone();
            }

            let mut taken_stack = stack_prev.clone();
            taken_stack.set_count(stack_prev.item_count - stack.item_count);
            slot.on_take_item(player, &taken_stack);

            if slot_index == 0 {
                slot.on_quick_move_crafted(stack.clone(), stack_prev.clone());
                if !stack.is_empty() {
                    player.drop_item(stack, false);
                }
            }
            return stack_prev;
        }
        ItemStack::EMPTY.clone()
    }
}

impl CraftingScreenHandler<CraftingInventory> for CraftingTableScreenHandler {}
