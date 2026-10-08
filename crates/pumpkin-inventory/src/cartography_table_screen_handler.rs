use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::player::player_inventory::PlayerInventory;
use crate::screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour};
use crate::slot::Slot;

use crate::inventory::Inventory;
use crate::inventory::SimpleInventory;
use pumpkin_data::data_component_impl::{MapIdImpl, MapPostProcessingImpl};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::screen::WindowType;
use pumpkin_data::statistic::StatisticCategory;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_protocol::java::server::play::SlotActionType;

#[must_use]
pub fn is_map_item(stack: &ItemStack) -> bool {
    stack.get_data_component::<MapIdImpl>().is_some()
}

#[must_use]
pub const fn is_additional_item(stack: &ItemStack) -> bool {
    stack.item.id == Item::PAPER.id
        || stack.item.id == Item::MAP.id
        || stack.item.id == Item::GLASS_PANE.id
}

pub struct CartographyTableScreenHandler {
    behaviour: ScreenHandlerBehaviour,
    pub input_inventory: Arc<SimpleInventory>,
    pub output_inventory: Arc<SimpleInventory>,
}

impl CartographyTableScreenHandler {
    pub const MAP_SLOT: usize = 0;
    pub const ADDITIONAL_SLOT: usize = 1;
    pub const RESULT_SLOT: usize = 2;
    pub const INV_SLOT_START: usize = 3;
    pub const INV_SLOT_END: usize = 30;
    pub const USE_ROW_SLOT_START: usize = 30;
    pub const USE_ROW_SLOT_END: usize = 39;

    pub fn new(sync_id: u8, player_inventory: &Arc<PlayerInventory>) -> Self {
        let behaviour = ScreenHandlerBehaviour::new(sync_id, Some(WindowType::CartographyTable));
        let input_inventory = Arc::new(SimpleInventory::new(2));
        let output_inventory = Arc::new(SimpleInventory::new(1));

        let mut handler = Self {
            behaviour,
            input_inventory: input_inventory.clone(),
            output_inventory: output_inventory.clone(),
        };

        handler.add_slot(Arc::new(CartographyMapSlot::new(
            input_inventory.clone() as Arc<dyn Inventory>,
            0,
        )));
        handler.add_slot(Arc::new(CartographyAdditionalSlot::new(
            input_inventory.clone() as Arc<dyn Inventory>,
            1,
        )));
        handler.add_slot(Arc::new(CartographyResultSlot::new(
            output_inventory as Arc<dyn Inventory>,
            input_inventory as Arc<dyn Inventory>,
            0,
        )));

        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();
        handler.add_player_slots(&player_inventory);

        handler
    }

    pub fn slots_changed(&mut self, player: &dyn InventoryPlayer) {
        let map_stack = self.input_inventory.get_stack(0);
        let additional_stack = self.input_inventory.get_stack(1);
        let result_stack = self.output_inventory.get_stack(0);

        if result_stack.is_empty() || (!map_stack.is_empty() && !additional_stack.is_empty()) {
            if !map_stack.is_empty() && !additional_stack.is_empty() {
                self.setup_result_slot(player);
            }
        } else {
            self.output_inventory.set_stack(0, ItemStack::EMPTY.clone());
        }
    }

    pub fn setup_result_slot(&mut self, player: &dyn InventoryPlayer) {
        refresh_result(player, &*self.input_inventory, &*self.output_inventory);
    }
}

// CartographyTableMenu.setupResultSlot validates saved data before advertising an output.
fn refresh_result(player: &dyn InventoryPlayer, input: &dyn Inventory, output: &dyn Inventory) {
    let mut map = input.get_stack(0);
    let material = input.get_stack(1);
    let result = if !map.is_empty() && !material.is_empty() && is_map_item(&map) {
        player.map_crafting_state(&map).map_or_else(
            || ItemStack::EMPTY.clone(),
            |(scale, locked)| {
                if material.item == &Item::PAPER
                    && map.item.has_tag(&tag::Item::MINECRAFT_EXTENDABLE_MAPS)
                    && !locked
                    && scale < 4
                {
                    map.item_count = 1;
                    map.set_data_component(MapPostProcessingImpl::SCALE);
                    map
                } else if material.item == &Item::GLASS_PANE && !locked {
                    map.item_count = 1;
                    map.set_data_component(MapPostProcessingImpl::LOCK);
                    map
                } else if material.item == &Item::MAP {
                    map.copy_with_count(2)
                } else {
                    ItemStack::EMPTY.clone()
                }
            },
        )
    } else {
        ItemStack::EMPTY.clone()
    };
    let current = output.get_stack(0);
    if current.item_count != result.item_count || !current.are_items_and_components_equal(&result) {
        output.set_stack(0, result);
    }
}

