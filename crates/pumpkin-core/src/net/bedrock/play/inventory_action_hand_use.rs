use super::{
    ActionType, BedrockClient, CSetPlayerInventory, EquipmentSlot, Hand, ItemStack,
    ItemStackSerializer, Player, PlayerInteractEntityEvent, PlayerInventory, hand_inventory_slot,
    hand_screen_handler_slot,
};
use crate::net::java::play::hand_use_result::{HandMutation, write_back_hand_item};
use pumpkin_data::BlockDirection;
use pumpkin_inventory::screen_handler::ScreenHandler;
use pumpkin_protocol::bedrock::{
    client::{player_hotbar::CPlayerHotbar, update_block::CUpdateBlock},
    server::inventory_transaction::{HandSlot, UseItemOnEntityTransactionData},
};
use pumpkin_protocol::codec::var_uint::VarUInt;
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

pub(super) struct LiveHands {
    selected: u8,
    main: ItemStack,
    off: ItemStack,
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
        hand: HandSlot,
    ) -> Option<u8> {
        let requested = usize::try_from(hot_bar_slot)
            .ok()
            .filter(|slot| PlayerInventory::is_valid_hotbar_index(*slot));
        let Some(requested) = requested else {
            self.correct_use_item_selection(player, hand);
            return None;
        };
        let requested = requested as u8;
        if player.inventory().get_selected_slot() != requested {
            let Some(server) = player.world().server.upgrade() else {
                self.correct_use_item_selection(player, hand);
                return None;
            };
            // Geyser BedrockInventoryTransactionTranslator.translate switches before use admission.
            if !self.select_hotbar_slot(&server, player, requested) {
                self.correct_use_item_selection(player, hand);
                return None;
            }
            self.send_live_hotbar_selection(player);
            if player.inventory().get_selected_slot() != requested {
                self.correct_use_item_selection(player, hand);
                return None;
            }
        }
        Some(requested)
    }

    fn send_live_hotbar_selection(&self, player: &Player) {
        self.try_enqueue_client_packet(&CPlayerHotbar {
            selected_slot: VarUInt(u32::from(player.inventory().get_selected_slot())),
            container_id: 0,
            should_select_slot: true,
        });
    }

    fn correct_use_item_selection(&self, player: &Player, hand: HandSlot) {
        self.send_player_inventory_content(player);
        let before = LiveHands::read(player);
        Self::sync_live_hands(
            player,
            Hand::Right,
            player.inventory().get_selected_slot(),
            &before,
        );
        if hand == HandSlot::Offhand {
            Self::sync_live_hands(
                player,
                Hand::Left,
                player.inventory().get_selected_slot(),
                &before,
            );
        }
        self.send_live_hotbar_selection(player);
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

    fn sync_hand_inventory_slot(player: &Player, hand: Hand, selected: u8) {
        let inventory_slot = hand_inventory_slot(hand, selected);
        let stack = player.inventory().get_slot(inventory_slot);
        if let Some(screen_slot) = hand_screen_handler_slot(hand, selected) {
            player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_received_stack(screen_slot, stack.clone());
        }
        player.try_send_slot_set_packet(&CSetPlayerInventory::new(
            (inventory_slot as i32).into(),
            &ItemStackSerializer::from(stack),
        ));
        player.sync_inventory_to_client();
    }

    /// Corrects the originating slot and publishes current hands with no guard across a send.
    pub(super) fn sync_live_hands(player: &Player, hand: Hand, selected: u8, before: &LiveHands) {
        // LivingEntity.detectEquipmentUpdates/collectEquipmentChanges reads current equipment.
        // Always correct the origin: even denied use can disagree with client prediction.
        let origin = player
            .inventory()
            .get_slot(hand_inventory_slot(hand, selected));
        Self::sync_hand_inventory_slot(player, hand, selected);
        let published = LiveHands::read(player);
        let mut equipment = Vec::new();
        let main_changed =
            published.selected != before.selected || !same_stack(&published.main, &before.main);
        let off_changed = !same_stack(&published.off, &before.off);
        if hand == Hand::Right || published.selected != selected || main_changed {
            if hand != Hand::Right || published.selected != selected {
                Self::sync_hand_inventory_slot(player, Hand::Right, published.selected);
            }
            equipment.push((EquipmentSlot::MAIN_HAND, published.main.clone()));
        }
        if hand == Hand::Left || off_changed {
            if hand != Hand::Left {
                Self::sync_hand_inventory_slot(player, Hand::Left, published.selected);
            }
            equipment.push((EquipmentSlot::OFF_HAND, published.off.clone()));
        }
        player.living_entity.send_equipment_changes(&equipment);

        // A PacketSentEvent can replace a stack or select another slot during publication.
        // Re-read both hands after that boundary, like the next vanilla equipment detection.
        let current = LiveHands::read(player);
        let main_changed =
            current.selected != published.selected || !same_stack(&current.main, &published.main);
        let off_changed = !same_stack(&current.off, &published.off);
        if !main_changed && !off_changed {
            return;
        }
        equipment.clear();
        if main_changed {
            if hand != Hand::Right
                || current.selected != selected
                || !same_stack(&current.main, &origin)
            {
                Self::sync_hand_inventory_slot(player, Hand::Right, current.selected);
            }
            equipment.push((EquipmentSlot::MAIN_HAND, current.main));
        }
        if off_changed {
            Self::sync_hand_inventory_slot(player, Hand::Left, current.selected);
            equipment.push((EquipmentSlot::OFF_HAND, current.off));
        }
        player.living_entity.send_equipment_changes(&equipment);
    }

    /// Runs entity interaction on the admitted server hand; the caller publishes once afterwards.
    pub(super) fn interact_entity_with_admitted_hand(
        player: &Arc<Player>,
        data: &UseItemOnEntityTransactionData,
        selected: u8,
        before: &ItemStack,
    ) {
        use crate::entity::EntityBase;
        let action = match data.action_type.0 {
            // Bedrock ItemInteract has no distinct hit-location action.
            0 | 2 => ActionType::Interact,
            1 => ActionType::Attack,
            action => {
                tracing::warn!("invalid UseItemOnEntity action type {action}");
                return;
            }
        };
        let Ok(target_id) = i32::try_from(data.target_entity_runtime_id.0) else {
            tracing::warn!(
                "invalid UseItemOnEntity target runtime ID {}",
                data.target_entity_runtime_id.0
            );
            return;
        };
        let world = player.world();
        let Some(target) = world.get_entity_by_id(target_id) else {
            return;
        };
        let Some(server) = world.server.upgrade() else {
            return;
        };
        if player.is_spectator() {
            return;
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
            return;
        }
        match event.action {
            ActionType::Interact | ActionType::InteractAt => {
                // ServerGamePacketListenerImpl.handleInteract / Player.interactOn retain the server tool.
                let Some(mut stack) =
                    Self::admitted_hand_stack(player, Hand::Right, selected, before)
                else {
                    return;
                };
                let item_id = stack.item.id;
                let interacted = event
                    .target
                    .interact_with_hand(player, &mut stack, Hand::Right);
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
                if !stack.are_equal(before) && !stand_swap {
                    player.increment_stat(
                        pumpkin_data::statistic::StatisticCategory::Used,
                        i32::from(item_id),
                        1,
                    );
                }
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
            ActionType::Attack => player.attack(&event.target),
        }
    }

    /// Releases the current active hand, ignoring any client-predicted hand write.
    pub(super) fn release_admitted_hand(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
    ) {
        // LivingEntity.releaseUsingItem checks the live used hand before releaseUsing, then stops use.
        let active = player
            .living_entity
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let active_hand = *player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active_hand.is_some_and(|current| current != hand) {
            return;
        }
        if active_hand == Some(hand)
            && let Some(live) = Self::admitted_hand_stack(player, hand, selected, before)
            && active.is_some_and(|stack| stack.item == live.item)
            && let Some(server) = player.world().server.upgrade()
        {
            server.item_registry.on_stopped_using(&live, player);
        }
        player.living_entity.clear_active_hand();
    }

    /// Commits an equipment swap before any announcement can run a plugin callback.
    pub(super) fn equip_admitted_hand(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
        slot: &EquipmentSlot,
    ) -> Option<(ItemStack, ItemStack)> {
        // Equippable.swapWithEquipmentSlot sets the real slot before LivingEntity.detectEquipmentUpdates.
        let inventory = player.inventory();
        let gamemode = player.gamemode.load();
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
        let mut held = equipment.get(slot);
        if held.is_empty() {
            held = current.clone();
            held.decrement_unless_creative(gamemode, 1);
        }
        equipment.put(slot, current.clone());
        let stored = if held.is_empty() {
            ItemStack::EMPTY.clone()
        } else {
            held.clone()
        };
        if let Some(main) = &mut main {
            main[usize::from(selected)] = stored.clone();
        } else {
            equipment.put(&EquipmentSlot::OFF_HAND, stored.clone());
        }
        Some((stored, current))
    }
}
