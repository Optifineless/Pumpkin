//! Crafting inventory implementation.
//!
//! This module provides a temporary inventory for crafting grids.
//! Crafting inventories are used for:
//! - The 2x2 crafting grid in the player inventory
//! - The 3x3 crafting grid in crafting tables
//! - Other recipe-based crafting mechanisms
//!
//! Unlike regular inventories, crafting grids are typically cleared when
//! the container closes, and their contents are used up when crafting.

use std::any::Any;
use std::sync::RwLock;

use crate::inventory::{Clearable, Inventory};
use pumpkin_data::item_stack::ItemStack;

use super::recipes::RecipeInputInventory;

/// A temporary inventory for crafting grids.
///
/// Crafting inventories hold items arranged in a grid pattern for crafting recipes.
/// The grid dimensions can vary (2x2 for player inventory, 3x3 for crafting table).
///
/// # Usage
///
/// When a player places items in the crafting grid, they are stored here.
/// When the crafting result is taken, the ingredients are consumed from this inventory.
#[derive(Default)]
pub struct CraftingInventory {
    /// Width of the crafting grid (typically 2 or 3).
    pub width: u8,
    /// Height of the crafting grid (typically 2 or 3).
    pub height: u8,
    pub(crate) transaction: std::sync::Arc<crate::inventory::slot_transaction::Transaction>,
    /// Items in the crafting grid, stored row by row.
    pub items: RwLock<Vec<ItemStack>>,
}

impl CraftingInventory {
    /// Creates a new crafting inventory with the given dimensions.
    ///
    /// # Arguments
    /// - `width` - Grid width (e.g., 2 for player crafting, 3 for crafting table)
    /// - `height` - Grid height (e.g., 2 for player crafting, 3 for crafting table)
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // 2x2 player inventory crafting grid
    /// let player_crafting = CraftingInventory::new(2, 2);
    ///
    /// // 3x3 crafting table grid
    /// let table_crafting = CraftingInventory::new(3, 3);
    /// ```
    #[must_use]
    pub fn new(width: u8, height: u8) -> Self {
        Self {
            width,
            height,
            transaction: std::sync::Arc::default(),
            items: RwLock::new(vec![
                ItemStack::EMPTY.clone();
                width as usize * height as usize
            ]),
        }
    }
}

impl Inventory for CraftingInventory {
    fn slot_transaction(
        &self,
        _slot: usize,
    ) -> std::sync::Arc<crate::inventory::slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        let mut items = self
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stack) = items.get_mut(slot) else {
            return;
        };
        update(stack);
    }

    fn size(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    fn is_empty(&self) -> bool {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.iter().all(ItemStack::is_empty)
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items
            .get(slot)
            .cloned()
            .unwrap_or_else(|| ItemStack::EMPTY.clone())
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            removed = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });

        removed
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            if !stack.is_empty() && amount > 0 {
                removed = stack.split(amount);
            }
        });

        removed
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        let mut replacement = Some(stack);
        self.update_slot(slot, &mut |current| {
            if let Some(stack) = replacement.take() {
                *current = stack;
            }
        });
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RecipeInputInventory for CraftingInventory {
    fn get_width(&self) -> usize {
        self.width as usize
    }

    fn get_height(&self) -> usize {
        self.height as usize
    }
}

impl Clearable for CraftingInventory {
    fn clear(&self) {
        crate::inventory::slot_transaction::clear(self);
    }
}
