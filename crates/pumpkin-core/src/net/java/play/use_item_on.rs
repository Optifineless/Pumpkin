#[allow(clippy::wildcard_imports)]
use super::*;
use crate::item::registry::should_try_block_placement;

impl JavaClient {
    #[allow(clippy::too_many_lines)]
    pub fn handle_use_item_on(
        &self,
        player: &Arc<Player>,
        use_item_on: &SUseItemOn,
        server: &Arc<Server>,
    ) -> Result<(), BlockPlacingError> {
        if !player.has_client_loaded() {
            return Ok(());
        }
        player.update_last_action_time();
        self.update_sequence(use_item_on.sequence.0);

        let position = use_item_on.position;
        let cursor_pos = use_item_on.cursor_pos;

        let mut should_try_decrement = false;

        if !player.can_interact_with_block_at(&position, 1.0) {
            // TODO: maybe log?
            return Err(BlockPlacingError::BlockOutOfReach);
        }

        let Ok(face) = BlockDirection::try_from(use_item_on.face.0) else {
            return Err(BlockPlacingError::InvalidBlockFace);
        };

        let Ok(hand) = Hand::from_packet_id(use_item_on.hand.0) else {
            return Err(BlockPlacingError::InvalidHand);
        };

        if player.gamemode.load() == GameMode::Spectator {
            let entity = &player.get_entity();
            let world = entity.world.load_full();
            let block = world.get_block(&position);

            let event = PlayerInteractEvent::new(
                player,
                InteractAction::RightClickBlock,
                block,
                Some(position),
            );

            send_cancellable_blocking! {{
                server;
                event;
                'cancelled: {
                    let state_id = world.get_block_state_id(&position);
                    player.try_send_client_packet(&CBlockUpdate::new(
                        position,
                        VarInt(i32::from(state_id.as_u16())),
                    ));
                    return Ok(());
                }
            }}

            if let Some(factory) = server
                .block_registry
                .get_screen_handler_factory(block, player, &position, server, &world)
            {
                player.open_handled_screen(factory.as_ref(), Some(position));
            }
            return Ok(());
        }

        let inventory = player.inventory();
        let held_item = inventory.held_item();
        let off_hand_item = inventory.off_hand_item();
        let held_item_empty = held_item.is_empty();
        let off_hand_item_empty = off_hand_item.is_empty();

        let entity = &player.get_entity();
        let world = entity.world.load_full();
        let block = world.get_block(&position);

        let event = PlayerInteractEvent::new(
            player,
            InteractAction::RightClickBlock,
            block,
            Some(position),
        );

