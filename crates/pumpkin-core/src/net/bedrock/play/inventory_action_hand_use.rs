use super::{
    ActionType, BedrockClient, EquipmentSlot, Hand, ItemStack, Player, PlayerInteractEntityEvent,
    PlayerInventory, hand_inventory_slot,
};
use crate::net::java::play::hand_use_result::{HandMutation, write_back_hand_item};
use pumpkin_data::BlockDirection;
use pumpkin_inventory::Inventory;
use pumpkin_protocol::bedrock::{
    client::{player_hotbar::CPlayerHotbar, update_block::CUpdateBlock},
    server::inventory_transaction::UseItemOnEntityTransactionData,
};
use pumpkin_protocol::codec::var_uint::VarUInt;
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

pub(super) struct LiveHands {
    selected: u8,
    main: ItemStack,
    off: ItemStack,
}

pub(super) struct ReleaseUseSnapshot {
    pub(super) hand: Option<Hand>,
    stack: Option<ItemStack>,
    remaining: i32,
}

impl ReleaseUseSnapshot {
    pub(super) fn read(player: &Player) -> Self {
        let hand = *player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stack = player
            .living_entity
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let remaining = player
            .living_entity
            .item_use_time
            .load(std::sync::atomic::Ordering::Relaxed);
        Self {
            hand,
            stack,
            remaining,
        }
    }

    fn is_current(&self, player: &Player) -> bool {
        let current = Self::read(player);
        let same_item = match (&self.stack, &current.stack) {
            (Some(before), Some(after)) => same_stack(before, after),
            (None, None) => true,
            _ => false,
        };
        // LivingEntity.releaseUsingItem stops the original use, never a callback's newer use.
        current.hand == self.hand && same_item && current.remaining <= self.remaining
    }
}

fn same_stack(left: &ItemStack, right: &ItemStack) -> bool {
    left.uid == right.uid && left.are_equal(right)
}

impl LiveHands {
    pub(super) fn read(player: &Player) -> Self {
        let selected = player.inventory().get_selected_slot();
        Self {
            selected,
            main: player.inventory().get_slot(usize::from(selected)),
            off: player.inventory().get_slot(PlayerInventory::OFF_HAND_SLOT),
        }
    }
}

impl BedrockClient {
    pub(super) fn reconcile_use_item_selection(
        &self,
        player: &Arc<Player>,
        hot_bar_slot: i32,
        before: &LiveHands,
    ) -> Option<u8> {
        let requested = usize::try_from(hot_bar_slot)
            .ok()
            .filter(|slot| PlayerInventory::is_valid_hotbar_index(*slot));
        let Some(requested) = requested else {
            self.correct_use_item_selection(player, before);
            return None;
        };
        let requested = requested as u8;
        if player.inventory().get_selected_slot() != requested {
            let Some(server) = player.world().server.upgrade() else {
                self.correct_use_item_selection(player, before);
                return None;
            };
            // Geyser BedrockInventoryTransactionTranslator.translate switches before use admission.
            if !self.select_hotbar_slot(&server, player, requested) {
                self.correct_use_item_selection(player, before);
                return None;
            }
            self.send_live_hotbar_selection(player);
            if player.inventory().get_selected_slot() != requested {
                self.correct_use_item_selection(player, before);
                return None;
            }
        }
        Some(requested)
    }

    /// Publishes the current selection without retaining a guard across the send.
    pub(crate) fn send_live_hotbar_selection(&self, player: &Player) {
        self.try_enqueue_client_packet(&CPlayerHotbar {
            selected_slot: VarUInt(u32::from(player.inventory().get_selected_slot())),
            container_id: 0,
            should_select_slot: true,
        });
    }

    fn correct_use_item_selection(&self, player: &Player, before: &LiveHands) {
        // The packet entry emits the single complete inventory correction after rejection.
        Self::sync_live_hands(
            player,
            Hand::Right,
            player.inventory().get_selected_slot(),
            before,
        );
        self.send_live_hotbar_selection(player);
    }

    pub(super) fn correct_rejected_block_use(
        &self,
        player: &Player,
        data: &pumpkin_protocol::bedrock::server::inventory_transaction::UseItemTransactionData,
    ) {
        if data.action_type.0 == 0 {
            let face =
                BlockDirection::try_from(i32::from(data.block_face)).unwrap_or(BlockDirection::Up);
            self.correct_denied_placement(player, data.block_position, face);
        }
    }

