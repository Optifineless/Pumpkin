use crate::screen_handler::ScreenProperty;
use crate::window_property::PropertyDelegate;
use std::any::Any;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};

use crate::player::player_inventory::PlayerInventory;
use crate::screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour};
use crate::slot::{NormalSlot, Slot};

use crate::inventory::Inventory;
use crate::inventory::SimpleInventory;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::{RECIPES_STONECUTTING, StonecutterRecipe};
use pumpkin_data::screen::WindowType;
use pumpkin_data::statistic::StatisticCategory;
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::{client::play::CSetContainerProperty, server::play::SlotActionType},
};

pub struct StonecutterScreenHandler {
    behaviour: ScreenHandlerBehaviour,
    pub input_inventory: Arc<SimpleInventory>,
    pub output_inventory: Arc<SimpleInventory>,
    pub selected_recipe: Arc<StonecutterSelection>,
    previous_input: Mutex<ItemStack>,
}

impl StonecutterScreenHandler {
    pub fn new(sync_id: u8, player_inventory: &Arc<PlayerInventory>) -> Self {
        let behaviour = ScreenHandlerBehaviour::new(sync_id, Some(WindowType::Stonecutter));
        let input_inventory = Arc::new(SimpleInventory::new(1));
        let output_inventory = Arc::new(SimpleInventory::new(1));

        let mut handler = Self {
            behaviour,
            input_inventory: input_inventory.clone(),
            output_inventory: output_inventory.clone(),
            selected_recipe: Arc::new(StonecutterSelection(AtomicI32::new(-1))),
            previous_input: Mutex::new(ItemStack::EMPTY.clone()),
        };

        handler.add_slot(Arc::new(NormalSlot::new(
            input_inventory.clone() as Arc<dyn Inventory>,
            0,
        )));
        handler.add_slot(Arc::new(StonecutterOutputSlot::new(
            output_inventory as Arc<dyn Inventory>,
            input_inventory as Arc<dyn Inventory>,
            handler.selected_recipe.clone(),
            0,
        )));

        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();

        handler.add_player_slots(&player_inventory);
        handler.add_property(ScreenProperty::new(handler.selected_recipe.clone(), 0));

        handler
    }

    fn update_output(&self) {
        // StonecutterMenu.slotsChanged resets selection when the input item type changes.
        let input = self.input_inventory.get_stack(0);
        let mut previous = self
            .previous_input
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if input.item != previous.item {
            self.selected_recipe.0.store(-1, Ordering::Relaxed);
            *previous = input;
        }
        refresh_output(
            &*self.input_inventory,
            &*self.output_inventory,
            &self.selected_recipe,
        );
    }

    fn get_available_recipes(input: &ItemStack) -> Vec<&'static StonecutterRecipe> {
        let item = input.item;
        RECIPES_STONECUTTING
            .iter()
            .filter(|r| r.ingredient.match_item(item))
            .collect()
    }

    fn sync_selection(&self, player: &dyn InventoryPlayer, previous: i32) {
        let selected = self.selected_recipe.0.load(Ordering::Relaxed);
        if previous != selected {
            // StonecutterMenu broadcasts its selectedRecipeIndex DataSlot after changes.
            player.enqueue_property_packet(&CSetContainerProperty::new(
                VarInt(i32::from(self.sync_id())),
                0,
                selected as i16,
            ));
        }
    }
}

