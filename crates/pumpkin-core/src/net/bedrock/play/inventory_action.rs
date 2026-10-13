#[allow(clippy::wildcard_imports)]
use super::*;
use crate::item::registry::should_try_block_placement;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_protocol::bedrock::server::inventory_transaction::HandSlot;

#[path = "inventory_action_hand_use.rs"]
mod hand_use;
use hand_use::{LiveHands, ReleaseUseSnapshot};
type HandAdmission = (u8, Option<ItemStack>, Option<ReleaseUseSnapshot>, LiveHands);

#[path = "inventory_action_use_item.rs"]
mod use_item;

#[path = "inventory_changes.rs"]
mod inventory_changes;

fn hand_inventory_slot(hand: Hand, selected_slot: u8) -> usize {
    match hand {
        Hand::Right => usize::from(selected_slot),
        Hand::Left => PlayerInventory::OFF_HAND_SLOT,
    }
}

impl BedrockClient {
    fn send_player_inventory_content(&self, player: &Player) {
        let slots = player
            .inventory()
            .main_inventory
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(NetworkItemStackDescriptor::from)
            .collect();
        self.try_enqueue_client_packet(&CInventoryContent {
            container_id: VarUInt(0),
            slots,
            full_container_name: FullContainerName {
                container_name: ContainerName::Inventory,
                dynamic_id: None,
            },
            storage_item: NetworkItemStackDescriptor::default(),
        });
    }

    fn respond_legacy_inventory(&self, request_id: VarInt, result: u8) {
        use pumpkin_protocol::bedrock::client::item_stack_response::{
            CItemStackResponse, ItemStackResponseInfo,
        };
        if request_id.0 != 0 {
            self.try_enqueue_client_packet(&CItemStackResponse {
                responses: vec![ItemStackResponseInfo {
                    result,
                    client_request_id: request_id,
                    containers: Vec::new(),
                }],
            });
        }
    }

    fn correct_inventory_prediction(&self, player: &Player) {
        let effects = crate::world::guard_effects::GuardEffects::new();
        let equipment = {
            let handler = player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            [
                (
                    120,
                    ContainerName::Armor,
                    (5..9)
                        .map(|slot| {
                            NetworkItemStackDescriptor::from(
                                &handler.get_slot(slot).get_cloned_stack(),
                            )
                        })
                        .collect(),
                ),
                (
                    119,
                    ContainerName::Offhand,
                    vec![NetworkItemStackDescriptor::from(
                        &handler.get_slot(45).get_cloned_stack(),
                    )],
                ),
            ]
        };
        drop(effects);
        self.send_player_inventory_content(player);
        for (window, container_name, slots) in equipment {
            self.try_enqueue_client_packet(&CInventoryContent {
                container_id: VarUInt(window),
                slots,
                full_container_name: FullContainerName {
                    container_name,
                    dynamic_id: None,
                },
                storage_item: NetworkItemStackDescriptor::default(),
            });
        }
    }

    fn correct_rejected_food_use(&self, player: &Player) {
        // Holding use can repeat rejected transactions. Limit prediction corrections
        // to every 20 ticks, without indefinitely trusting a previously sent snapshot.
        const CORRECTION_INTERVAL_TICKS: i32 = 20;
        let tick = player.tick_counter.load(Ordering::Relaxed);
        let recently_corrected = self.last_food_rejection_tick.load().is_some_and(|last| {
            tick >= last && tick.saturating_sub(last) < CORRECTION_INTERVAL_TICKS
        });
        let has_active_use = player
            .living_entity
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if recently_corrected && !has_active_use {
            return;
        }
        if player.has_client_loaded() {
            self.last_food_rejection_tick.store(Some(tick));
        }
        player.living_entity.clear_active_hand();
        player.send_health();
    }

    fn validated_use_item_stack(
        player: &Player,
        hand: Hand,
        client_descriptor: &NetworkItemDescriptor,
    ) -> Option<ItemStack> {
        // ServerGamePacketListenerImpl.handleUseItemOn/handleUseItem use the server's held stack.
        // Validate before predicted actions can empty or transform the originating item.
        let held = player.inventory().get_stack_in_hand(hand);
        // Several Java items share a Bedrock ID, including sticks and debug sticks.
        let authoritative_descriptor = NetworkItemDescriptor::from(&held);
        if client_descriptor.id != authoritative_descriptor.id
            || client_descriptor.aux_value != authoritative_descriptor.aux_value
        {
            return None;
        }
        Some(held)
    }

    pub fn handle_inventory_action(&self, player: &Arc<Player>, packet: SInventoryTransaction) {
        tracing::debug!("handle_inventory_action: packet={:?}", packet);
        let request_id = packet.legacy_request_id;
        let before = inventory_changes::InventorySnapshot::capture(player);
        let correct_prediction = !packet.actions.is_empty();
        Self::drop_legacy_inventory_slots(player, &packet);
        // ServerGamePacketListenerImpl.handleContainerClick treats client stacks as predictions.
        // handleUseItemOn/handleUseItem still execute against the server's hand first.
        let mut rejected_hand_prediction = false;
        let accepted =
            self.process_inventory_transaction(player, packet, &mut rejected_hand_prediction);
        self.publish_inventory_changes(
            player,
            &before,
            correct_prediction || (!accepted && rejected_hand_prediction),
        );
        // Bedrock ItemStackResponseInfo: 0 = SUCCESS, 1 = ERROR. Discarding a
        // prediction does not reject an otherwise processed authoritative use.
        self.respond_legacy_inventory(request_id, u8::from(!accepted));
    }

