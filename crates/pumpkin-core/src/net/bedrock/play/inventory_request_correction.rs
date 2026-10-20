use super::{
    BedrockClient, CInventoryContent, ContainerName, FullContainerName, ItemStack,
    NetworkItemStackDescriptor, Player, PlayerInventory, VarUInt,
};
use crate::entity::EntityBase;
use pumpkin_inventory::{
    Inventory,
    inventory::slot_transaction,
    screen_handler::{ScreenHandler, SharedScreenHandler},
};
use pumpkin_protocol::bedrock::client::inventory_slot::CInventorySlot;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum CorrectionTarget {
    Inventory,
    Armor,
    Offhand,
    MenuSlot(usize),
    MenuContent,
    Cursor,
}

struct CorrectionSnapshot {
    window: u32,
    name: ContainerName,
    slot: Option<u32>,
    stacks: Vec<ItemStack>,
}

impl CorrectionSnapshot {
    fn matches(&self, other: &Self) -> bool {
        self.window == other.window
            && self.name == other.name
            && self.slot == other.slot
            && self.stacks.len() == other.stacks.len()
            && self
                .stacks
                .iter()
                .zip(&other.stacks)
                .all(|(a, b)| a.uid == b.uid && a.are_equal(b))
    }

    fn send(&self, client: &BedrockClient) {
        let name = FullContainerName {
            container_name: self.name,
            dynamic_id: None,
        };
        if let Some(slot) = self.slot {
            client.try_enqueue_client_packet(&CInventorySlot {
                container_id: VarUInt(self.window),
                slot: VarUInt(slot),
                full_container_name: Some(name),
                storage_item: None,
                item: NetworkItemStackDescriptor::from(&self.stacks[0]),
            });
        } else {
            client.try_enqueue_client_packet(&CInventoryContent {
                container_id: VarUInt(self.window),
                slots: self
                    .stacks
                    .iter()
                    .map(NetworkItemStackDescriptor::from)
                    .collect(),
                full_container_name: name,
                storage_item: NetworkItemStackDescriptor::default(),
            });
        }
    }
}

fn is_current(player: &Player, menu: &SharedScreenHandler) -> bool {
    !player.client.closed()
        && !player.get_entity().is_removed()
        && player
            .world()
            .get_player_by_uuid(player.gameprofile.id)
            .is_some_and(|live| std::ptr::eq(player, live.as_ref()))
        && Arc::ptr_eq(
            menu,
            &player
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
}

impl CorrectionTarget {
    fn capture(
        self,
        player: &Player,
        menu: &SharedScreenHandler,
        storage_only: bool,
    ) -> Option<CorrectionSnapshot> {
        let current = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Arc::ptr_eq(menu, &current) {
            return None;
        }
        let handler = menu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut inventories = handler
            .get_behaviour()
            .slots
            .iter()
            .map(|slot| slot.get_inventory())
            .collect::<Vec<_>>();
        inventories.push(player.inventory().clone());
        let refs = inventories.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        let owners = slot_transaction::ordered_owners(&refs);
        let _guards = slot_transaction::lock_owners(&owners);
        if !owners.iter().all(|owner| owner.is_live()) {
            return None;
        }
        self.snapshot(player, &*handler, storage_only)
    }

    // The caller pins the active menu and owns every backing inventory.
    fn snapshot(
        self,
        player: &Player,
        handler: &dyn ScreenHandler,
        storage_only: bool,
    ) -> Option<CorrectionSnapshot> {
        let slots = &handler.get_behaviour().slots;
        let inventory = player.inventory();
        let read = |index: usize| {
            let slot = &slots[index];
            if storage_only {
                // Revalidation must not invoke slot callbacks that can invalidate its own read.
                slot.get_inventory().get_stack(slot.get_index())
            } else {
                slot.get_cloned_stack()
            }
        };
        let (window, name, slot, stacks) = match self {
            Self::Inventory => (
                0,
                ContainerName::Inventory,
                None,
                (0..PlayerInventory::MAIN_SIZE)
                    .map(|index| inventory.get_stack(index))
                    .collect(),
            ),
            Self::Armor => (
                120,
                ContainerName::Armor,
                None,
                (36..40)
                    .rev()
                    .map(|index| inventory.get_stack(index))
                    .collect(),
            ),
            Self::Offhand => (
                119,
                ContainerName::Offhand,
                None,
                vec![inventory.get_stack(PlayerInventory::OFF_HAND_SLOT)],
            ),
            Self::Cursor => (
                59,
                ContainerName::Cursor,
                None,
                vec![
                    handler
                        .get_behaviour()
                        .cursor_stack
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone(),
                ],
            ),
            Self::MenuContent => (
                u32::from(handler.sync_id()),
                ContainerName::LevelEntity,
                None,
                (0..slots.len().saturating_sub(PlayerInventory::MAIN_SIZE))
                    .map(read)
                    .collect(),
            ),
            Self::MenuSlot(index) => {
                slots.get(index)?;
                // Same crafting addresses as stack responses and Player's Bedrock encoder.
                let (name, slot) = if index == 0 {
                    (ContainerName::CreatedOutput, 0)
                } else {
                    let start = if handler.window_type().is_none() {
                        28
                    } else {
                        32
                    };
                    (ContainerName::CraftingInput, start + index as u32 - 1)
                };
                (
                    u32::from(handler.sync_id()),
                    name,
                    Some(slot),
                    vec![read(index)],
                )
            }
        };
        Some(CorrectionSnapshot {
            window,
            name,
            slot,
            stacks,
        })
    }
}

impl BedrockClient {
    pub(super) fn correct_stack_request_prediction(&self, player: &Player) {
        let menu = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut targets = vec![
            CorrectionTarget::Inventory,
            CorrectionTarget::Armor,
            CorrectionTarget::Offhand,
        ];
        {
            let handler = menu
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match handler.window_type() {
                None => targets.extend((0..5).map(CorrectionTarget::MenuSlot)),
                Some(pumpkin_data::screen::WindowType::Crafting) => {
                    targets.extend((0..10).map(CorrectionTarget::MenuSlot));
                }
                Some(_) => targets.push(CorrectionTarget::MenuContent),
            }
        }
        targets.push(CorrectionTarget::Cursor);
        // AbstractContainerMenu.sendAllDataToRemote forces unchanged slots and carried stock.
        // Capture afresh after each publication; callbacks can replace stock or the menu.
        for target in targets {
            if !is_current(player, &menu) {
                return;
            }
            let effects = crate::world::guard_effects::GuardEffects::new();
            let captured = target.capture(player, &menu, false);
            drop(effects);
            if !is_current(player, &menu) {
                return;
            }
            let Some(mut captured) = captured else {
                return;
            };
            let Some(current) = target.capture(player, &menu, true) else {
                return;
            };
            if !captured.matches(&current) {
                // A callback changed this same target; publish its current backing stock.
                captured = current;
            }
            if is_current(player, &menu) {
                captured.send(self);
            }
        }
    }
}