        send_cancellable_blocking! {{
            server;
            event;
            'cancelled: {
                let state_id = world.get_block_state_id(&position);
                player.try_send_client_packet(&CBlockUpdate::new(
                    position,
                    VarInt(i32::from(state_id.as_u16())),
                ));
                return Ok(());
            }
        }}

        // Read after plugin callbacks, which may change the inventory themselves.
        let source_slot = super::hand_use_result::hand_slot(player, hand);
        let mut item = inventory.get_stack_in_hand(hand);
        let before = item.clone();
        // ItemStack.getItem hides the backing item of a depleted stack from block interactions.
        if item.is_empty() {
            item.clear();
        }
        let equipment_slot = if matches!(hand, Hand::Right) {
            EquipmentSlot::MAIN_HAND
        } else {
            EquipmentSlot::OFF_HAND
        };

        let sneaking = player.get_entity().is_sneaking();

        // Code based on the java class ServerPlayerInteractionManager
        if !(sneaking && (!held_item_empty || !off_hand_item_empty)) {
            let result = Self::call_use_item_on(
                player,
                &position,
                &cursor_pos,
                face,
                &mut item,
                &equipment_slot,
                &world,
                block,
                server,
            );
            // ServerPlayerGameMode.useItemOn supplies the live hand stack even on consuming returns.
            super::hand_use_result::write_back_used_item(player, hand, source_slot, &before, &item);
            if result.consumes_action() || matches!(result, BlockActionResult::Fail) {
                if matches!(result, BlockActionResult::SuccessServer) {
                    player.swing_hand(hand, true);
                }
                return Ok(());
            }
        }

        let source_slot = super::hand_use_result::hand_slot(player, hand);
        item = inventory.get_stack_in_hand(hand);

        // ServerPlayerGameMode.useItemOn checks cooldowns before item use or placement.
        if item.is_empty()
            || !crate::entity::item_use::item_use_allowed(&item, |group| {
                player.is_on_cooldown(group)
            })
        {
            return Ok(());
        }

        let before = item.clone();

        let state_before = world.get_block_state_id(&position);
        let item_result = server.item_registry.use_on_block_with_hand(
            &mut item, player, position, face, cursor_pos, block, server, hand,
        );

        if should_try_block_placement(&item_result) {
            // Check if the item is a block, because not every item can be placed :D
            let item_id = item.item.id;
            if let Some(block) = Block::from_item_id(item_id) {
                match Self::run_is_block_place(player, block, server, use_item_on, position, face) {
                    Ok(placed) => should_try_decrement = placed,
                    Err(error) => {
                        super::hand_use_result::write_back_used_item(
                            player,
                            hand,
                            source_slot,
                            &before,
                            &item,
                        );
                        return Err(error);
                    }
                }
            }
        }

        if should_try_decrement {
            // TODO: Config
            // Decrease block count
            if player.gamemode.load() != GameMode::Creative {
                item.decrement(1);
            }
        }

        super::hand_use_result::write_back_used_item(player, hand, source_slot, &before, &item);
        if item_result.consumes_action() || should_try_decrement {
            if !before.is_empty() {
                player.increment_stat(StatisticCategory::Used, i32::from(before.item.id), 1);
            }
            super::hand_use_result::trigger_item_used_on_block(
                player,
                position,
                &before,
                state_before,
            );
        }
        if matches!(item_result, BlockActionResult::SuccessServer) {
            player.swing_hand(hand, true);
        }

        Ok(())
    }

    #[expect(clippy::too_many_arguments)]
    fn call_use_item_on(
        player: &Arc<Player>,
        position: &BlockPos,
        cursor_pos: &Vector3<f32>,
        face: BlockDirection,
        held_item: &mut ItemStack,
        equipment_slot: &EquipmentSlot,
        world: &Arc<World>,
        block: &Block,
        server: &Arc<Server>,
    ) -> BlockActionResult {
        let before = held_item.clone();
        let state_before = world.get_block_state_id(position);
        let result = server.block_registry.use_with_item(
            block,
            player,
            position,
            &BlockHitResult {
                face: &face,
                cursor_pos,
            },
            held_item,
            equipment_slot,
            server,
            world,
        );

        if matches!(result, BlockActionResult::Fail) {
            return result;
        }
        if result.consumes_action() {
            super::hand_use_result::trigger_item_used_on_block(
                player,
                *position,
                &before,
                state_before,
            );
            return result;
        }

        if matches!(result, BlockActionResult::PassToDefaultBlockAction)
            && equipment_slot == &EquipmentSlot::MAIN_HAND
        {
            let result = server.block_registry.on_use(
                block,
                player,
                position,
                &BlockHitResult {
                    face: &face,
                    cursor_pos,
                },
                server,
                world,
            );

            if result.consumes_action() {
                player.trigger_advancement(crate::entity::player::advancement::trigger::AdvancementTrigger::DefaultBlockUse { position: *position });
                return result;
            }
        }

        BlockActionResult::Pass
    }

    fn run_is_block_place(
        player: &Arc<Player>,
        block: &'static Block,
        server: &Arc<Server>,
        use_item_on: &SUseItemOn,
        location: BlockPos,
        face: BlockDirection,
    ) -> Result<bool, BlockPlacingError> {
        match server
            .block_registry
            .place_block(player, block, server, use_item_on, location, face)
        {
            Ok(Some((final_block_pos, new_state))) => {
                player.try_send_client_packet(&CBlockUpdate::new(
                    final_block_pos,
                    VarInt(i32::from(new_state.as_u16())),
                ));
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(crate::block::registry::BlockPlacingError::InvalidGamemode) => {
                Err(BlockPlacingError::InvalidGamemode)
            }
            Err(crate::block::registry::BlockPlacingError::BlockOutOfWorld) => {
                Err(BlockPlacingError::BlockOutOfWorld)
            }
        }
    }
}