    fn drop_legacy_inventory_slots(player: &Player, packet: &SInventoryTransaction) {
        let mut dropped_items = Vec::new();
        let effects = crate::world::guard_effects::GuardEffects::new();
        if packet.actions.is_empty()
            && packet.legacy_request_id.0 != 0
            && !packet.legacy_set_item_slots.is_empty()
        {
            let mut player_screen_handler = player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for legacy_slot in &packet.legacy_set_item_slots {
                let mapped_window_id = match legacy_slot.container_id {
                    28 | 29 => 0,    // HotBar or Inventory
                    6 | 120 => 120,  // Armor
                    34 | 119 => 119, // Offhand
                    other => other as i32,
                };
                for &slot_id in &legacy_slot.slots {
                    if let Some(screen_slot) =
                        map_bedrock_slot_to_screen_handler(mapped_window_id, slot_id as u32)
                    {
                        let current_stack = player_screen_handler
                            .get_slot(screen_slot)
                            .get_cloned_stack();
                        if !current_stack.is_empty() {
                            let taken = player_screen_handler
                                .get_slot(screen_slot)
                                .safe_take_matching(
                                    &current_stack,
                                    current_stack.item_count,
                                    u8::MAX,
                                    player,
                                );
                            if taken.is_empty() {
                                continue;
                            }
                            dropped_items.push(taken);
                            player_screen_handler
                                .set_received_stack(screen_slot, ItemStack::EMPTY.clone());
                        }
                    }
                }
            }
            player_screen_handler.send_content_updates();
            drop(player_screen_handler);
        }

        drop(effects);
        for item in dropped_items {
            player.drop_item(item);
        }
    }

    fn admit_inventory_transaction(
        &self,
        player: &Arc<Player>,
        packet: &SInventoryTransaction,
        rejected_hand_prediction: &mut bool,
    ) -> Option<HandAdmission> {
        let mut source_selected_slot = player.inventory().get_selected_slot();
        let mut release_use = None;
        let hands_before_use = LiveHands::read(player);
        let use_item_stack = match &packet.transaction_data {
            TransactionData::UseItem(data) if matches!(data.action_type.0, 0 | 1) => {
                let Some(selected) = self.reconcile_use_item_selection(
                    player,
                    data.hot_bar_slot.0,
                    &hands_before_use,
                ) else {
                    *rejected_hand_prediction = true;
                    self.correct_rejected_block_use(player, data);
                    return None;
                };
                source_selected_slot = selected;
                let hand = match data.hand {
                    HandSlot::Mainhand => Hand::Right,
                    HandSlot::Offhand => Hand::Left,
                };
                let Some(stack) = Self::validated_use_item_stack(player, hand, &data.item_in_hand)
                else {
                    *rejected_hand_prediction = true;
                    self.correct_rejected_block_use(player, data);
                    return None;
                };
                Some(stack)
            }
            TransactionData::UseItemOnEntity(data) => {
                let Some(selected) = self.reconcile_use_item_selection(
                    player,
                    data.hot_bar_slot.0,
                    &hands_before_use,
                ) else {
                    *rejected_hand_prediction = true;
                    return None;
                };
                source_selected_slot = selected;
                let Some(stack) =
                    Self::validated_use_item_stack(player, Hand::Right, &data.item_in_hand)
                else {
                    *rejected_hand_prediction = true;
                    return None;
                };
                Some(stack)
            }
            TransactionData::ReleaseItem(_) => {
                let snapshot = ReleaseUseSnapshot::read(player);
                let hand = snapshot.hand.unwrap_or(Hand::Right);
                release_use = Some(snapshot);
                Some(player.inventory().get_stack_in_hand(hand))
            }
            _ => None,
        };
        Some((
            source_selected_slot,
            use_item_stack,
            release_use,
            hands_before_use,
        ))
    }

    fn process_inventory_transaction(
        &self,
        player: &Arc<Player>,
        packet: SInventoryTransaction,
        rejected_hand_prediction: &mut bool,
    ) -> bool {
        let Some((source_selected_slot, use_item_stack, release_use, hands_before_use)) =
            self.admit_inventory_transaction(player, &packet, rejected_hand_prediction)
        else {
            return false;
        };
        match packet.transaction_data {
            TransactionData::Normal(_) | TransactionData::Mismatch(_) => {
                return packet.actions.is_empty();
            }
            TransactionData::UseItem(data) => {
                return self.process_admitted_item(
                    player,
                    &data,
                    source_selected_slot,
                    use_item_stack,
                    &hands_before_use,
                );
            }
            TransactionData::UseItemOnEntity(data) => {
                if let Some(before) = use_item_stack {
                    let accepted = Self::interact_entity_with_admitted_hand(
                        player,
                        &data,
                        source_selected_slot,
                        &before,
                    );
                    Self::sync_live_hands(
                        player,
                        Hand::Right,
                        source_selected_slot,
                        &hands_before_use,
                    );
                    return accepted;
                }
            }
            TransactionData::ReleaseItem(_) => {
                if let (Some(before), Some(release_use)) = (use_item_stack, release_use) {
                    let hand = release_use.hand.unwrap_or(Hand::Right);
                    Self::release_admitted_hand(
                        player,
                        source_selected_slot,
                        &before,
                        &release_use,
                    );
                    Self::sync_live_hands(player, hand, source_selected_slot, &hands_before_use);
                    if release_use.hand.is_none() {
                        Self::sync_live_hands(
                            player,
                            Hand::Left,
                            source_selected_slot,
                            &hands_before_use,
                        );
                    }
                }
            }
        }

        true
    }
}

#[cfg(test)]
#[path = "inventory_action_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "inventory_action_prediction_tests.rs"]
mod prediction_tests;
