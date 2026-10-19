#[allow(clippy::wildcard_imports)]
use super::*;

impl BedrockClient {
    #[allow(clippy::too_many_lines)]
    pub fn handle_item_stack_request(
        &self,
        player: &Arc<Player>,
        packet: pumpkin_protocol::bedrock::server::item_stack_request::SItemStackRequest,
    ) {
        use pumpkin_protocol::bedrock::client::item_stack_response::{
            CItemStackResponse, ItemStackResponseContainerInfo, ItemStackResponseInfo,
            ItemStackResponseSlotInfo,
        };
        use pumpkin_protocol::bedrock::server::item_stack_request::ItemStackRequestAction;

        // Snapshot dynamic recipes before entering the menu; matching staged grids
        // must not invoke a live recipe provider beneath its guard.
        let recipes = super::inventory_request_take::RecipeSnapshot(
            if packet
                .requests
                .iter()
                .flat_map(|request| &request.actions)
                .any(|action| {
                    matches!(
                        action,
                        ItemStackRequestAction::CraftRecipe { .. }
                            | ItemStackRequestAction::CraftRecipeAuto { .. }
                    )
                })
            {
                player
                    .world()
                    .server
                    .upgrade()
                    .map_or_else(Vec::new, |server| {
                        server.recipe_manager.get_dynamic_recipes_internal()
                    })
            } else {
                Vec::new()
            },
        );
        let effects = crate::world::guard_effects::GuardEffects::new();
        let mut correction_needed = false;
        let (responses, mut inventory_updated, unclaimed_outputs, crafting_effects) = {
            let current_screen_handler = player
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let mut screen_handler = current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            let mut responses = Vec::with_capacity(packet.requests.len());
            let mut unclaimed_outputs = Vec::new();
            let mut crafting_effects = Vec::new();
            let mut processed_request = false;

            for request in packet.requests {
                let mut created_item: Option<ItemStack> = None;
                let mut crafting_inputs_consumed = false;
                let mut pending_craft: Option<(ItemStack, u8)> = None;
                let mut updates = Vec::new();
                let mut result = 0u8; // 0 = Success, 1 = Error

                // Validate the entire request before its first write, including pseudo-slots.
                if !super::inventory_request_validation::validate_request(
                    &*screen_handler,
                    &request.actions,
                    player.gamemode.load(),
                ) {
                    responses.push(ItemStackResponseInfo {
                        result: 1,
                        client_request_id: request.request_id,
                        containers: Vec::new(),
                    });
                    continue;
                }

                if !super::inventory_request_take::validate_take_rules(
                    &*screen_handler,
                    &request.actions,
                    player.as_ref(),
                    &recipes,
                ) {
                    correction_needed = true;
                    responses.push(ItemStackResponseInfo {
                        result: 1,
                        client_request_id: request.request_id,
                        containers: Vec::new(),
                    });
                    continue;
                }

                processed_request = true;
                for action in request.actions {
                    match action {
                        ItemStackRequestAction::CraftCreative {
                            creative_item_id,
                            repetitions,
                        } => {
                            // handleSetCreativeModeSlot authorizes creation; ResultSlot.onTake pays one batch.
                            if player.gamemode.load() != GameMode::Creative
                                || created_item.is_some()
                                || crafting_inputs_consumed
                            {
                                result = 1;
                                break;
                            }
                            let index = (creative_item_id.0.saturating_sub(1)) as usize;
                            if index < pumpkin_data::bedrock_creative::CREATIVE_ENTRIES.len() {
                                let entry = pumpkin_data::bedrock_creative::CREATIVE_ENTRIES[index];
                                if let Some(mapping) =
                                    pumpkin_data::item::JavaToBedrockItemMapping::from_bedrock(
                                        entry.item_id,
                                        entry.item_aux_value,
                                    )
                                {
                                    // Bedrock `repetitions` represents how many stacks to create; use the item's max stack size
                                    let max_stack =
                                        ItemStack::static_new_java(1, mapping.java_item)
                                            .get_max_stack_size();
                                    let count = ((max_stack as u16) * (repetitions as u16))
                                        .min(u8::MAX as u16)
                                        as u8;
                                    created_item = Some(ItemStack::new(count, mapping.java_item));
                                } else {
                                    tracing::warn!(
                                        "Failed to map bedrock item id {} and data {} to Java item",
                                        entry.item_id,
                                        entry.item_aux_value
                                    );
                                    result = 1;
                                    break;
                                }
                            } else {
                                tracing::warn!(
                                    "Creative item index {} out of bounds (len: {})",
                                    index,
                                    pumpkin_data::bedrock_creative::CREATIVE_ENTRIES.len()
                                );
                                result = 1;
                                break;
                            }
                        }
                        ItemStackRequestAction::Take {
                            count,
                            source,
                            destination,
                        }
                        | ItemStackRequestAction::Place {
                            count,
                            source,
                            destination,
                        } => {
                            // AbstractContainerMenu.doClick resolves a destination before taking stock.
                            if destination.container_name.container_name != ContainerName::Cursor
                                && map_bedrock_container_slot(
                                    &*screen_handler,
                                    destination.container_name.container_name,
                                    destination.slot_id,
                                )
                                .and_then(|index| screen_handler.get_behaviour().slots.get(index))
                                .is_none()
                            {
                                result = 1;
                                break;
                            }
                            // ResultSlot.onTake: a pending recipe can leave only through its virtual output.
                            if destination.container_name.container_name
                                == ContainerName::CreatedOutput
                                || ((crafting_inputs_consumed || pending_craft.is_some())
                                    && source.container_name.container_name
                                        == ContainerName::CraftingOutputPreview)
                            {
                                result = 1;
                                break;
                            }
                            let mut source_stack =
                                get_slot_stack(&*screen_handler, &source, created_item.as_ref());
                            if source_stack.is_empty() && created_item.is_none() {
                                tracing::debug!("Source stack is empty in Take/Place");
                                result = 1;
                                break;
                            }
                            let expected_source = source_stack.clone();
                            let count = count.min(source_stack.item_count);
                            if count > 0 {
                                let mut dest_stack = get_slot_stack(
                                    &*screen_handler,
                                    &destination,
                                    created_item.as_ref(),
                                );
                                let expected_destination = dest_stack.clone();
                                let virtual_source = source.container_name.container_name
                                    == ContainerName::CreatedOutput
                                    && created_item.is_some();
                                if virtual_source {
                                    let Some(replacement) =
                                        super::inventory_commit::virtual_destination(
                                            &*screen_handler,
                                            &destination,
                                            &source_stack,
                                            &expected_destination,
                                            count,
                                        )
                                    else {
                                        result = 1;
                                        break;
                                    };
                                    dest_stack = replacement;
                                } else if dest_stack.is_empty() {
                                    dest_stack = source_stack.copy_with_count(count);
                                } else if dest_stack.are_items_and_components_equal(&source_stack) {
                                    // Slot.safeInsert retains input when the destination cannot grow.
                                    let Some(total) = dest_stack.item_count.checked_add(count)
                                    else {
                                        result = 1;
                                        break;
                                    };
                                    dest_stack.item_count = total;
                                } else {
                                    tracing::debug!(
                                        "Destination stack is not compatible with source stack"
                                    );
                                    result = 1;
                                    break;
                                }

                                let merchant_result = screen_handler.window_type()
                                    == Some(pumpkin_data::screen::WindowType::Merchant)
                                    && matches!(
                                        source.container_name.container_name,
                                        ContainerName::CreatedOutput
                                            | ContainerName::TradeResultPreview
                                            | ContainerName::Trade2ResultPreview
                                    );
                                if merchant_result {
                                    if count != source_stack.item_count {
                                        result = 1;
                                        break;
                                    }
                                    let Some(handler) = screen_handler
                                    .as_any_mut()
                                    .downcast_mut::<pumpkin_inventory::merchant::merchant_screen_handler::MerchantScreenHandler>()
                                else {
                                    result = 1;
                                    break;
                                };
                                    let destination_slot = map_bedrock_container_slot(
                                        handler,
                                        destination.container_name.container_name,
                                        destination.slot_id,
                                    )
                                    .map(|index| handler.get_behaviour().slots[index].clone());
                                    if !handler.complete_bedrock_trade_into(
                                        player.as_ref(),
                                        &expected_source,
                                        destination_slot.as_deref(),
                                        (&expected_destination, dest_stack.clone()),
                                    ) {
                                        result = 1;
                                        break;
                                    }

                                    update_slot_stack(
                                        player,
                                        handler,
                                        &destination,
                                        &expected_destination,
                                        dest_stack.clone(),
                                    );
                                    for (container_name, slot_id, screen_slot) in [
                                        (ContainerName::Trade2Ingredient1, 4, 0),
                                        (ContainerName::Trade2Ingredient2, 5, 1),
                                        (ContainerName::Trade2ResultPreview, 50, 2),
                                    ] {
                                        let stack = handler.get_behaviour().slots[screen_slot]
                                            .get_cloned_stack();
                                        record_update(
                                            &mut updates,
                                            FullContainerName {
                                                container_name,
                                                dynamic_id: None,
                                            },
                                            slot_id,
                                            &stack,
                                        );
                                    }
                                    record_update(
                                        &mut updates,
                                        source.container_name.clone(),
                                        source.slot_id,
                                        ItemStack::EMPTY,
                                    );
                                    record_update(
                                        &mut updates,
                                        destination.container_name.clone(),
                                        destination.slot_id,
                                        &dest_stack,
                                    );
                                    continue;
                                }

                                source_stack.decrement(count);
                                let source_stack = if source_stack.is_empty() {
                                    ItemStack::EMPTY.clone()
                                } else {
                                    source_stack
                                };
                                let committed = if virtual_source {
                                    if let Some((output, repetitions)) = &pending_craft {
                                        let destination_slot =
                                            if destination.container_name.container_name
                                                == ContainerName::Cursor
                                            {
                                                // The menu guard owns the cursor until update_slot_stack publishes this staged replacement.
                                                let inventory = Arc::new(
                                                    pumpkin_inventory::SimpleInventory::new(1),
                                                );
                                                pumpkin_inventory::Inventory::set_stack(
                                                    inventory.as_ref(),
                                                    0,
                                                    expected_destination.clone(),
                                                );
                                                Some(Arc::new(
                                                    pumpkin_inventory::slot::NormalSlot::new(
                                                        inventory, 0,
                                                    ),
                                                )
                                                    as Arc<dyn pumpkin_inventory::slot::Slot>)
                                            } else {
                                                map_bedrock_container_slot(
                                                    &*screen_handler,
                                                    destination.container_name.container_name,
                                                    destination.slot_id,
                                                )
                                                .map(|index| {
                                                    screen_handler.get_behaviour().slots[index]
                                                        .clone()
                                                })
                                            };
                                        destination_slot.is_some_and(|slot| {
                                            let commit = screen_handler.get_behaviour().slots[0]
                                                .craft_into(
                                                    output,
                                                    *repetitions,
                                                    slot.as_ref(),
                                                    &expected_destination,
                                                    &dest_stack,
                                                );
                                            commit.is_some_and(|commit| {
                                                crafting_effects.push(commit);
                                                true
                                            })
                                        })
                                    } else {
                                        super::inventory_commit::commit(
                                            &*screen_handler,
                                            player.as_ref(),
                                            &[(&destination, &expected_destination, &dest_stack)],
                                        )
                                    }
                                } else {
                                    super::inventory_commit::commit(
                                        &*screen_handler,
                                        player.as_ref(),
                                        &[
                                            (&source, &expected_source, &source_stack),
                                            (&destination, &expected_destination, &dest_stack),
                                        ],
                                    )
                                };
                                if !committed {
                                    result = 1;
                                    break;
                                }
                                if virtual_source {
                                    // Bedrock CreatedOutput is request-owned stock, never a physical result slot.
                                    created_item =
                                        (!source_stack.is_empty()).then(|| source_stack.clone());
                                    crafting_inputs_consumed |= pending_craft.take().is_some();
                                } else {
                                    update_slot_stack(
                                        player,
                                        &mut *screen_handler,
                                        &source,
                                        &expected_source,
                                        source_stack.clone(),
                                    );
                                }
                                update_slot_stack(
                                    player,
                                    &mut *screen_handler,
                                    &destination,
                                    &expected_destination,
                                    dest_stack.clone(),
                                );

                                record_update(
                                    &mut updates,
                                    source.container_name.clone(),
                                    source.slot_id,
                                    &source_stack,
                                );
                                record_update(
                                    &mut updates,
                                    destination.container_name.clone(),
                                    destination.slot_id,
                                    &dest_stack,
                                );
                            }
                        }
                        ItemStackRequestAction::Swap { slot1, slot2 } => {
                            if [
                                slot1.container_name.container_name,
                                slot2.container_name.container_name,
                            ]
                            .contains(&ContainerName::CreatedOutput)
                                || ((crafting_inputs_consumed || pending_craft.is_some())
                                    && [
                                        slot1.container_name.container_name,
                                        slot2.container_name.container_name,
                                    ]
                                    .contains(&ContainerName::CraftingOutputPreview))
                            {
                                result = 1;
                                break;
                            }
                            let stack1 =
                                get_slot_stack(&*screen_handler, &slot1, created_item.as_ref());
                            let stack2 =
                                get_slot_stack(&*screen_handler, &slot2, created_item.as_ref());

                            if !super::inventory_commit::commit(
                                &*screen_handler,
                                player.as_ref(),
                                &[(&slot1, &stack1, &stack2), (&slot2, &stack2, &stack1)],
                            ) {
                                result = 1;
                                break;
                            }
                            update_slot_stack(
                                player,
                                &mut *screen_handler,
                                &slot1,
                                &stack1,
                                stack2.clone(),
                            );
                            update_slot_stack(
                                player,
                                &mut *screen_handler,
                                &slot2,
                                &stack2,
                                stack1.clone(),
                            );

                            record_update(
                                &mut updates,
                                slot1.container_name.clone(),
                                slot1.slot_id,
                                &stack2,
                            );
                            record_update(
                                &mut updates,
                                slot2.container_name.clone(),
                                slot2.slot_id,
                                &stack1,
                            );
                        }
                        ItemStackRequestAction::Drop {
                            count,
                            source,
                            randomly: _,
                        } => {
                            if source.container_name.container_name == ContainerName::CreatedOutput
                                || ((crafting_inputs_consumed || pending_craft.is_some())
                                    && source.container_name.container_name
                                        == ContainerName::CraftingOutputPreview)
                            {
                                result = 1;
                                break;
                            }
                            let mut source_stack =
                                get_slot_stack(&*screen_handler, &source, created_item.as_ref());
                            if source_stack.is_empty() {
                                result = 1;
                                break;
                            }
                            let expected_source = source_stack.clone();
                            let count = count.min(source_stack.item_count);
                            if count > 0 {
                                let dropped_stack = source_stack.copy_with_count(count);
                                source_stack.decrement(count);
                                let source_stack = if source_stack.is_empty() {
                                    ItemStack::EMPTY.clone()
                                } else {
                                    source_stack
                                };

                                if !super::inventory_commit::commit(
                                    &*screen_handler,
                                    player.as_ref(),
                                    &[(&source, &expected_source, &source_stack)],
                                ) {
                                    result = 1;
                                    break;
                                }
                                update_slot_stack(
                                    player,
                                    &mut *screen_handler,
                                    &source,
                                    &expected_source,
                                    source_stack.clone(),
                                );

                                let dropper = player.clone();
                                player.run_menu_callback(Box::new(move || {
                                    dropper.drop_item(dropped_stack);
                                }));
                                record_update(
                                    &mut updates,
                                    source.container_name.clone(),
                                    source.slot_id,
                                    &source_stack,
                                );
                            }
                        }
                        ItemStackRequestAction::Destroy { count, source }
                        | ItemStackRequestAction::Consume { count, source } => {
                            if source.container_name.container_name == ContainerName::CreatedOutput
                                || ((crafting_inputs_consumed || pending_craft.is_some())
                                    && source.container_name.container_name
                                        == ContainerName::CraftingOutputPreview)
                            {
                                result = 1;
                                break;
                            }
                            if screen_handler.window_type()
                                == Some(pumpkin_data::screen::WindowType::Merchant)
                                && matches!(
                                    source.container_name.container_name,
                                    ContainerName::TradeIngredient1
                                        | ContainerName::TradeIngredient2
                                        | ContainerName::Trade2Ingredient1
                                        | ContainerName::Trade2Ingredient2
                                )
                            {
                                // MerchantScreenHandler consumes both payment slots atomically when
                                // Bedrock subsequently takes the CreatedOutput result.
                                continue;
                            }
                            if (crafting_inputs_consumed || pending_craft.is_some())
                                && source.container_name.container_name
                                    == ContainerName::CraftingInput
                            {
                                let source_stack = get_slot_stack(
                                    &*screen_handler,
                                    &source,
                                    created_item.as_ref(),
                                );
                                record_update(
                                    &mut updates,
                                    source.container_name.clone(),
                                    source.slot_id,
                                    &source_stack,
                                );
                                continue;
                            }

                            let mut source_stack =
                                get_slot_stack(&*screen_handler, &source, created_item.as_ref());
                            if source_stack.is_empty() {
                                result = 1;
                                break;
                            }
                            let expected_source = source_stack.clone();
                            let count = count.min(source_stack.item_count);
                            if count > 0 {
                                source_stack.decrement(count);
                                let source_stack = if source_stack.is_empty() {
                                    ItemStack::EMPTY.clone()
                                } else {
                                    source_stack
                                };

                                if !super::inventory_commit::commit(
                                    &*screen_handler,
                                    player.as_ref(),
                                    &[(&source, &expected_source, &source_stack)],
                                ) {
                                    result = 1;
                                    break;
                                }
                                update_slot_stack(
                                    player,
                                    &mut *screen_handler,
                                    &source,
                                    &expected_source,
                                    source_stack.clone(),
                                );

                                record_update(
                                    &mut updates,
                                    source.container_name.clone(),
                                    source.slot_id,
                                    &source_stack,
                                );
                            }
                        }
                        ItemStackRequestAction::CraftRecipe {
                            recipe_id,
                            repetitions,
                        }
                        | ItemStackRequestAction::CraftRecipeAuto {
                            recipe_id,
                            repetitions,
                        } => {
                            if screen_handler.window_type()
                                == Some(pumpkin_data::screen::WindowType::Merchant)
                            {
                                let Some(handler) = screen_handler
                                .as_any_mut()
                                .downcast_mut::<pumpkin_inventory::merchant::merchant_screen_handler::MerchantScreenHandler>()
                            else {
                                result = 1;
                                break;
                            };
                                let trade = recipe_id.0.saturating_sub(1) as usize;
                                if trade >= handler.offers.len() {
                                    result = 1;
                                    break;
                                }
                                handler.set_selected_offer(trade, player.as_ref());
                                for (container_name, slot_id, screen_slot) in [
                                    (ContainerName::Trade2Ingredient1, 4, 0),
                                    (ContainerName::Trade2Ingredient2, 5, 1),
                                    (ContainerName::Trade2ResultPreview, 50, 2),
                                ] {
                                    let stack = handler.get_behaviour().slots[screen_slot]
                                        .get_cloned_stack();
                                    record_update(
                                        &mut updates,
                                        FullContainerName {
                                            container_name,
                                            dynamic_id: None,
                                        },
                                        slot_id,
                                        &stack,
                                    );
                                }
                                continue;
                            }

                            if repetitions > 0 {
                                // ResultSlot.onTake pays one generated batch; a later recipe cannot inherit payment.
                                if created_item.is_some() || crafting_inputs_consumed {
                                    result = 1;
                                    break;
                                }
                                screen_handler.update_to_client();

                                let is_player = screen_handler.window_type().is_none();
                                let grid_size = if is_player { 4 } else { 9 };
                                let bedrock_grid_start = if is_player { 28 } else { 32 };
                                for i in 0..grid_size {
                                    let grid_slot_index = 1 + i;
                                    let grid_slot = screen_handler.get_behaviour().slots
                                        [grid_slot_index]
                                        .clone();
                                    let grid_stack = grid_slot.get_cloned_stack();
                                    tracing::info!(
                                        "Crafting Grid slot {i} (slot index {grid_slot_index}): Item ID: {}, Count: {}",
                                        grid_stack.item.id,
                                        grid_stack.item_count
                                    );
                                }

                                let output_slot = screen_handler.get_behaviour().slots[0].clone();
                                let output_stack = output_slot.get_cloned_stack();

                                if output_stack.is_empty()
                                    || repetitions > output_slot.get_max_item_count()
                                {
                                    tracing::warn!("Client sent an invalid crafting request");
                                    result = 1;
                                    break;
                                }

                                let mut total_crafted = output_stack.clone();
                                let Some(count) = total_crafted.item_count.checked_mul(repetitions)
                                else {
                                    result = 1;
                                    break;
                                };
                                total_crafted.item_count = count;
                                created_item = Some(total_crafted);

                                // ResultSlot.onTake: consumption is deferred until the destination can commit.
                                pending_craft = Some((output_stack, repetitions));

                                // Record updates for all grid slots so Bedrock client is notified of consumed ingredients!
                                let is_player = screen_handler.window_type().is_none();
                                let grid_size = if is_player { 4 } else { 9 };
                                for i in 0..grid_size {
                                    let grid_slot_index = 1 + i;
                                    let grid_slot = screen_handler.get_behaviour().slots
                                        [grid_slot_index]
                                        .clone();
                                    let grid_stack = grid_slot.get_cloned_stack();
                                    record_update(
                                        &mut updates,
                                        FullContainerName {
                                            container_name: ContainerName::CraftingInput,
                                            dynamic_id: None,
                                        },
                                        (bedrock_grid_start + i) as u8,
                                        &grid_stack,
                                    );
                                }
                            }
                        }
                        ItemStackRequestAction::CraftResultsDeprecated { .. }
                        | ItemStackRequestAction::MineBlock { .. }
                        | ItemStackRequestAction::BeaconPayment { .. }
                        | ItemStackRequestAction::Create { .. }
                        | ItemStackRequestAction::LabTableCombine
                        | ItemStackRequestAction::Optional { .. }
                        | ItemStackRequestAction::Grindstone { .. }
                        | ItemStackRequestAction::Loom { .. }
                        | ItemStackRequestAction::CraftNonImplemented => {
                            // Successful no-ops to prevent client-side transaction rollbacks
                        }
                    }
                }

                if crafting_inputs_consumed {
                    let is_player = screen_handler.window_type().is_none();
                    let grid_size = if is_player { 4 } else { 9 };
                    let grid_start = if is_player { 28 } else { 32 };
                    for i in 0..grid_size {
                        let stack = screen_handler.get_behaviour().slots[1 + i].get_cloned_stack();
                        record_update(
                            &mut updates,
                            FullContainerName {
                                container_name: ContainerName::CraftingInput,
                                dynamic_id: None,
                            },
                            (grid_start + i) as u8,
                            &stack,
                        );
                    }
                    if let Some(stack) = created_item.take() {
                        unclaimed_outputs.push(stack);
                        for update in &mut updates {
                            if update.container_name.container_name == ContainerName::CreatedOutput
                            {
                                update.count = 0;
                                update.stack_id = VarInt(0);
                            }
                        }
                    }
                }

                correction_needed |= result != 0;
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
                                full_container_name: update.container_name,
                                slots: vec![slot_info],
                            });
                        }
                    }
                }

                responses.push(ItemStackResponseInfo {
                    result,
                    client_request_id: request.request_id,
                    containers: container_infos,
                });
            }

            // Send updates to Java client
            if processed_request {
                screen_handler.send_content_updates();
            }

            // Collect inventory updates if we modified player inventory
            let mut inventory_updated = false;
            for response in &responses {
                if response.result == 0 {
                    for info in &response.containers {
                        if info.full_container_name.container_name == ContainerName::Inventory
                            || info.full_container_name.container_name
                                == ContainerName::CombinedHotBarAndInventory
                            || info.full_container_name.container_name == ContainerName::HotBar
                        {
                            inventory_updated = true;
                        }
                    }
                }
            }

            (
                responses,
                inventory_updated,
                unclaimed_outputs,
                crafting_effects,
            )
        };

        drop(effects);

        // Send Bedrock specific responses and updates
        // Publish the committed request before callbacks can make further inventory edits.
        self.try_enqueue_client_packet(&CItemStackResponse { responses });
        if correction_needed {
            self.correct_stack_request_prediction(player);
        }
        for commit in crafting_effects {
            commit.finish(player.as_ref());
            inventory_updated = true;
        }
        for stack in unclaimed_outputs {
            crate::item::item_utils::give_or_drop(player, stack);
            inventory_updated = true;
        }

        if inventory_updated {
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
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn map_bedrock_container_slot(
    screen_handler: &dyn ScreenHandler,
    container_name: ContainerName,
    slot_id: u8,
) -> Option<usize> {
    let is_player_screen = screen_handler.window_type().is_none();
    let container_slots = screen_handler
        .get_behaviour()
        .slots
        .len()
        .saturating_sub(PlayerInventory::MAIN_SIZE);

    match container_name {
        ContainerName::HotBar => {
            if is_player_screen {
                Some(36 + slot_id as usize)
            } else {
                Some(container_slots + 27 + slot_id as usize)
            }
        }
        ContainerName::Inventory | ContainerName::CombinedHotBarAndInventory => {
            if slot_id < 9 {
                if is_player_screen {
                    Some(36 + slot_id as usize)
                } else {
                    Some(container_slots + 27 + slot_id as usize)
                }
            } else if slot_id < 36 {
                if is_player_screen {
                    Some(slot_id as usize)
                } else {
                    Some(container_slots + (slot_id - 9) as usize)
                }
            } else {
                None
            }
        }
        ContainerName::Armor => (slot_id < 4).then(|| 5 + slot_id as usize),
        ContainerName::Offhand => (slot_id == 0).then_some(45),
        ContainerName::Cursor => None,
        ContainerName::CraftingInput => {
            if is_player_screen {
                if slot_id < 4 {
                    Some(1 + slot_id as usize)
                } else if (28..32).contains(&slot_id) {
                    Some(1 + (slot_id - 28) as usize)
                } else {
                    None
                }
            } else if screen_handler.window_type()
                == Some(pumpkin_data::screen::WindowType::Crafting)
            {
                if slot_id < 9 {
                    Some(1 + slot_id as usize)
                } else if (32..41).contains(&slot_id) {
                    Some(1 + (slot_id - 32) as usize)
                } else {
                    None
                }
            } else {
                None
            }
        }
        // Only merchant previews have physical stock behind CreatedOutput; their Take path pays the trade.
        ContainerName::CreatedOutput => (screen_handler.window_type()
            == Some(pumpkin_data::screen::WindowType::Merchant))
        .then_some(2),
        ContainerName::CraftingOutputPreview => {
            if is_player_screen {
                Some(0)
            } else if let Some(window_type) = screen_handler.window_type() {
                match window_type {
                    pumpkin_data::screen::WindowType::Crafting => Some(0),
                    pumpkin_data::screen::WindowType::Stonecutter => Some(1),
                    pumpkin_data::screen::WindowType::Anvil
                    | pumpkin_data::screen::WindowType::Furnace
                    | pumpkin_data::screen::WindowType::BlastFurnace
                    | pumpkin_data::screen::WindowType::Smoker
                    | pumpkin_data::screen::WindowType::Grindstone
                    | pumpkin_data::screen::WindowType::Merchant => Some(2),
                    pumpkin_data::screen::WindowType::Loom
                    | pumpkin_data::screen::WindowType::Smithing => Some(3),
                    _ => None,
                }
            } else {
                None
            }
        }
        ContainerName::AnvilInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Anvil)
        )
        .then_some(0),
        ContainerName::AnvilMaterial => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Anvil)
        )
        .then_some(1),
        ContainerName::AnvilResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Anvil)
        )
        .then_some(2),
        ContainerName::BeaconPayment => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Beacon)
        )
        .then_some(0),
        ContainerName::BrewingStandResult => (matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::BrewingStand)
        ) && slot_id < 3)
            .then_some(slot_id as usize),
        ContainerName::BrewingStandInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::BrewingStand)
        )
        .then_some(3),
        ContainerName::BrewingStandFuel => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::BrewingStand)
        )
        .then_some(4),
        ContainerName::FurnaceIngredient
        | ContainerName::BlastFurnaceIngredient
        | ContainerName::SmokerIngredient => matches!(
            screen_handler.window_type(),
            Some(
                pumpkin_data::screen::WindowType::Furnace
                    | pumpkin_data::screen::WindowType::BlastFurnace
                    | pumpkin_data::screen::WindowType::Smoker
            )
        )
        .then_some(0),
        ContainerName::FurnaceFuel => matches!(
            screen_handler.window_type(),
            Some(
                pumpkin_data::screen::WindowType::Furnace
                    | pumpkin_data::screen::WindowType::BlastFurnace
                    | pumpkin_data::screen::WindowType::Smoker
            )
        )
        .then_some(1),
        ContainerName::FurnaceResult => matches!(
            screen_handler.window_type(),
            Some(
                pumpkin_data::screen::WindowType::Furnace
                    | pumpkin_data::screen::WindowType::BlastFurnace
                    | pumpkin_data::screen::WindowType::Smoker
            )
        )
        .then_some(2),
        ContainerName::EnchantingInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Enchantment)
        )
        .then_some(0),
        ContainerName::EnchantingMaterial => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Enchantment)
        )
        .then_some(1),
        ContainerName::GrindstoneInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Grindstone)
        )
        .then_some(0),
        ContainerName::GrindstoneAdditional => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Grindstone)
        )
        .then_some(1),
        ContainerName::GrindstoneResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Grindstone)
        )
        .then_some(2),
        ContainerName::LoomInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Loom)
        )
        .then_some(0),
        ContainerName::LoomDye => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Loom)
        )
        .then_some(1),
        ContainerName::LoomMaterial => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Loom)
        )
        .then_some(2),
        ContainerName::LoomResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Loom)
        )
        .then_some(3),
        ContainerName::StonecutterInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Stonecutter)
        )
        .then_some(0),
        ContainerName::StonecutterResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Stonecutter)
        )
        .then_some(1),
        ContainerName::CartographyInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::CartographyTable)
        )
        .then_some(0),
        ContainerName::CartographyAdditional => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::CartographyTable)
        )
        .then_some(1),
        ContainerName::CartographyResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::CartographyTable)
        )
        .then_some(2),
        ContainerName::SmithingTableTemplate => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Smithing)
        )
        .then_some(0),
        ContainerName::SmithingTableInput => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Smithing)
        )
        .then_some(1),
        ContainerName::SmithingTableMaterial => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Smithing)
        )
        .then_some(2),
        ContainerName::SmithingTableResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Smithing)
        )
        .then_some(3),
        ContainerName::TradeIngredient1 | ContainerName::Trade2Ingredient1 => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Merchant)
        )
        .then_some(0),
        ContainerName::TradeIngredient2 | ContainerName::Trade2Ingredient2 => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Merchant)
        )
        .then_some(1),
        ContainerName::TradeResultPreview | ContainerName::Trade2ResultPreview => matches!(
            screen_handler.window_type(),
            Some(pumpkin_data::screen::WindowType::Merchant)
        )
        .then_some(2),
        _ => ((slot_id as usize) < container_slots).then_some(slot_id as usize),
    }
}