    pub(super) fn correct_denied_placement(
        &self,
        player: &Player,
        clicked: BlockPos,
        face: BlockDirection,
    ) {
        // ServerGamePacketListenerImpl.handleUseItemOn corrects pos and pos.relative(direction).
        let world = player.world();
        let offset = face.to_offset();
        // BlockPos.relative uses Java's wrapping int addition, even for rejected positions.
        let adjacent = BlockPos::new(
            clicked.0.x.wrapping_add(offset.x),
            clicked.0.y.wrapping_add(offset.y),
            clicked.0.z.wrapping_add(offset.z),
        );
        for position in [clicked, adjacent] {
            let state = world.get_block_state_id(&position);
            self.try_enqueue_client_packet(&CUpdateBlock::new(
                position,
                pumpkin_data::BlockState::to_be_network_id(state),
            ));
        }
    }

    /// Commits the server's block-use result before publishing the live hands.
    pub(super) fn finish_block_use(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
        after: &ItemStack,
        hands_before: &LiveHands,
    ) {
        // ServerPlayerGameMode.useItemOn keeps the source stack; callback replacements win.
        write_back_hand_item(
            player,
            hand,
            hand_inventory_slot(hand, selected),
            before,
            after,
            HandMutation::ItemUse,
        );
        Self::sync_live_hands(player, hand, selected, hands_before);
    }

    fn admitted_hand_stack(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
    ) -> Option<ItemStack> {
        // ServerPlayerGameMode.useItem/useItemOn receives one server-held stack at admission.
        let live = player
            .inventory()
            .get_slot(hand_inventory_slot(hand, selected));
        let selection_matches =
            hand == Hand::Left || player.inventory().get_selected_slot() == selected;
        (selection_matches && same_stack(&live, before)).then_some(live)
    }

    /// Admits the pinned stack; only main-hand use requires the original hotbar selection.
    pub(super) fn stack_for_hand_use(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
        hands_before: &LiveHands,
    ) -> Option<ItemStack> {
        let live = Self::admitted_hand_stack(player, hand, selected, before);
        if live.is_none() {
            Self::sync_live_hands(player, hand, selected, hands_before);
        }
        live
    }

    /// Queues changed live hands for the baseline tick publisher.
    pub(super) fn sync_live_hands(player: &Player, _hand: Hand, _selected: u8, before: &LiveHands) {
        // LivingEntity.detectEquipmentUpdates reads final live slots; no snapshot is installed.
        let live = LiveHands::read(player);
        if live.selected != before.selected || !same_stack(&live.main, &before.main) {
            player.queue_equipment_updates(&[EquipmentSlot::MAIN_HAND]);
        }
        if !same_stack(&live.off, &before.off) {
            player.queue_equipment_updates(&[EquipmentSlot::OFF_HAND]);
        }
    }