impl ScreenHandler for CartographyTableScreenHandler {
    fn tick(&mut self, player: &dyn InventoryPlayer) {
        // CartographyTableMenu.setupResultSlot, retried after asynchronous SavedDataStorage.get.
        self.slots_changed(player);
    }

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
        self.internal_on_slot_click(slot_index, button, action_type, player);
        if (0..=2).contains(&slot_index) {
            self.slots_changed(player);
        }
    }

    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let mut clicked = ItemStack::EMPTY.clone();
        let slot = self.get_behaviour().slots.get(slot_index as usize).cloned();

        if let Some(slot) = slot {
            let mut stack = slot.get_cloned_stack();
            if !stack.is_empty() {
                clicked = stack.clone();
                if slot_index == 2 {
                    slot.on_crafted_by(player, &mut stack);
                    // Java mutates the slot's stack before moveItemStackTo can fail.
                    slot.set_stack(stack.clone());
                    clicked = stack.clone();
                    if !self.insert_item(&mut stack, 3, 39, true) {
                        return ItemStack::EMPTY.clone();
                    }
                    slot.on_quick_move_crafted(stack.clone(), clicked.clone());
                } else if slot_index != 1 && slot_index != 0 {
                    if is_map_item(&stack) {
                        if !self.insert_item(&mut stack, 0, 1, false) {
                            return ItemStack::EMPTY.clone();
                        }
                    } else if !is_additional_item(&stack) {
                        if (3..30).contains(&slot_index) {
                            if !self.insert_item(&mut stack, 30, 39, false) {
                                return ItemStack::EMPTY.clone();
                            }
                        } else if (30..39).contains(&slot_index)
                            && !self.insert_item(&mut stack, 3, 30, false)
                        {
                            return ItemStack::EMPTY.clone();
                        }
                    } else if !self.insert_item(&mut stack, 1, 2, false) {
                        return ItemStack::EMPTY.clone();
                    }
                } else if !self.insert_item(&mut stack, 3, 39, false) {
                    return ItemStack::EMPTY.clone();
                }
                if stack.is_empty() {
                    slot.set_stack(ItemStack::EMPTY.clone());
                } else {
                    slot.set_stack(stack.clone());
                }

                if stack.item_count == clicked.item_count {
                    return ItemStack::EMPTY.clone();
                }

                if slot_index == 2 {
                    slot.on_take_item(
                        player,
                        &clicked.copy_with_count(clicked.item_count - stack.item_count),
                    );
                }
                self.slots_changed(player);
            }
        }

        clicked
    }

    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.default_on_closed(player);
        self.drop_inventory(player, self.input_inventory.clone());
    }
}

pub struct CartographyMapSlot {
    inventory: Arc<dyn Inventory>,
    index: usize,
    id: AtomicU8,
}

impl CartographyMapSlot {
    pub fn new(inventory: Arc<dyn Inventory>, index: usize) -> Self {
        Self {
            inventory,
            index,
            id: AtomicU8::new(0),
        }
    }
}

impl Slot for CartographyMapSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }

    fn get_index(&self) -> usize {
        self.index
    }

    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }

    fn can_insert(&self, stack: &ItemStack) -> bool {
        is_map_item(stack)
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

    fn set_stack_prev(&self, _stack: ItemStack, _previous_stack: ItemStack) {}

    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }
}

pub struct CartographyAdditionalSlot {
    inventory: Arc<dyn Inventory>,
    index: usize,
    id: AtomicU8,
}

impl CartographyAdditionalSlot {
    pub fn new(inventory: Arc<dyn Inventory>, index: usize) -> Self {
        Self {
            inventory,
            index,
            id: AtomicU8::new(0),
        }
    }
}

impl Slot for CartographyAdditionalSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }

    fn get_index(&self) -> usize {
        self.index
    }

    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }

    fn can_insert(&self, stack: &ItemStack) -> bool {
        is_additional_item(stack)
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

    fn set_stack_prev(&self, _stack: ItemStack, _previous_stack: ItemStack) {}

    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }
}

pub struct CartographyResultSlot {
    inventory: Arc<dyn Inventory>,
    input_inventory: Arc<dyn Inventory>,
    index: usize,
    id: AtomicU8,
}

impl CartographyResultSlot {
    pub fn new(
        inventory: Arc<dyn Inventory>,
        input_inventory: Arc<dyn Inventory>,
        index: usize,
    ) -> Self {
        Self {
            inventory,
            input_inventory,
            index,
            id: AtomicU8::new(0),
        }
    }
}

impl Slot for CartographyResultSlot {
    fn take_stack(&self, _amount: u8) -> ItemStack {
        // CartographyTableMenu uses ResultContainer.removeItem: take the whole result.
        self.inventory.remove_stack(self.index)
    }

    fn on_crafted_by(&self, player: &dyn InventoryPlayer, stack: &mut ItemStack) {
        player.process_crafted_map(stack);
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

    fn can_insert(&self, _stack: &ItemStack) -> bool {
        false
    }

    fn on_take_item(&self, player: &dyn InventoryPlayer, stack: &ItemStack) {
        player.increment_stat(
            StatisticCategory::Crafted,
            stack.item.id as i32,
            stack.item_count as i32,
        );
        self.input_inventory.remove_stack_specific(0, 1);
        self.input_inventory.remove_stack_specific(1, 1);
        refresh_result(player, &*self.input_inventory, &*self.inventory);
        self.mark_dirty();
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

    fn set_stack_prev(&self, _stack: ItemStack, _previous_stack: ItemStack) {}

    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }
}