pub(crate) struct SlotUpdate {
    pub(crate) container_name: FullContainerName,
    pub(crate) slot_id: u8,
    pub(crate) count: u8,
    pub(crate) stack_id: VarInt,
}

pub(crate) fn record_update(
    updates: &mut Vec<SlotUpdate>,
    container_name: FullContainerName,
    slot_id: u8,
    stack: &ItemStack,
) {
    let count = stack.item_count;
    let stack_id = if stack.is_empty() {
        VarInt(0)
    } else {
        VarInt(stack.uid.get())
    };
    if let Some(existing) = updates
        .iter_mut()
        .find(|u| u.container_name == container_name && u.slot_id == slot_id)
    {
        existing.count = count;
        existing.stack_id = stack_id;
    } else {
        updates.push(SlotUpdate {
            container_name,
            slot_id,
            count,
            stack_id,
        });
    }
}

fn get_slot_stack(
    screen_handler: &dyn ScreenHandler,
    slot_info: &pumpkin_protocol::bedrock::server::item_stack_request::ItemStackRequestSlotInfo,
    created_item: Option<&ItemStack>,
) -> ItemStack {
    if let (ContainerName::CreatedOutput, Some(stack)) =
        (slot_info.container_name.container_name, created_item)
    {
        return stack.clone();
    }
    if slot_info.container_name.container_name == ContainerName::CreatedOutput
        && screen_handler.window_type() != Some(pumpkin_data::screen::WindowType::Merchant)
    {
        return ItemStack::EMPTY.clone();
    }
    if slot_info.container_name.container_name == ContainerName::Cursor {
        return screen_handler
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
    }
    map_bedrock_container_slot(
        screen_handler,
        slot_info.container_name.container_name,
        slot_info.slot_id,
    )
    .map_or_else(
        || ItemStack::EMPTY.clone(),
        |screen_slot| screen_handler.get_behaviour().slots[screen_slot].get_cloned_stack(),
    )
}

