//! Generic container screen handler.
//!
//! This module provides a generic screen handler for simple containers like:
//! - Chests (single, double, ender chest)
//! - Hoppers
//! - Dispensers/Droppers
//! - Barrels
//!
//! These containers have a simple grid layout with no special behaviors
//! (no smelting, no crafting, just item storage).

use std::{any::Any, sync::Arc};

use crate::inventory::Inventory;
use pumpkin_data::{item_stack::ItemStack, screen::WindowType};

use crate::{
    player::player_inventory::PlayerInventory,
    screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour},
    slot::NormalSlot,
};

/// Creates a generic 9x3 container (single chest).
///
/// Used for single chests, ender chests, and similar containers.
pub fn create_generic_9x3(
    sync_id: u8,
    player_inventory: &Arc<PlayerInventory>,
    inventory: Arc<dyn Inventory>,
    player: &dyn InventoryPlayer,
) -> GenericContainerScreenHandler {
    GenericContainerScreenHandler::new(
        WindowType::Generic9x3,
        sync_id,
        player_inventory,
        inventory,
        3,
        9,
        player.is_spectator(),
    )
}

/// Creates a generic 9x6 container (double chest).
///
/// Used for double chests and similar large containers.
pub fn create_generic_9x6(
    sync_id: u8,
    player_inventory: &Arc<PlayerInventory>,
    inventory: Arc<dyn Inventory>,
    player: &dyn InventoryPlayer,
) -> GenericContainerScreenHandler {
    GenericContainerScreenHandler::new(
        WindowType::Generic9x6,
        sync_id,
        player_inventory,
        inventory,
        6,
        9,
        player.is_spectator(),
    )
}

/// Creates a generic 3x3 container.
///
/// Used for dispensers, droppers, and similar containers.
pub fn create_generic_3x3(
    sync_id: u8,
    player_inventory: &Arc<PlayerInventory>,
    inventory: Arc<dyn Inventory>,
    player: &dyn InventoryPlayer,
) -> GenericContainerScreenHandler {
    GenericContainerScreenHandler::new(
        WindowType::Generic3x3,
        sync_id,
        player_inventory,
        inventory,
        3,
        3,
        player.is_spectator(),
    )
}

/// Creates a crafter container (9 slots, 3x3 layout).
pub fn create_crafter_3x3(
    sync_id: u8,
    player_inventory: &Arc<PlayerInventory>,
    inventory: Arc<dyn Inventory>,
    player: &dyn InventoryPlayer,
) -> GenericContainerScreenHandler {
    GenericContainerScreenHandler::new(
        WindowType::Crafter3x3,
        sync_id,
        player_inventory,
        inventory,
        3,
        3,
        player.is_spectator(),
    )
}

/// Creates a hopper container (5 slots).
///
/// Hoppers have a single row of 5 slots.
pub fn create_hopper(
    sync_id: u8,
    player_inventory: &Arc<PlayerInventory>,
    inventory: Arc<dyn Inventory>,
    player: &dyn InventoryPlayer,
) -> GenericContainerScreenHandler {
    GenericContainerScreenHandler::new(
        WindowType::Hopper,
        sync_id,
        player_inventory,
        inventory,
        1,
        5,
        player.is_spectator(),
    )
}

/// Generic container screen handler.
///
/// Handles simple grid-based containers without special behaviors.
/// The container grid is followed by the player's inventory (27 slots + 9 hotbar).
pub struct GenericContainerScreenHandler {
    /// The container's inventory.
    pub inventory: Arc<dyn Inventory>,
    /// Number of rows in the container grid.
    pub rows: u8,
    /// Number of columns in the container grid.
    pub columns: u8,
    /// Whether the opener is in spectator mode.
    pub is_spectator: bool,
    /// Core screen handler behavior (slots, sync ID, listeners).
    behaviour: ScreenHandlerBehaviour,
}

struct ItemMove {
    slot: Arc<dyn crate::slot::Slot>,
    expected: ItemStack,
    count: u8,
}

impl GenericContainerScreenHandler {
    /// Creates a new generic container screen handler.
    ///
    /// # Arguments
    /// - `screen_type` - The window type for this container
    /// - `sync_id` - The sync ID for client-server matching
    /// - `player_inventory` - The player's inventory
    /// - `inventory` - The container's inventory
    /// - `rows` - Number of rows in the container
    /// - `columns` - Number of columns in the container
    /// - `is_spectator` - Whether the opener is a spectator
    fn new(
        screen_type: WindowType,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        inventory: Arc<dyn Inventory>,
        rows: u8,
        columns: u8,
        is_spectator: bool,
    ) -> Self {
        let mut handler = Self {
            inventory,
            rows,
            columns,
            is_spectator,
            behaviour: ScreenHandlerBehaviour::new(sync_id, Some(screen_type)),
        };

        if !is_spectator {
            handler.inventory.on_open();
        }

        handler.add_inventory_slots();
        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();
        handler.add_player_slots(&player_inventory);

        handler
    }

