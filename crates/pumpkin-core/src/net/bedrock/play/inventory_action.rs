#[allow(clippy::wildcard_imports)]
use super::*;
use crate::item::registry::should_try_block_placement;
use pumpkin_protocol::bedrock::server::inventory_transaction::HandSlot;

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

    fn sync_consumed_hand_item(player: &Player, stack: ItemStack) {
        let selected_slot = player.inventory().get_selected_slot();
        player.inventory().set_held_item(stack.clone());
        let mut screen_handler = player
            .player_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        screen_handler.set_received_stack(36 + usize::from(selected_slot), stack);
        drop(screen_handler);
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

    pub fn handle_inventory_action(&self, player: &Arc<Player>, packet: SInventoryTransaction) {
        tracing::debug!("handle_inventory_action: packet={:?}", packet);
        let request_id = packet.legacy_request_id;
        Self::drop_legacy_inventory_slots(player, &packet);
        // ServerGamePacketListenerImpl.handleContainerClick treats client stacks as predictions.
        // handleUseItemOn/handleUseItem still execute against the server's hand first.
        let accepted = self.process_inventory_transaction(player, packet);
        self.correct_inventory_prediction(player);
        player.send_menu_equipment(&[
            (EquipmentSlot::MAIN_HAND, player.inventory().held_item()),
            (EquipmentSlot::OFF_HAND, player.inventory().off_hand_item()),
        ]);
        // Bedrock ItemStackResponseInfo: 0 = SUCCESS, 1 = ERROR. Discarding a
        // prediction does not reject an otherwise processed authoritative use.
        self.respond_legacy_inventory(request_id, u8::from(!accepted));
    }

    fn drop_legacy_inventory_slots(player: &Player, packet: &SInventoryTransaction) {
        let mut dropped_items = Vec::new();
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

    #[allow(clippy::too_many_lines, clippy::collapsible_if, clippy::unreachable)]
    fn process_inventory_transaction(
        &self,
        player: &Arc<Player>,
        packet: SInventoryTransaction,
    ) -> bool {
        match packet.transaction_data {
            TransactionData::Normal(_) | TransactionData::Mismatch(_) => {
                // No authoritative operation exists for legacy slot writes, including
                // World-source drops of client-described stacks.
                return packet.actions.is_empty();
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
                    return false;
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
                    return true;
                }

                if data.action_type.0 == 0 {
                    // Click block
                    let client_stack = descriptor_to_stack(&data.item_in_hand);

                    let mut held_item = player.inventory().held_item();
                    let authoritative_held_item = held_item.item.id == client_stack.item.id
                        && held_item.item_count == client_stack.item_count;
                    if !authoritative_held_item {
                        return false;
                    }
                    let before_block_use = held_item.clone();

                    let result = server.block_registry.use_with_item(
                        block,
                        player,
                        &data.block_position,
                        &BlockHitResult {
                            face: &face,
                            cursor_pos: &data.click_position,
                        },
                        &mut held_item,
                        &EquipmentSlot::MAIN_HAND,
                        &server,
                        &world,
                    );

                    if result.consumes_action() {
                        if !before_block_use.are_equal(&held_item) {
                            // ServerPlayerGameMode.useItemOn persists the item after the block callback consumes it.
                            Self::sync_consumed_hand_item(player, held_item);
                        }
                        return true;
                    }

                    if matches!(result, BlockActionResult::PassToDefaultBlockAction) {
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
                            return true;
                        }
                    }

                    let mut stack = held_item;
                    if !stack.is_empty() {
                        let item_id = stack.item.id;
                        let before = stack.clone();
                        player.increment_stat(
                            pumpkin_data::statistic::StatisticCategory::Used,
                            item_id as i32,
                            1,
                        );
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
                                        hand: VarInt(0),
                                        position: data.block_position,
                                        face: VarInt(i32::from(data.block_face)),
                                        cursor_pos: data.click_position,
                                        inside_block: false,
                                        is_against_world_border: false,
                                        sequence: VarInt(0),
                                    };

                                if let Ok(Some(_)) = server.block_registry.place_block(
                                    player,
                                    placed_block,
                                    &server,
                                    &dummy_use_item_on,
                                    data.block_position,
                                    face,
                                ) && player.gamemode.load() != GameMode::Creative
                                {
                                    stack.decrement(1);
                                }
                            }
                        }
                        if before.is_damageable() && stack.is_empty() {
                            player.increment_stat(
                                pumpkin_data::statistic::StatisticCategory::Broken,
                                item_id as i32,
                                1,
                            );
                            player.world().send_entity_status(
                                player.get_entity(),
                                crate::entity::equipment_break_status(&EquipmentSlot::MAIN_HAND),
                                None,
                            );
                        }
                        player.inventory().set_held_item(stack);
                    }
                } else if data.action_type.0 == 1 {
                    // Click air / Use item
                    let client_stack = descriptor_to_stack(&data.item_in_hand);

                    let hand = match data.hand {
                        HandSlot::Mainhand => Hand::Right,
                        HandSlot::Offhand => Hand::Left,
                    };
                    let mut held = player.inventory().get_stack_in_hand(hand);
                    if held.item.id != client_stack.item.id
                        || held.item_count != client_stack.item_count
                    {
                        return false;
                    }
                    // Consumable.onConsume and BucketItem/BottleItem.use count on success.
                    if !held.is_empty() && !server.item_registry.records_item_use_stat(&held) {
                        player.increment_stat(
                            pumpkin_data::statistic::StatisticCategory::Used,
                            held.item.id as i32,
                            1,
                        );
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
                                    player.enqueue_equipment_change(equippable.slot, &held);

                                    let inventory = player.inventory();
                                    let mut equipment_guard = inventory
                                        .entity_equipment
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    let equip_item = equipment_guard
                                        .equipment
                                        .entry(equippable.slot.clone())
                                        .or_insert_with(|| ItemStack::EMPTY.clone());
                                    if equip_item.is_empty() {
                                        *equip_item = held.clone();
                                        held.decrement_unless_creative(player.gamemode.load(), 1);
                                    } else {
                                        let old_held = held.clone();
                                        held = equip_item.clone();
                                        *equip_item = old_held;
                                    }
                                    drop(equipment_guard);
                                    player.inventory().set_stack_in_hand(hand, held.clone());
                                }
                            }
                        }
                    }

                    send_cancellable_blocking! {{
                        &server;
                        event;
                        'after: {
                            server.item_registry.on_use(&stack_for_use, player, hand);
                        }
                        'cancelled: {
                            return false;
                        }
                    }}
                }
            }
            TransactionData::UseItemOnEntity(data) => {
                let action = match data.action_type.0 {
                    // Bedrock does not distinguish an entity hit position here. ItemInteract is
                    // therefore exposed as the general Interact action rather than InteractAt.
                    0 | 2 => ActionType::Interact,
                    1 => ActionType::Attack,
                    action => {
                        tracing::warn!("invalid UseItemOnEntity action type {action}");
                        return false;
                    }
                };
                let Ok(target_runtime_id) = i32::try_from(data.target_entity_runtime_id.0) else {
                    tracing::warn!(
                        "invalid UseItemOnEntity target runtime ID {}",
                        data.target_entity_runtime_id.0
                    );
                    return false;
                };

                let world = player.world();
                let Some(target) = world.get_entity_by_id(target_runtime_id) else {
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
                    target.clone(),
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
                        let source_slot =
                            crate::net::java::play::hand_use_result::hand_slot(player, Hand::Right);
                        let mut stack = player.inventory().held_item();
                        let item_id = stack.item.id;
                        let before = stack.clone();
                        let interacted =
                            event
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
                        if !stack.are_equal(&before) {
                            if !stand_swap {
                                player.increment_stat(
                                    pumpkin_data::statistic::StatisticCategory::Used,
                                    item_id as i32,
                                    1,
                                );
                            }
                        }
                        crate::net::java::play::hand_use_result::write_back_hand_item(
                            player,
                            Hand::Right,
                            source_slot,
                            &before,
                            &stack,
                            if stand_swap {
                                crate::net::java::play::hand_use_result::HandMutation::EquipmentTransfer
                            } else {
                                crate::net::java::play::hand_use_result::HandMutation::ItemUse
                            },
                        );
                    }
                    ActionType::Attack => player.attack(&event.target),
                }
            }
            TransactionData::ReleaseItem(_data) => {
                let item_in_use = player
                    .living_entity
                    .item_in_use
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                if let Some(stack) = item_in_use {
                    let Some(server) = player.world().server.upgrade() else {
                        return false;
                    };
                    server.item_registry.on_stopped_using(&stack, player);
                }
                player.living_entity.clear_active_hand();
            }
        }

        true
    }
}

#[cfg(test)]
#[path = "inventory_action_tests.rs"]
mod tests;
