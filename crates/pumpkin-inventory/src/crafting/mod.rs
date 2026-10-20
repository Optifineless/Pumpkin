//! Crafting module.
//!
//! This module handles crafting mechanics including:
//! - [`CraftingInventory`] - Temporary inventory for crafting slots
//! - [`CraftingScreenHandler`] - Screen handler for crafting tables and inventory crafting
//! - [`recipes`] - Recipe matching and crafting result calculation

pub mod crafting_inventory;
pub mod crafting_screen_handler;
pub mod recipe_provider;
pub mod recipes;

mod recipe_matching;
pub use recipe_matching::crafting_remainder;
mod special_recipes;

mod result_slot_transaction;

pub use result_slot_transaction::CraftingCommit;
