#[allow(clippy::wildcard_imports)]
use super::*;
use crate::item::registry::should_try_block_placement;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_protocol::bedrock::server::inventory_transaction::{
    HandSlot, WINDOW_ID_INVENTORY, WINDOW_ID_OFF_HAND,
};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer, java::client::play::CSetPlayerInventory,
};

// InventoryMenu.USE_ROW_SLOT_START and SHIELD_SLOT delimit the hand screen slots.
const USE_ROW_SLOT_START: usize = 36;
const SHIELD_SLOT: usize = 45;

#[path = "inventory_action_hand_use.rs"]
mod hand_use;
use hand_use::LiveHands;

// ServerGamePacketListenerImpl.handleUseItemOn selects the hand passed to useItemOn.
fn hand_screen_handler_slot(hand: Hand, selected_slot: u8) -> Option<usize> {
    match hand {
        Hand::Right => {
            map_bedrock_slot_to_screen_handler(WINDOW_ID_INVENTORY, u32::from(selected_slot))
        }
        Hand::Left => map_bedrock_slot_to_screen_handler(WINDOW_ID_OFF_HAND, 0),
    }
}

fn hand_inventory_slot(hand: Hand, selected_slot: u8) -> usize {
    match hand {
        Hand::Right => usize::from(selected_slot),
        Hand::Left => PlayerInventory::OFF_HAND_SLOT,
    }
}

