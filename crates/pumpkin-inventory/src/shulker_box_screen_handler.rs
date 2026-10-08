use crate::{
    Inventory,
    generic_container_screen_handler::{GenericContainerScreenHandler, create_generic_9x3},
    player::player_inventory::PlayerInventory,
    screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour},
    slot::{NormalSlot, Slot},
};
use pumpkin_data::{
    item_stack::ItemStack,
    screen::WindowType,
    tag::{self, Taggable},
};
use std::{any::Any, sync::Arc};

pub struct ShulkerBoxSlot(NormalSlot);
impl ShulkerBoxSlot {
    pub fn new(inventory: Arc<dyn Inventory>, index: usize) -> Self {
        Self(NormalSlot::new(inventory, index))
    }
}
impl Slot for ShulkerBoxSlot {
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
        // ShulkerBoxSlot.mayPlace -> ShulkerBoxBlockItem.canFitInsideContainerItems.
        !stack.item.has_tag(&tag::Item::MINECRAFT_SHULKER_BOXES)
    }
}

type ValidityCheck = Box<dyn Fn(&dyn InventoryPlayer) -> bool + Send + Sync>;

pub struct ShulkerBoxScreenHandler {
    inner: GenericContainerScreenHandler,
    /// Checks the original block entity and player reach before interactions and each tick.
    pub validity_check: Option<ValidityCheck>,
}
impl ShulkerBoxScreenHandler {
    pub fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        inventory: Arc<dyn Inventory>,
        player: &dyn InventoryPlayer,
    ) -> Self {
        // ShulkerBoxMenu uses the same transfer layout, with restricted container slots.
        let mut inner = create_generic_9x3(sync_id, player_inventory, inventory, player);
        inner.get_behaviour_mut().window_type = Some(WindowType::ShulkerBox);
        for i in 0..inner.inventory.size() {
            let slot = ShulkerBoxSlot::new(inner.inventory.clone(), i);
            slot.set_id(i);
            inner.get_behaviour_mut().slots[i] = Arc::new(slot);
        }
        Self {
            inner,
            validity_check: None,
        }
    }
}
impl ScreenHandler for ShulkerBoxScreenHandler {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        self.inner.get_behaviour()
    }
    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        self.inner.get_behaviour_mut()
    }
    fn quick_move(&mut self, player: &dyn InventoryPlayer, index: i32) -> ItemStack {
        if !self.can_use(player) {
            return ItemStack::EMPTY.clone();
        }
        self.inner.quick_move(player, index)
    }
    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.inner.on_closed(player);
    }
    fn can_use(&self, player: &dyn InventoryPlayer) -> bool {
        // ShulkerBoxMenu.stillValid delegates to Container.stillValidBlockEntity.
        self.validity_check
            .as_ref()
            .is_none_or(|check| check(player))
    }
    fn on_slot_click(
        &mut self,
        index: i32,
        button: i32,
        action: pumpkin_protocol::java::server::play::SlotActionType,
        player: &dyn InventoryPlayer,
    ) {
        if self.can_use(player) {
            self.internal_on_slot_click(index, button, action, player);
        }
    }
}