#[allow(clippy::unreachable)]
fn update_slot_stack(
    player: &Player,
    screen_handler: &mut dyn ScreenHandler,
    slot_info: &pumpkin_protocol::bedrock::server::item_stack_request::ItemStackRequestSlotInfo,
    expected: &ItemStack,
    new_stack: ItemStack,
) {
    if slot_info.container_name.container_name == ContainerName::CreatedOutput {
        return;
    }
    if slot_info.container_name.container_name == ContainerName::Cursor {
        let mut cursor_lock = screen_handler
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cursor_lock = new_stack;
        return;
    }
    if let Some(screen_slot) = map_bedrock_container_slot(
        screen_handler,
        slot_info.container_name.container_name,
        slot_info.slot_id,
    ) {
        // LivingEntity.detectEquipmentUpdates needs the admitted transition, before the write.
        if !expected.are_equal(&new_stack) {
            super::inventory_commit::queue_equipment_change(
                player,
                screen_handler,
                screen_slot,
                &new_stack,
            );
        }

        screen_handler.set_received_stack(screen_slot, new_stack);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::item::Item;
    use pumpkin_inventory::{
        build_equipment_slots, crafting::crafting_screen_handler::CraftingTableScreenHandler,
        entity_equipment::EntityEquipment,
    };
    use std::sync::Mutex;

    #[tokio::test]
    async fn crafting_table_maps_bedrock_player_inventory_after_its_ten_slots() {
        let inventory = Arc::new(PlayerInventory::new(
            Arc::new(Mutex::new(EntityEquipment::new())),
            Arc::new(build_equipment_slots()),
        ));
        let handler = CraftingTableScreenHandler::new(1, &inventory, None);

        assert_eq!(
            map_bedrock_container_slot(&handler, ContainerName::Inventory, 26),
            Some(27)
        );
        assert_eq!(
            map_bedrock_container_slot(&handler, ContainerName::HotBar, 0),
            Some(37)
        );
        assert_eq!(
            map_bedrock_container_slot(&handler, ContainerName::CraftingInput, 32),
            Some(1)
        );
        assert_eq!(
            map_bedrock_container_slot(&handler, ContainerName::CraftingInput, 40),
            Some(9)
        );
    }

    #[test]
    fn item_stack_response_uses_the_authoritative_stack_id() {
        let stack = ItemStack::new(3, &Item::SPRUCE_DOOR);
        let container = FullContainerName {
            container_name: ContainerName::Cursor,
            dynamic_id: None,
        };
        let mut updates = Vec::new();

        record_update(&mut updates, container.clone(), 0, &stack);
        assert_eq!(updates[0].count, 3);
        assert_eq!(updates[0].stack_id, VarInt(stack.uid.get()));

        record_update(&mut updates, container, 0, ItemStack::EMPTY);
        assert_eq!(updates[0].count, 0);
        assert_eq!(updates[0].stack_id, VarInt(0));
    }
}
