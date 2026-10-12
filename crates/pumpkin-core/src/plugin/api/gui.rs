use std::any::Any;
use std::sync::Arc;
use std::sync::RwLock;

use pumpkin_data::{item_stack::ItemStack, screen::WindowType};
use pumpkin_inventory::screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour};
use pumpkin_inventory::slot::NormalSlot;
use pumpkin_inventory::{Clearable, Inventory};
use pumpkin_util::text::TextComponent;

pub struct PluginGui {
    pub window_type: WindowType,
    pub title: TextComponent,
    pub inventory: Arc<PluginInventory>,
    pub allow_grab_items: bool,
    pub allow_put_items: bool,
}

pub struct PluginInventory {
    pub(crate) transaction:
        std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    pub slots: RwLock<Vec<ItemStack>>,
}

impl PluginInventory {
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            transaction: std::sync::Arc::default(),
            slots: RwLock::new(vec![ItemStack::EMPTY.clone(); size]),
        }
    }
}

impl Clearable for PluginInventory {
    fn clear(&self) {
        pumpkin_inventory::inventory::slot_transaction::clear(self);
    }
}

impl Inventory for PluginInventory {
    fn slot_transaction(
        &self,
        _slot: usize,
    ) -> std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        let mut items = self
            .slots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stack) = items.get_mut(slot) else {
            return;
        };
        update(stack);
    }

    fn size(&self) -> usize {
        self.slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn is_empty(&self) -> bool {
        let slots = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots.iter().all(ItemStack::is_empty)
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        let slots = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots
            .get(slot)
            .cloned()
            .unwrap_or_else(|| ItemStack::EMPTY.clone())
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            removed = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });
        if !removed.is_empty() {
            self.on_slot_changed(slot);
        }
        removed
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            if !stack.is_empty() && amount > 0 {
                removed = stack.split(amount);
            }
        });
        if !removed.is_empty() {
            self.on_slot_changed(slot);
        }
        removed
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        let mut replacement = Some(stack);
        self.update_slot(slot, &mut |current| {
            if let Some(stack) = replacement.take() {
                *current = stack;
            }
        });
        self.on_slot_changed(slot);
    }

    fn on_open(&self) {}

    fn on_close(&self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct PluginScreenHandler {
    pub inventory: Arc<PluginInventory>,
    behaviour: ScreenHandlerBehaviour,
}

impl PluginScreenHandler {
    #[must_use]
    pub fn new(
        sync_id: u8,
        window_type: WindowType,
        inventory: &Arc<PluginInventory>,
        allow_grab_items: bool,
        allow_put_items: bool,
    ) -> Self {
        let mut behaviour = ScreenHandlerBehaviour::new(sync_id, Some(window_type));
        behaviour.allow_grab_items = allow_grab_items;
        behaviour.allow_put_items = allow_put_items;
        behaviour.container_slots = inventory.size();

        let mut handler = Self {
            inventory: inventory.clone(),
            behaviour,
        };

        for i in 0..inventory.size() {
            handler.add_slot(Arc::new(NormalSlot::new(inventory.clone(), i)));
        }

        handler
    }
}

impl ScreenHandler for PluginScreenHandler {
    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.default_on_closed(player);
        self.inventory.on_close();
    }

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

    fn quick_move(&mut self, _player: &dyn InventoryPlayer, _slot_index: i32) -> ItemStack {
        ItemStack::EMPTY.clone()
    }
}