    /// Adds slots for the container's inventory grid.
    fn add_inventory_slots(&mut self) {
        for i in 0..self.rows {
            for j in 0..self.columns {
                self.add_slot(Arc::new(NormalSlot::new(
                    self.inventory.clone(),
                    (j + i * self.columns) as usize,
                )));
            }
        }
    }
}

impl ScreenHandler for GenericContainerScreenHandler {
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
        if !self.is_spectator && !player.is_spectator() {
            self.inventory.on_close();
        }
    }

    /// Quick move logic for generic containers.
    ///
    /// - From container: Move to player inventory (end first)
    /// - From player inventory: Move to container (start first)
    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let slot = self.get_behaviour().slots[slot_index as usize].clone();
        if !slot.has_stack() {
            return ItemStack::EMPTY.clone();
        }
        let expected = slot.get_stack();
        let source = slot.get_inventory();
        let index = slot.get_index();
        let player_inventory = self.get_behaviour().slots[self.inventory.size()].get_inventory();
        let owners = crate::inventory::slot_transaction::ordered_owners(&[
            self.inventory.as_ref(),
            player_inventory.as_ref(),
        ]);
        let transfers = owners
            .iter()
            .map(|owner| {
                owner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            })
            .collect::<Vec<_>>();
        // ChestMenu.quickMoveStack / AbstractContainerMenu.moveItemStackTo. Re-read after
        // ownership admission: another viewer may have completed a partial transfer.
        let current = source.get_stack(index);
        let expected = if current.uid == expected.uid && current.are_equal(&expected) {
            expected
        } else {
            current
        };
        let boundary = i32::from(self.rows * self.columns);
        let (start, end, reverse) = if slot_index < boundary {
            (boundary, self.get_behaviour().slots.len() as i32, true)
        } else {
            (0, boundary, false)
        };
        let moves = self.plan_item_moves(&expected, start, end, reverse);
        let amount = moves.iter().map(|planned| planned.count).sum();
        if amount == 0 {
            return ItemStack::EMPTY.clone();
        }
        let mut remaining =
            crate::inventory::slot_transaction::withdraw(source.as_ref(), index, &expected, amount);
        if remaining.is_empty() {
            return ItemStack::EMPTY.clone();
        }
        let offered = remaining.item_count;
        let changed = Self::move_item_stack_to(&mut remaining, moves);
        let moved = offered - remaining.item_count;
        let leftover = crate::inventory::slot_transaction::refund(
            source.as_ref(),
            index,
            &expected,
            remaining,
        );
        let source_changed = !source.get_stack(index).are_equal(&expected);
        drop(transfers);
        if source_changed {
            source.on_slot_changed(index);
        }
        for slot in changed {
            slot.get_inventory().on_slot_changed(slot.get_index());
        }
        if !leftover.is_empty() {
            player.drop_item(leftover, false);
        }
        if moved == 0 {
            ItemStack::EMPTY.clone()
        } else {
            expected
        }
    }
}

impl GenericContainerScreenHandler {
    // AbstractContainerMenu.moveItemStackTo: merge first, then fill one empty slot.
    // Plan before withdrawing so a failed move leaves the source visible to other readers.
    fn plan_item_moves(
        &self,
        stack: &ItemStack,
        start: i32,
        end: i32,
        reverse: bool,
    ) -> Vec<ItemMove> {
        let mut moves = Vec::new();
        let mut remaining = stack.item_count;
        for empty_only in [false, true] {
            if !empty_only && !stack.is_stackable() {
                continue;
            }
            for offset in 0..end - start {
                if remaining == 0 {
                    break;
                }
                let index = if reverse {
                    end - 1 - offset
                } else {
                    start + offset
                };
                let slot = self.get_behaviour().slots[index as usize].clone();
                let target = slot.get_stack();
                let max = slot.get_max_item_count_for_stack(stack);
                let capacity = if empty_only {
                    if target.is_empty() && slot.can_insert(stack) {
                        max
                    } else {
                        0
                    }
                } else if !target.is_empty() && target.are_items_and_components_equal(stack) {
                    max.saturating_sub(target.item_count)
                } else {
                    0
                };
                let count = capacity.min(remaining);
                if count > 0 {
                    moves.push(ItemMove {
                        slot,
                        expected: target,
                        count,
                    });
                    remaining -= count;
                    if empty_only {
                        break;
                    }
                }
            }
        }
        moves
    }

    fn move_item_stack_to(
        stack: &mut ItemStack,
        moves: Vec<ItemMove>,
    ) -> Vec<Arc<dyn crate::slot::Slot>> {
        let mut changed = Vec::new();
        for planned in moves {
            let mut moved = false;
            planned
                .slot
                .get_inventory()
                .update_slot(planned.slot.get_index(), &mut |target| {
                    if target.uid != planned.expected.uid || !target.are_equal(&planned.expected) {
                        return;
                    }
                    let count = planned.count.min(stack.item_count);
                    if target.is_empty() {
                        *target = stack.split(count);
                    } else {
                        target.increment(count);
                        stack.decrement(count);
                    }
                    moved = count > 0;
                });
            if moved {
                changed.push(planned.slot);
            }
        }
        changed
    }
}