    /// Runs entity interaction on the admitted server hand; the caller publishes once afterwards.
    pub(super) fn interact_entity_with_admitted_hand(
        player: &Arc<Player>,
        data: &UseItemOnEntityTransactionData,
        selected: u8,
        before: &ItemStack,
    ) -> bool {
        use crate::entity::EntityBase;
        let action = match data.action_type.0 {
            // Bedrock ItemInteract has no distinct hit-location action.
            0 | 2 => ActionType::Interact,
            1 => ActionType::Attack,
            action => {
                tracing::warn!("invalid UseItemOnEntity action type {action}");
                return false;
            }
        };
        let Ok(target_id) = i32::try_from(data.target_entity_runtime_id.0) else {
            tracing::warn!(
                "invalid UseItemOnEntity target runtime ID {}",
                data.target_entity_runtime_id.0
            );
            return false;
        };
        let world = player.world();
        let Some(target) = world.get_entity_by_id(target_id) else {
            return false;
        };
        let Some(server) = world.server.upgrade() else {
            return false;
        };
        if player.is_spectator() {
            return false;
        }
        let mut event = PlayerInteractEntityEvent::new(
            player,
            target,
            action,
            None,
            player.get_entity().is_sneaking(),
        );
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return false;
        }
        match event.action {
            ActionType::Interact | ActionType::InteractAt => {
                // ServerGamePacketListenerImpl.handleInteract / Player.interactOn retain the server tool.
                let Some(mut stack) =
                    Self::admitted_hand_stack(player, Hand::Right, selected, before)
                else {
                    return false;
                };
                let item_id = stack.item.id;
                let (interacted, committed) = crate::entity::mob::interaction::with_hand_commit(
                    player,
                    usize::from(selected),
                    before,
                    || {
                        event
                            .target
                            .interact_with_hand(player, &mut stack, Hand::Right)
                    },
                );
                let stand_swap = interacted
                    && event
                        .target
                        .cast_any()
                        .is::<crate::entity::decoration::armor_stand::ArmorStandEntity>();
                if !interacted {
                    server
                        .item_registry
                        .use_on_entity(&mut stack, player, event.target);
                }
                if committed.unwrap_or_else(|| !stack.are_equal(before)) && !stand_swap {
                    player.increment_stat(
                        pumpkin_data::statistic::StatisticCategory::Used,
                        i32::from(item_id),
                        1,
                    );
                }
                if committed.is_none() {
                    write_back_hand_item(
                        player,
                        Hand::Right,
                        usize::from(selected),
                        before,
                        &stack,
                        if stand_swap {
                            HandMutation::EquipmentTransfer
                        } else {
                            HandMutation::ItemUse
                        },
                    );
                }
            }
            ActionType::Attack => player.attack(&event.target),
        }
        true
    }

    /// Releases the admitted use and preserves a newer use started by a callback.
    pub(super) fn release_admitted_hand(
        player: &Player,
        selected: u8,
        before: &ItemStack,
        use_before: &ReleaseUseSnapshot,
    ) {
        // LivingEntity.releaseUsingItem checks the live used hand before releaseUsing, then stops use.
        if !use_before.is_current(player) {
            return;
        }
        if let Some(hand) = use_before.hand
            && let Some(live) = Self::admitted_hand_stack(player, hand, selected, before)
            && use_before
                .stack
                .as_ref()
                .is_some_and(|stack| stack.item == live.item)
            && let Some(server) = player.world().server.upgrade()
        {
            server.item_registry.on_stopped_using(&live, player);
        }
        if use_before.is_current(player) {
            player.living_entity.clear_active_hand();
        }
    }

    /// Commits an equipment swap before any announcement can run a plugin callback.
    pub(super) fn equip_admitted_hand(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
        slot: &EquipmentSlot,
    ) -> Option<(ItemStack, ItemStack, ItemStack)> {
        // Equippable.swapWithEquipmentSlot sets the real slot before LivingEntity.detectEquipmentUpdates.
        let inventory = player.inventory();
        let gamemode = player.gamemode.load();
        // Both slots share PlayerInventory's existing transaction owner, including saves/transfers.
        let owner = inventory.slot_transaction(0);
        let _transaction = owner.lock();
        if !owner.is_live() {
            return None;
        }
        // PlayerInventory.swap_item uses this same main-inventory -> equipment order.
        let mut main = (hand == Hand::Right).then(|| {
            inventory
                .main_inventory
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        let mut equipment = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = main.as_ref().map_or_else(
            || equipment.get(&EquipmentSlot::OFF_HAND),
            |main| main[usize::from(selected)].clone(),
        );
        if (hand == Hand::Right
            && inventory
                .selected_slot
                .load(std::sync::atomic::Ordering::Relaxed)
                != selected)
            || !same_stack(&current, before)
        {
            return None;
        }
        let displaced = equipment.get(slot);
        // Equippable.swapWithEquipmentSlot consumes one from a multi-item source (Java line 145).
        let (held, installed, displaced_to_inventory) = if current.item_count > 1 {
            let mut remainder = current.clone();
            remainder.decrement_unless_creative(gamemode, 1);
            (remainder, current.copy_with_count(1), displaced)
        } else {
            let held = if gamemode == pumpkin_util::GameMode::Creative && displaced.is_empty() {
                current.clone()
            } else {
                displaced
            };
            (held, current, ItemStack::EMPTY.clone())
        };
        equipment.put(slot, installed.clone());
        let stored = if held.is_empty() {
            ItemStack::EMPTY.clone()
        } else {
            held
        };
        if let Some(main) = &mut main {
            main[usize::from(selected)] = stored.clone();
        } else {
            equipment.put(&EquipmentSlot::OFF_HAND, stored.clone());
        }
        Some((stored, installed, displaced_to_inventory))
    }
}
