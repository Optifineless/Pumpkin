use std::{any::Any, sync::Arc};

use crate::inventory::{Inventory, SimpleInventory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{item_stack::ItemStack, screen::WindowType};

use crate::{
    player::player_inventory::PlayerInventory,
    screen_handler::{
        InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour, ScreenProperty, offer_or_drop_stack,
    },
    slot::NormalSlot,
    window_property::PropertyDelegate,
};

/// Whether an item belongs to the vanilla beacon payment tag.
#[must_use]
pub fn is_beacon_payment_item(stack: &ItemStack) -> bool {
    !stack.is_empty()
        && stack
            .item
            .has_tag(&tag::Item::MINECRAFT_BEACON_PAYMENT_ITEMS)
}

struct BeaconPaymentSlot(NormalSlot);

impl BeaconPaymentSlot {
    fn new(inventory: Arc<dyn Inventory>) -> Self {
        Self(NormalSlot::new(
            inventory,
            BeaconScreenHandler::PAYMENT_SLOT,
        ))
    }
}

impl crate::slot::Slot for BeaconPaymentSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.0.get_inventory()
    }

    fn get_index(&self) -> usize {
        self.0.get_index()
    }

    fn set_id(&self, id: usize) {
        self.0.set_id(id);
    }

    fn can_insert(&self, stack: &ItemStack) -> bool {
        is_beacon_payment_item(stack)
    }

    fn get_max_item_count(&self) -> u8 {
        1
    }

    fn mark_dirty(&self) {
        self.0.mark_dirty();
    }
}

/// Creates a beacon container screen handler.
///
/// Beacons feature a single payment slot and a specialized UI for selecting status effects.
pub fn create_beacon_handler(
    sync_id: u8,
    player_inventory: &Arc<PlayerInventory>,
    beacon_data: Arc<dyn PropertyDelegate>,
) -> BeaconScreenHandler {
    BeaconScreenHandler::new(sync_id, player_inventory, beacon_data)
}

type ValidityCheck = Box<dyn Fn(&dyn InventoryPlayer) -> bool + Send + Sync>;

/// Screen handler specifically for Beacon blocks.
pub struct BeaconScreenHandler {
    /// This menu's one-slot payment inventory.
    pub inventory: Arc<dyn Inventory>,
    /// The block entity's synchronized levels and selected effects.
    pub beacon_data: Arc<dyn PropertyDelegate>,
    /// The server-side block entity retained by this menu, when available.
    pub block_entity: Option<Arc<dyn Any + Send + Sync>>,
    /// `BeaconMenu.stillValid`; `None` keeps the menu valid.
    pub validity_check: Option<ValidityCheck>,
    /// Core screen handler behavior (slots, sync ID, listeners).
    behaviour: ScreenHandlerBehaviour,
}

impl BeaconScreenHandler {
    pub const PAYMENT_SLOT: usize = 0;
    pub const SLOT_COUNT: usize = 1;
    pub const DATA_COUNT: u8 = 3;

    /// Creates a new beacon screen handler.
    fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        beacon_data: Arc<dyn PropertyDelegate>,
    ) -> Self {
        let inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(Self::SLOT_COUNT));
        let mut handler = Self {
            inventory: inventory.clone(),
            beacon_data,
            block_entity: None,
            validity_check: None,
            behaviour: ScreenHandlerBehaviour::new(sync_id, Some(WindowType::Beacon)),
        };
        handler.behaviour.container_slots = Self::SLOT_COUNT;

        handler.inventory.on_open();

        // BeaconMenu.addDataSlots tracks levels and the encoded selected effect IDs.
        for index in 0..Self::DATA_COUNT {
            handler.add_property(ScreenProperty::new(handler.beacon_data.clone(), index));
        }

        handler.add_slot(Arc::new(BeaconPaymentSlot::new(handler.inventory.clone())));

        // Add the player's inventory slots (27 slots + 9 hotbar)
        let player_inventory_arc: Arc<dyn Inventory> = player_inventory.clone();
        handler.add_player_slots(&player_inventory_arc);

        handler
    }

    /// Consumes the one eligible payment item after effect validation has succeeded.
    pub fn consume_payment(&self) -> bool {
        let mut consumed = false;
        self.inventory
            .update_slot(Self::PAYMENT_SLOT, &mut |stack| {
                if stack.item_count == 1 && is_beacon_payment_item(stack) {
                    *stack = ItemStack::EMPTY.clone();
                    consumed = true;
                }
            });
        if consumed {
            self.inventory.on_slot_changed(Self::PAYMENT_SLOT);
        }
        consumed
    }

    #[must_use]
    pub fn has_valid_payment(&self) -> bool {
        let payment = self.inventory.get_stack(Self::PAYMENT_SLOT);
        payment.item_count == 1 && is_beacon_payment_item(&payment)
    }
}

impl ScreenHandler for BeaconScreenHandler {
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

    fn can_use(&self, player: &dyn InventoryPlayer) -> bool {
        self.validity_check
            .as_ref()
            .is_none_or(|check| check(player))
    }

    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.default_on_closed(player);
        self.inventory.on_close();
        let payment = self.inventory.remove_stack(Self::PAYMENT_SLOT);
        if !payment.is_empty() {
            offer_or_drop_stack(player, payment);
        }
    }

    /// Quick move logic specifically for the beacon UI.
    ///
    /// - From beacon payment slot (0): Move to player inventory
    /// - From player inventory (1+): Move to beacon payment slot
    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let mut stack_left = ItemStack::EMPTY.clone();
        let slot = self.get_behaviour().slots[slot_index as usize].clone();

        if slot.has_stack() {
            let mut slot_stack = slot.get_stack();
            stack_left = slot_stack.clone();

            if slot_index == Self::PAYMENT_SLOT as i32 {
                // Move from the single beacon slot to the player inventory (slots 1 to end)
                if !self.transfer_item(
                    player,
                    slot.as_ref(),
                    &mut slot_stack,
                    (1, self.get_behaviour().slots.len() as i32, true),
                    &mut stack_left,
                ) {
                    return ItemStack::EMPTY.clone();
                }
            } else {
                if slot_stack.item_count != 1 || !is_beacon_payment_item(&slot_stack) {
                    return ItemStack::EMPTY.clone();
                }

                // Move from player inventory into the beacon payment slot (slot 0)
                if !self.transfer_item(
                    player,
                    slot.as_ref(),
                    &mut slot_stack,
                    (
                        Self::PAYMENT_SLOT as i32,
                        Self::PAYMENT_SLOT as i32 + 1,
                        false,
                    ),
                    &mut stack_left,
                ) {
                    return ItemStack::EMPTY.clone();
                }
            }
        }

        stack_left
    }
}
