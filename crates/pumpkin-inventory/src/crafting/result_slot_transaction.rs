use super::{
    crafting_inventory::CraftingInventory,
    crafting_screen_handler::{ResultSlot, match_crafting_recipe},
    recipe_provider::RecipeProvider,
};
use crate::{Inventory, inventory::slot_transaction, screen_handler::InventoryPlayer, slot::Slot};
use pumpkin_data::{item_stack::ItemStack, statistic::StatisticCategory};

struct RecipeSnapshot(Vec<pumpkin_protocol::codec::recipe::DynamicRecipe>);

impl RecipeProvider for RecipeSnapshot {
    fn get_dynamic_recipes(&self) -> Vec<pumpkin_protocol::codec::recipe::DynamicRecipe> {
        self.0.clone()
    }
}

/// Holds the effects of a paid crafting transfer until inventory ownership is released.
#[must_use = "Finish the crafting effects after releasing the menu guard"]
pub struct CraftingCommit {
    grid: std::sync::Arc<dyn super::recipes::RecipeInputInventory>,
    target: std::sync::Arc<dyn Inventory>,
    target_slot: Option<usize>,
    grid_slots: slot_transaction::ChangedSlots,
    output: ItemStack,
    repetitions: u8,
    excess: Vec<ItemStack>,
}

impl CraftingCommit {
    /// Publishes notifications, remainders and crafting statistics.
    pub fn finish(self, player: &dyn InventoryPlayer) {
        self.publish(player);
    }

    // Java menu callers use InventoryPlayer's existing callback deferral; Bedrock releases its menu.
    fn publish(self, player: &dyn InventoryPlayer) {
        for i in self.grid_slots {
            self.grid.on_slot_changed(i);
        }
        if let Some(slot) = self.target_slot {
            self.target.on_slot_changed(slot);
        }
        for stack in self.excess {
            crate::screen_handler::offer_or_drop_stack(player, stack);
        }
        for _ in 0..self.repetitions {
            player.increment_stat(
                StatisticCategory::Crafted,
                i32::from(self.output.item.id),
                i32::from(self.output.item_count),
            );
        }
    }
}

impl ResultSlot {
    // ResultSlot.onTake consumes recipe inputs and preserves their remainders.
    // Bedrock's virtual output must pay those inputs in the same commit as its destination.
    pub(crate) fn commit_craft_into(
        &self,
        output: &ItemStack,
        repetitions: u8,
        destination: &dyn Slot,
        expected_destination: &ItemStack,
        replacement: &ItemStack,
    ) -> Option<CraftingCommit> {
        let recipes = RecipeSnapshot(
            self.recipe_provider
                .as_ref()
                .map_or_else(Vec::new, |provider| provider.get_dynamic_recipes()),
        );
        let target = destination.get_inventory();
        let owners = slot_transaction::ordered_owners(&[
            self.inventory.as_ref() as &dyn Inventory,
            target.as_ref(),
        ]);
        let guards = slot_transaction::lock_owners(&owners);
        if repetitions == 0 || !owners.iter().all(|owner| owner.is_live()) {
            return None;
        }
        let same_grid = std::sync::Arc::ptr_eq(
            &target,
            &(self.inventory.clone() as std::sync::Arc<dyn Inventory>),
        );
        let same_owner = std::sync::Arc::ptr_eq(
            &target.slot_transaction(destination.get_index()),
            &self.inventory.slot_transaction(0),
        );
        let current = destination.get_stack();
        if current.uid != expected_destination.uid
            || !current.are_equal(expected_destination)
            || !destination.can_insert(replacement)
            || replacement.item_count > destination.get_max_item_count_for_stack(replacement)
            || destination.get_index() >= target.size()
            || (same_owner && (!same_grid || !current.is_empty()))
        {
            return None;
        }
        let produced = output.item_count.checked_mul(repetitions)?;
        // ItemStack.getCount reports zero for every empty stack, including AIR.
        let current_count = if current.is_empty() {
            0
        } else {
            current.item_count
        };
        let added = replacement.item_count.checked_sub(current_count)?;
        if added == 0
            || added > produced
            || !replacement.are_items_and_components_equal(output)
            || (!current.is_empty() && !current.are_items_and_components_equal(output))
        {
            return None;
        }
        let (simulation, expected) = self.snapshot_inputs()?;
        let excess = Self::simulate_crafts(&simulation, &recipes, output, repetitions)?;
        if same_grid {
            if !simulation.get_stack(destination.get_index()).is_empty() {
                return None;
            }
            simulation.set_stack(destination.get_index(), replacement.clone());
        }
        let grid_slots = self.apply_crafting_inputs(&simulation, &expected);
        if !same_grid {
            let committed = slot_transaction::exchange(
                target.as_ref(),
                destination.get_index(),
                expected_destination,
                replacement.clone(),
            );
            debug_assert!(committed);
        }
        drop(guards);
        self.refill_output();
        Some(CraftingCommit {
            grid: self.inventory.clone(),
            target,
            target_slot: (!same_grid).then_some(destination.get_index()),
            grid_slots,
            output: output.clone(),
            repetitions,
            excess,
        })
    }