impl ScreenHandler for StonecutterScreenHandler {
    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        &self.behaviour
    }

    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        &mut self.behaviour
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn on_slot_click(
        &mut self,
        slot_index: i32,
        button: i32,
        action_type: SlotActionType,
        player: &dyn InventoryPlayer,
    ) {
        let previous = self.selected_recipe.0.load(Ordering::Relaxed);
        self.internal_on_slot_click(slot_index, button, action_type, player);
        self.update_output();
        self.sync_selection(player, previous);
    }

    fn on_button_click(&mut self, player: &dyn InventoryPlayer, button: i32) -> bool {
        let previous = self.selected_recipe.0.load(Ordering::Relaxed);
        self.update_output();
        // StonecutterMenu.clickMenuButton selects a valid recipe and broadcasts its output.
        if self.selected_recipe.0.load(Ordering::Relaxed) == button {
            self.sync_selection(player, previous);
            return false;
        }
        if usize::try_from(button).is_ok_and(|i| {
            i < Self::get_available_recipes(&self.input_inventory.get_stack(0)).len()
        }) {
            self.selected_recipe.0.store(button, Ordering::Relaxed);
            self.update_output();
            self.send_content_updates();
        }
        self.sync_selection(player, previous);
        true
    }

    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        // StonecutterMenu.removed discards output and returns the unspent input.
        self.default_on_closed(player);
        self.output_inventory.set_stack(0, ItemStack::EMPTY.clone());
        self.drop_inventory(player, self.input_inventory.clone());
    }

    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let Some(slot) = self.get_behaviour().slots.get(slot_index as usize).cloned() else {
            return ItemStack::EMPTY.clone();
        };
        let mut stack = slot.get_stack();
        if stack.is_empty() {
            return ItemStack::EMPTY.clone();
        }
        let original = stack.clone();
        let previous = self.selected_recipe.0.load(Ordering::Relaxed);
        let (start, end, reverse) = if slot_index < 2 {
            (2, 38, slot_index == 1)
        } else if !Self::get_available_recipes(&stack).is_empty() {
            (0, 1, false)
        } else if slot_index < 29 {
            (29, 38, false)
        } else {
            (2, 29, false)
        };
        if !self.insert_item(&mut stack, start, end, reverse) {
            return ItemStack::EMPTY.clone();
        }
        slot.set_stack(stack.clone());
        if slot_index == 1 {
            slot.on_take_item(
                player,
                &original.copy_with_count(original.item_count - stack.item_count),
            );
            if !stack.is_empty() {
                player.drop_item(stack, false);
            }
        }
        self.update_output();
        self.sync_selection(player, previous);
        original
    }
}

pub struct StonecutterOutputSlot {
    pub inventory: Arc<dyn Inventory>,
    pub input_inventory: Arc<dyn Inventory>,
    pub index: usize,
    pub id: AtomicU8,
    selection: Arc<StonecutterSelection>,
}

impl StonecutterOutputSlot {
    pub fn new(
        inventory: Arc<dyn Inventory>,
        input_inventory: Arc<dyn Inventory>,
        selection: Arc<StonecutterSelection>,
        index: usize,
    ) -> Self {
        Self {
            inventory,
            input_inventory,
            selection,
            index,
            id: AtomicU8::new(0),
        }
    }
}

impl Slot for StonecutterOutputSlot {
    fn take_stack(&self, _amount: u8) -> ItemStack {
        // StonecutterMenu uses ResultContainer.removeItem: take the whole result.
        self.inventory.remove_stack(self.index)
    }

    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }

    fn get_index(&self) -> usize {
        self.index
    }

    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }

    fn on_take_item(&self, player: &dyn InventoryPlayer, stack: &ItemStack) {
        player.increment_stat(
            StatisticCategory::Crafted,
            stack.item.id as i32,
            stack.item_count as i32,
        );
        self.input_inventory.remove_stack_specific(0, 1);
        refresh_output(&*self.input_inventory, &*self.inventory, &self.selection);
        self.mark_dirty();
    }

    fn can_insert(&self, _stack: &ItemStack) -> bool {
        false
    }

    fn get_stack(&self) -> ItemStack {
        self.inventory.get_stack(self.index)
    }

    fn get_cloned_stack(&self) -> ItemStack {
        self.inventory.get_stack(self.index)
    }

    fn has_stack(&self) -> bool {
        !self.inventory.get_stack(self.index).is_empty()
    }

    fn set_stack(&self, stack: ItemStack) {
        self.inventory.set_stack(self.index, stack);
    }

    fn set_stack_prev(&self, _stack: ItemStack, _previous_stack: ItemStack) {
        // Do nothing
    }

    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }
}

pub struct StonecutterSelection(AtomicI32);
impl PropertyDelegate for StonecutterSelection {
    fn get_property(&self, _index: i32) -> i32 {
        self.0.load(Ordering::Relaxed)
    }
    fn set_property(&self, _index: i32, value: i32) {
        self.0.store(value, Ordering::Relaxed);
    }
    fn get_properties_size(&self) -> i32 {
        1
    }
}

fn refresh_output(input: &dyn Inventory, output: &dyn Inventory, selection: &StonecutterSelection) {
    let input = input.get_stack(0);
    let recipes = StonecutterScreenHandler::get_available_recipes(&input);
    let result = usize::try_from(selection.0.load(Ordering::Relaxed))
        .ok()
        .and_then(|i| recipes.get(i))
        .filter(|_| !input.is_empty())
        .map_or_else(
            || ItemStack::EMPTY.clone(),
            |recipe| recipe.result.assemble(None, 0),
        );
    output.set_stack(0, result);
}