impl BedrockClient {
    fn apply_inventory_action(
        player: &Player,
        screen_slot: usize,
        item_stack: ItemStack,
    ) -> (bool, Option<(EquipmentSlot, ItemStack)>) {
        let _effects = crate::world::guard_effects::GuardEffects::new();
        let changed = {
            let mut screen_handler = player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = !screen_handler
                .get_slot(screen_slot)
                .get_cloned_stack()
                .are_equal(&item_stack);
            // ServerPlayerGameMode.useItemOn retains the original stack reference.
            // An equal client prediction must not replace that stack's UID.
            if changed {
                screen_handler
                    .get_slot(screen_slot)
                    .set_stack(item_stack.clone());
                screen_handler.set_received_stack(screen_slot, item_stack.clone());
            }
            changed
        };

        // Release the screen guard before synchronization or equipment callbacks.
        let mut equipment_change = None;
        if changed {
            match screen_slot {
                5..=8 => player.enqueue_equipment_change(
                    &match screen_slot {
                        5 => EquipmentSlot::HEAD,
                        6 => EquipmentSlot::CHEST,
                        7 => EquipmentSlot::LEGS,
                        _ => EquipmentSlot::FEET,
                    },
                    &item_stack,
                ),
                USE_ROW_SLOT_START..=SHIELD_SLOT => {
                    let inventory_slot = if screen_slot == SHIELD_SLOT {
                        PlayerInventory::OFF_HAND_SLOT
                    } else {
                        screen_slot - USE_ROW_SLOT_START
                    };
                    player.try_send_slot_set_packet(&CSetPlayerInventory::new(
                        (inventory_slot as i32).into(),
                        &ItemStackSerializer::from(item_stack.clone()),
                    ));
                    let equipment_slot = if inventory_slot == PlayerInventory::OFF_HAND_SLOT {
                        Some(EquipmentSlot::OFF_HAND)
                    } else if inventory_slot == usize::from(player.inventory().get_selected_slot())
                    {
                        Some(EquipmentSlot::MAIN_HAND)
                    } else {
                        None
                    };
                    equipment_change = equipment_slot.map(|slot| (slot, item_stack));
                }
                _ => {}
            }
        }
        player.sync_inventory_to_client();
        (changed, equipment_change)
    }

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
        if client_descriptor.id.0 != 0
            && client_descriptor.stack_size != 0
            && client_descriptor.id != authoritative_descriptor.id
        {
            player.try_send_slot_set_packet(&CSetPlayerInventory::new(
                (hand_inventory_slot(hand, player.inventory().get_selected_slot()) as i32).into(),
                &ItemStackSerializer::from(held),
            ));
            return None;
        }
        Some(held)
    }

    #[allow(clippy::too_many_lines, clippy::collapsible_if, clippy::unreachable)]
    pub fn handle_inventory_action(&self, player: &Arc<Player>, packet: SInventoryTransaction) {
        tracing::debug!("handle_inventory_action: packet={:?}", packet);
        let mut source_selected_slot = player.inventory().get_selected_slot();
        let mut origin_screen_slot = None;
        let mut release_hand = None;
        let use_item_stack = match &packet.transaction_data {
            TransactionData::UseItem(data) if matches!(data.action_type.0, 0 | 1) => {
                let Some(selected) =
                    self.reconcile_use_item_selection(player, data.hot_bar_slot.0, data.hand)
                else {
                    return;
                };
                source_selected_slot = selected;
                let hand = match data.hand {
                    HandSlot::Mainhand => Hand::Right,
                    HandSlot::Offhand => Hand::Left,
                };
                let Some(stack) = Self::validated_use_item_stack(player, hand, &data.item_in_hand)
                else {
                    return;
                };
                origin_screen_slot = hand_screen_handler_slot(hand, source_selected_slot);
                Some(stack)
            }
            TransactionData::UseItemOnEntity(data) => {
                let Some(selected) = self.reconcile_use_item_selection(
                    player,
                    data.hot_bar_slot.0,
                    HandSlot::Mainhand,
                ) else {
                    return;
                };
                source_selected_slot = selected;
                origin_screen_slot = hand_screen_handler_slot(Hand::Right, source_selected_slot);
                let Some(stack) =
                    Self::validated_use_item_stack(player, Hand::Right, &data.item_in_hand)
                else {
                    return;
                };
                Some(stack)
            }
            TransactionData::ReleaseItem(_) => {
                release_hand = *player
                    .living_entity
                    .active_hand
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let hand = release_hand.unwrap_or(Hand::Right);
                origin_screen_slot = hand_screen_handler_slot(hand, source_selected_slot);
                Some(player.inventory().get_stack_in_hand(hand))
            }
            _ => None,
        };
        let mut inventory_updated = false;
        let mut dropped_items = Vec::new();
        let mut hand_updates: Vec<(EquipmentSlot, ItemStack)> = Vec::new();
        let mut updates = Vec::new();
        let result = 0u8;

        let effects = crate::world::guard_effects::GuardEffects::new();
        if packet.actions.is_empty() && packet.legacy_request_id.0 != 0 {
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
                            // ServerPlayer.drop awards statistics after slot removal commits.
                            dropped_items.push(current_stack.clone());

                            player_screen_handler
                                .get_slot(screen_slot)
                                .set_stack(ItemStack::EMPTY.clone());
                            player_screen_handler
                                .set_received_stack(screen_slot, ItemStack::EMPTY.clone());

                            record_update(
                                &mut updates,
                                FullContainerName {
                                    container_name: match legacy_slot.container_id {
                                        28 => ContainerName::HotBar,
                                        _ => ContainerName::Inventory,
                                    },
                                    dynamic_id: None,
                                },
                                slot_id,
                                ItemStack::EMPTY,
                            );
                            inventory_updated = true;
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

        for action in &packet.actions {
            use pumpkin_protocol::bedrock::server::inventory_transaction::InventoryActionSource;
            let source_type = InventoryActionSource::from(action.source_type);
            if source_type == InventoryActionSource::World {
                let old_stack = descriptor_to_stack(&action.old_item);
                let new_stack = descriptor_to_stack(&action.new_item);
                if old_stack.is_empty() && !new_stack.is_empty() {
                    player.drop_item(new_stack);
                }
            } else if let Some(window_id) = action.window_id {
                if let Some(screen_slot) =
                    map_bedrock_slot_to_screen_handler(window_id, action.inventory_slot)
                {
                    // ServerPlayerGameMode.useItem/useItemOn and Player.interactOn consume the server stack once.
                    if origin_screen_slot == Some(screen_slot) {
                        continue;
                    }
                    let item_stack = descriptor_to_stack(&action.new_item);
                    inventory_updated = true;
                    // Descriptor-only no-ops must also retain authoritative components and UID.
                    if descriptor_to_stack(&action.old_item).are_equal(&item_stack) {
                        continue;
                    }
                    let (_, equipment_change) =
                        Self::apply_inventory_action(player, screen_slot, item_stack);
                    if let Some((slot, stack)) = equipment_change {
                        if let Some((_, previous)) =
                            hand_updates.iter_mut().find(|(s, _)| *s == slot)
                        {
                            *previous = stack;
                        } else {
                            hand_updates.push((slot, stack));
                        }
                    }
                }
            }
        }

        // LivingEntity.collectEquipmentChanges removes all old modifiers before adding new ones.
        if !hand_updates.is_empty() {
            for (slot, stack) in &mut hand_updates {
                let hand = if *slot == EquipmentSlot::MAIN_HAND {
                    Hand::Right
                } else {
                    Hand::Left
                };
                *stack = player.inventory().get_stack_in_hand(hand);
            }
            hand_updates.sort_unstable_by_key(|(slot, _)| slot.discriminant());
            player.send_menu_equipment(&hand_updates);
        }
        if inventory_updated {
            self.send_player_inventory_content(player);
        }

        // Bundled actions have published; retain both hands before gameplay callbacks.
        let hands_before_use = LiveHands::read(player);
        match packet.transaction_data {
            TransactionData::Normal(_data) => {
                // Actions are already applied to the inventory screen handler above.
            }
            TransactionData::Mismatch(_data) => {
                // Actions are already applied to the inventory screen handler above.
            }
            TransactionData::UseItem(data) => {
                let face = match data.block_face {
                    0 => BlockDirection::Down,
                    2 => BlockDirection::North,
                    3 => BlockDirection::South,
                    4 => BlockDirection::West,
                    5 => BlockDirection::East,
                    _ => BlockDirection::Up,
                };
                let world = player.world();
                let block = world.get_block(&data.block_position);
                let Some(server) = world.server.upgrade() else {
                    return;
                };

                if player.gamemode.load() == GameMode::Spectator {
                    if let Some(factory) = server.block_registry.get_screen_handler_factory(
                        block,
                        player,
                        &data.block_position,
                        &server,
                        &world,
                    ) {
                        player.open_handled_screen(factory.as_ref(), Some(data.block_position));
                    }
                    return;
                }

                if data.action_type.0 == 0 {
                    // Click block
                    // Bedrock UseItemTransactionData.hand serves as ServerboundUseItemOnPacket.hand.
                    let hand = match data.hand {
                        HandSlot::Mainhand => Hand::Right,
                        HandSlot::Offhand => Hand::Left,
                    };
                    let equipment_slot = match hand {
                        Hand::Right => EquipmentSlot::MAIN_HAND,
                        Hand::Left => EquipmentSlot::OFF_HAND,
                    };

                    let Some(before_block_use) = use_item_stack else {
                        return;
                    };
                    let Some(mut held_item) = Self::stack_for_hand_use(
                        player,
                        hand,
                        source_selected_slot,
                        &before_block_use,
                        &hands_before_use,
                    ) else {
                        return;
                    };

                    let result = server.block_registry.use_with_item(
                        block,
                        player,
                        &data.block_position,
                        &BlockHitResult {
                            face: &face,
                            cursor_pos: &data.click_position,
                        },
                        &mut held_item,
                        &equipment_slot,
                        &server,
                        &world,
                    );

                    if result.consumes_action() {
                        // ServerPlayerGameMode.useItemOn persists the item after the block callback consumes it.
                        Self::finish_block_use(
                            player,
                            hand,
                            source_selected_slot,
                            &before_block_use,
                            &held_item,
                            &hands_before_use,
                        );
                        return;
                    }

                    // ServerPlayerGameMode.useItemOn restricts useWithoutItem to the main hand.
                    if matches!(result, BlockActionResult::PassToDefaultBlockAction)
                        && hand == Hand::Right
                    {
                        if Self::stack_for_hand_use(
                            player,
                            hand,
                            source_selected_slot,
                            &before_block_use,
                            &hands_before_use,
                        )
                        .is_none()
                        {
                            return;
                        }
                        let result = server.block_registry.on_use(
                            block,
                            player,
                            &data.block_position,
                            &BlockHitResult {
                                face: &face,
                                cursor_pos: &data.click_position,
                            },
                            &server,
                            &world,
                        );

                        if result.consumes_action() {
                            Self::finish_block_use(
                                player,
                                hand,
                                source_selected_slot,
                                &before_block_use,
                                &held_item,
                                &hands_before_use,
                            );
                            return;
                        }
                    }

                    let mut stack = held_item;
                    let mut placement_denied = false;
                    if !stack.is_empty() {
                        let item_id = stack.item.id;
                        player.increment_stat(
                            pumpkin_data::statistic::StatisticCategory::Used,
                            item_id as i32,
                            1,
                        );
                        if Self::stack_for_hand_use(
                            player,
                            hand,
                            source_selected_slot,
                            &before_block_use,
                            &hands_before_use,
                        )
                        .is_none()
                        {
                            return;
                        }
                        let item_result = server.item_registry.use_on_block(
                            &mut stack,
                            player,
                            data.block_position,
                            face,
                            data.click_position,
                            block,
                            &server,
                        );

                        if should_try_block_placement(&item_result) {
                            let item_id = stack.item.id;
                            if let Some(placed_block) = pumpkin_data::Block::from_item_id(item_id) {
                                let dummy_use_item_on =
                                    pumpkin_protocol::java::server::play::SUseItemOn {
                                        // UseOnContext retains ServerboundUseItemOnPacket.hand.
                                        hand: VarInt(i32::from(hand == Hand::Left)),
                                        position: data.block_position,
                                        face: VarInt(i32::from(data.block_face)),
                                        cursor_pos: data.click_position,
                                        inside_block: false,
                                        is_against_world_border: false,
                                        sequence: VarInt(0),
                                    };

                                match server.block_registry.place_block_with_stack(
                                    player,
                                    placed_block,
                                    &server,
                                    &dummy_use_item_on,
                                    data.block_position,
                                    face,
                                    &stack,
                                ) {
                                    Ok(Some(_)) => {
                                        if player.gamemode.load() != GameMode::Creative {
                                            // BlockItem.place consumes the retained UseOnContext stack.
                                            stack.decrement(1);
                                        }
                                    }
                                    Ok(None) | Err(_) => placement_denied = true,
                                }
                            }
                        }
                    }
                    Self::finish_block_use(
                        player,
                        hand,
                        source_selected_slot,
                        &before_block_use,
                        &stack,
                        &hands_before_use,
                    );
                    if placement_denied {
                        self.correct_denied_placement(player, data.block_position, face);
                    }
                } else if data.action_type.0 == 1 {
                    // Click air / Use item
                    let hand = match data.hand {
                        HandSlot::Mainhand => Hand::Right,
                        HandSlot::Offhand => Hand::Left,
                    };
                    let Some(before_actions) = use_item_stack else {
                        return;
                    };
                    let Some(mut held) = Self::stack_for_hand_use(
                        player,
                        hand,
                        source_selected_slot,
                        &before_actions,
                        &hands_before_use,
                    ) else {
                        return;
                    };
                    // Consumable.onConsume and BucketItem/BottleItem.use count on success.
                    if !held.is_empty() && !server.item_registry.records_item_use_stat(&held) {
                        player.increment_stat(
                            pumpkin_data::statistic::StatisticCategory::Used,
                            held.item.id as i32,
                            1,
                        );
                    }

                    if Self::stack_for_hand_use(
                        player,
                        hand,
                        source_selected_slot,
                        &before_actions,
                        &hands_before_use,
                    )
                    .is_none()
                    {
                        return;
                    }

                    let event = PlayerInteractEvent::new(
                        player,
                        InteractAction::RightClickAir,
                        &pumpkin_data::Block::AIR,
                        None,
                    );

                    let stack_for_use = held.clone();

                    {
                        let mut cooldown_active = false;
                        if let Some(cooldown) = held.get_use_cooldown() {
                            let group = cooldown
                                .cooldown_group
                                .clone()
                                .unwrap_or_else(|| held.item.registry_key.to_string());
                            if player.is_on_cooldown(&group) {
                                cooldown_active = true;
                            }
                        }

                        if !cooldown_active {
                            // Bedrock can repeat click-air while using an item. Do not
                            // restart its server-side use timer on these timed inputs.
                            let already_using =
                                player.living_entity.item_use_time.load(Ordering::Relaxed) > 0
                                    && player
                                        .living_entity
                                        .item_in_use
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .as_ref()
                                        .is_some_and(|item| {
                                            item.are_items_and_components_equal(&held)
                                        });
                            if !already_using
                                && (held.get_data_component::<ConsumableImpl>().is_some()
                                    || held.get_data_component::<BlocksAttacksImpl>().is_some())
                            {
                                if held
                                    .get_data_component::<FoodImpl>()
                                    .is_none_or(|food| player.can_eat(food.can_always_eat))
                                {
                                    player.living_entity.set_active_hand(
                                        hand,
                                        held.clone(),
                                        held.get_max_use_time(),
                                    );
                                } else {
                                    // Correct predicted eating when the server's food
                                    // level is already full, even if it has not changed.
                                    self.correct_rejected_food_use(player);
                                }
                            }
                            if let Some(equippable) = held.get_data_component::<EquippableImpl>() {
                                let should_change = {
                                    let inventory = player.inventory();
                                    let equipment_guard = inventory
                                        .entity_equipment
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    let current_equipped = equipment_guard.get(equippable.slot);
                                    !current_equipped.are_items_and_components_equal(&held)
                                };
                                if should_change {
                                    let slot = equippable.slot;
                                    let Some((new_held, equipped)) = Self::equip_admitted_hand(
                                        player,
                                        hand,
                                        source_selected_slot,
                                        &held,
                                        slot,
                                    ) else {
                                        Self::sync_live_hands(
                                            player,
                                            hand,
                                            source_selected_slot,
                                            &hands_before_use,
                                        );
                                        return;
                                    };
                                    held = new_held;
                                    player.enqueue_equipment_change(slot, &equipped);
                                }
                            }
                        }
                    }

                    send_cancellable_blocking! {{
                        &server;
                        event;
                        'after: {
                            if Self::stack_for_hand_use(
                                player, hand, source_selected_slot, &held, &hands_before_use).is_none() {
                                return;
                            }
                            server.item_registry.on_use(&stack_for_use, player, hand);
                        }
                    }}
                    // Publish the final authoritative hand after use or cancellation.
                    Self::sync_live_hands(player, hand, source_selected_slot, &hands_before_use);
                }
            }
            TransactionData::UseItemOnEntity(data) => {
                if let Some(before) = use_item_stack {
                    Self::interact_entity_with_admitted_hand(
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
                }
            }
            TransactionData::ReleaseItem(_) => {
                let hand = release_hand.unwrap_or(Hand::Right);
                if let Some(before) = use_item_stack {
                    Self::release_admitted_hand(player, hand, source_selected_slot, &before);
                    Self::sync_live_hands(player, hand, source_selected_slot, &hands_before_use);
                }
            }
        }

        if packet.legacy_request_id.0 != 0 {
            use pumpkin_protocol::bedrock::client::item_stack_response::{
                CItemStackResponse, ItemStackResponseContainerInfo, ItemStackResponseInfo,
                ItemStackResponseSlotInfo,
            };

            let mut container_infos = Vec::new();
            if result == 0 {
                for update in updates {
                    let container_info = container_infos.iter_mut().find(
                        |info: &&mut ItemStackResponseContainerInfo| {
                            info.full_container_name == update.container_name
                        },
                    );

                    let slot_info = ItemStackResponseSlotInfo {
                        requested_slot: update.slot_id,
                        slot: update.slot_id,
                        amount: update.count,
                        item_stack_net_id: update.stack_id,
                        custom_name: String::new(),
                        filtered_custom_name: String::new(),
                        durability_correction: VarInt(0),
                    };

                    if let Some(info) = container_info {
                        info.slots.push(slot_info);
                    } else {
                        container_infos.push(ItemStackResponseContainerInfo {
                            full_container_name: update.container_name.clone(),
                            slots: vec![slot_info],
                        });
                    }
                }
            }

            self.try_enqueue_client_packet(&CItemStackResponse {
                responses: vec![ItemStackResponseInfo {
                    result,
                    client_request_id: packet.legacy_request_id,
                    containers: container_infos,
                }],
            });
        }
    }
}

#[cfg(test)]
#[path = "inventory_action_tests.rs"]
mod tests;
