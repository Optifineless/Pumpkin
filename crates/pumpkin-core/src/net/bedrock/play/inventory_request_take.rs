use pumpkin_data::{item_stack::ItemStack, screen::WindowType};
use pumpkin_inventory::{
    Inventory,
    crafting::{
        crafting_inventory::CraftingInventory,
        crafting_screen_handler::{ResultSlot, match_crafting_recipe},
        recipe_provider::RecipeProvider,
    },
    screen_handler::{InventoryPlayer, ScreenHandler},
};
use pumpkin_protocol::bedrock::{
    network_item::ContainerName,
    server::item_stack_request::{ItemStackRequestAction as Action, ItemStackRequestSlotInfo},
};

use super::item_stack_request::map_bedrock_container_slot;

pub(super) struct RecipeSnapshot(pub Vec<pumpkin_protocol::codec::recipe::DynamicRecipe>);

impl RecipeProvider for RecipeSnapshot {
    fn get_dynamic_recipes(&self) -> Vec<pumpkin_protocol::codec::recipe::DynamicRecipe> {
        self.0.clone()
    }
}

struct TakePreview<'a> {
    handler: &'a dyn ScreenHandler,
    stacks: Vec<ItemStack>,
    physical: Vec<usize>,
    pending_craft: Option<(ItemStack, u8)>,
    crafting_started: bool,
}

impl<'a> TakePreview<'a> {
    fn new(handler: &'a dyn ScreenHandler) -> Self {
        let slots = &handler.get_behaviour().slots;
        let physical = slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                slots[..index]
                    .iter()
                    .position(|other| {
                        other.get_index() == slot.get_index()
                            && std::sync::Arc::ptr_eq(&other.get_inventory(), &slot.get_inventory())
                    })
                    .unwrap_or(index)
            })
            .collect();
        let mut stacks: Vec<_> = slots.iter().map(|slot| slot.get_cloned_stack()).collect();
        stacks.push(
            handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        );
        stacks.push(ItemStack::EMPTY.clone());
        Self {
            handler,
            stacks,
            physical,
            pending_craft: None,
            crafting_started: false,
        }
    }

    fn index(&self, info: &ItemStackRequestSlotInfo) -> Option<usize> {
        let length = self.physical.len();
        match info.container_name.container_name {
            ContainerName::Cursor => Some(length),
            ContainerName::CreatedOutput
                if self.handler.window_type() != Some(WindowType::Merchant) =>
            {
                Some(length + 1)
            }
            name => map_bedrock_container_slot(self.handler, name, info.slot_id)
                .and_then(|index| self.physical.get(index).copied()),
        }
    }

    fn can_take(&self, index: usize, player: &dyn InventoryPlayer) -> bool {
        self.stacks[index].is_empty()
            || self
                .handler
                .get_behaviour()
                .slots
                .get(index)
                .is_none_or(|slot| slot.can_take_stack(player, &self.stacks[index]))
    }

    fn transfer(&mut self, source: usize, destination: usize, count: u8) {
        let count = count.min(self.stacks[source].item_count);
        let stack = self.stacks[source].copy_with_count(count);
        if self.stacks[destination].is_empty() {
            self.stacks[destination] = stack;
        } else if self.stacks[destination].are_items_and_components_equal(&stack) {
            self.stacks[destination].item_count =
                self.stacks[destination].item_count.saturating_add(count);
        } else {
            return;
        }
        self.stacks[source].decrement(count);
    }

    fn grid(&self) -> CraftingInventory {
        let width = if self.handler.window_type().is_none() {
            2
        } else {
            3
        };
        let grid = CraftingInventory::new(width, width);
        for index in 0..usize::from(width * width) {
            grid.set_stack(index, self.stacks[self.physical[1 + index]].clone());
        }
        grid
    }

    fn craft(&mut self, repetitions: u8, recipes: &RecipeSnapshot) {
        if self.handler.window_type() == Some(WindowType::Merchant) || repetitions == 0 {
            return;
        }
        let output = match_crafting_recipe(&self.grid(), Some(recipes))
            .map_or_else(|| ItemStack::EMPTY.clone(), |result| result.stack);
        let length = self.physical.len();
        self.stacks[length + 1] =
            output.copy_with_count(output.item_count.saturating_mul(repetitions));
        self.pending_craft = Some((output, repetitions));
        self.crafting_started = true;
    }

    fn pay_craft(&mut self, recipes: &RecipeSnapshot) -> bool {
        let Some((output, repetitions)) = self.pending_craft.take() else {
            return true;
        };
        let grid = self.grid();
        if ResultSlot::simulate_crafts(&grid, recipes, &output, repetitions).is_none() {
            return false;
        }
        for index in 0..grid.size() {
            self.stacks[self.physical[1 + index]] = grid.get_stack(index);
        }
        true
    }

    fn skips_consumption(&self, action: &Action) -> bool {
        let (Action::Consume { source, .. } | Action::Destroy { source, .. }) = action else {
            return false;
        };
        let name = source.container_name.container_name;
        (self.crafting_started && name == ContainerName::CraftingInput)
            || (self.handler.window_type() == Some(WindowType::Merchant)
                && matches!(
                    name,
                    ContainerName::TradeIngredient1
                        | ContainerName::TradeIngredient2
                        | ContainerName::Trade2Ingredient1
                        | ContainerName::Trade2Ingredient2
                ))
    }
}

// Check every Slot.mayPickup before any request action can pay, drop or publish.
// Shadow movements also catch a request that equips Binding before trying to remove it.
pub(super) fn validate_take_rules(
    handler: &dyn ScreenHandler,
    actions: &[Action],
    player: &dyn InventoryPlayer,
    recipes: &RecipeSnapshot,
) -> bool {
    let mut preview = TakePreview::new(handler);
    for action in actions {
        if preview.skips_consumption(action) {
            continue;
        }
        match action {
            Action::Take {
                count,
                source,
                destination,
            }
            | Action::Place {
                count,
                source,
                destination,
            } if *count > 0 => {
                if source.container_name.container_name == ContainerName::CreatedOutput
                    && !preview.pay_craft(recipes)
                {
                    return false;
                }
                if let (Some(source), Some(destination)) =
                    (preview.index(source), preview.index(destination))
                {
                    if !preview.can_take(source, player) {
                        return false;
                    }
                    preview.transfer(source, destination, *count);
                }
            }
            Action::Swap { slot1, slot2 } => {
                if let (Some(first), Some(second)) = (preview.index(slot1), preview.index(slot2)) {
                    let stack1 = &preview.stacks[first];
                    let stack2 = &preview.stacks[second];
                    if (stack1.uid != stack2.uid || !stack1.are_equal(stack2))
                        && (!preview.can_take(first, player) || !preview.can_take(second, player))
                    {
                        return false;
                    }
                    preview.stacks.swap(first, second);
                }
            }
            Action::Drop { count, source, .. }
            | Action::Destroy { count, source }
            | Action::Consume { count, source }
                if *count > 0 =>
            {
                if let Some(source) = preview.index(source) {
                    if !preview.can_take(source, player) {
                        return false;
                    }
                    let count = (*count).min(preview.stacks[source].item_count);
                    preview.stacks[source].decrement(count);
                }
            }
            Action::CraftRecipe { repetitions, .. }
            | Action::CraftRecipeAuto { repetitions, .. } => {
                preview.craft(*repetitions, recipes);
            }
            _ => {}
        }
    }
    true
}