    fn snapshot_inputs(&self) -> Option<(CraftingInventory, Vec<ItemStack>)> {
        let simulation = CraftingInventory::new(
            self.inventory.get_width().try_into().ok()?,
            self.inventory.get_height().try_into().ok()?,
        );
        if simulation.size() != self.inventory.size() {
            return None;
        }
        let mut expected = Vec::with_capacity(self.inventory.size());
        for i in 0..self.inventory.size() {
            let stack = self.inventory.get_stack(i);
            simulation.set_stack(i, stack.clone());
            expected.push(stack);
        }
        Some((simulation, expected))
    }

    fn apply_crafting_inputs(
        &self,
        simulation: &CraftingInventory,
        expected: &[ItemStack],
    ) -> slot_transaction::ChangedSlots {
        let mut changed = slot_transaction::ChangedSlots::new();
        for (i, before) in expected.iter().enumerate() {
            let replacement = simulation.get_stack(i);
            if replacement.uid != before.uid || !replacement.are_equal(before) {
                let committed =
                    slot_transaction::exchange(self.inventory.as_ref(), i, before, replacement);
                debug_assert!(committed);
                changed.push(i);
            }
        }
        changed
    }

    fn simulate_crafts(
        simulation: &CraftingInventory,
        recipes: &RecipeSnapshot,
        output: &ItemStack,
        repetitions: u8,
    ) -> Option<Vec<ItemStack>> {
        let mut excess = Vec::new();
        for _ in 0..repetitions {
            let recipe = match_crafting_recipe(simulation, Some(recipes))?;
            if !recipe.stack.are_equal(output) {
                return None;
            }
            for (i, mut remainder) in recipe.remaining_items.into_iter().enumerate() {
                simulation.remove_stack_specific(i, 1);
                let remaining = simulation.get_stack(i);
                slot_transaction::deposit(
                    simulation,
                    i,
                    &mut remainder,
                    remaining.get_max_stack_size(),
                );
                if !remainder.is_empty() {
                    excess.push(remainder);
                }
            }
        }
        Some(excess)
    }
}

impl ResultSlot {
    // CraftingMenu / InventoryMenu.quickMoveStack and ResultSlot.onTake are one paid click.
    pub(crate) fn commit_quick_move(
        &self,
        player: &dyn InventoryPlayer,
        slots: &[std::sync::Arc<dyn Slot>],
        range: (i32, i32, bool),
    ) -> ItemStack {
        // Recipe providers may call plugins, so snapshot before acquiring inventory owners.
        let recipes = RecipeSnapshot(
            self.recipe_provider
                .as_ref()
                .map_or_else(Vec::new, |provider| provider.get_dynamic_recipes()),
        );
        let output = self.result.get_stack(0);
        let committed = crate::menu_transfer::transfer_result(
            slots,
            (self.result.as_ref(), 0, Some(self.inventory.as_ref())),
            &output,
            range,
            false,
            || {
                let (simulation, expected) = self.snapshot_inputs()?;
                let excess = Self::simulate_crafts(&simulation, &recipes, &output, 1)?;
                let changed = self.apply_crafting_inputs(&simulation, &expected);
                Some((changed, excess))
            },
        );
        let Some(((grid_slots, excess), leftover)) = committed else {
            return ItemStack::EMPTY.clone();
        };
        let crafted = output.copy_with_count(output.item_count - leftover.item_count);
        CraftingCommit {
            grid: self.inventory.clone(),
            target: self.result.clone(),
            target_slot: None,
            grid_slots,
            output: crafted,
            repetitions: 1,
            excess,
        }
        .publish(player);
        self.refill_output();
        if !leftover.is_empty() {
            player.drop_item(leftover, false);
        }
        output
    }
}
