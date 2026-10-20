use super::{
    Arc, BedrockClient, BlockActionResult, BlockDirection, BlockHitResult, BlocksAttacksImpl,
    ConsumableImpl, EquipmentSlot, EquippableImpl, FoodImpl, Hand, HandSlot, InteractAction,
    InventoryPlayer, ItemStack, LiveHands, Ordering, Player, PlayerInteractEvent, VarInt,
    should_try_block_placement,
};
use crate::{server::Server, world::World};
use pumpkin_data::Block;
use pumpkin_inventory::Inventory;
use pumpkin_protocol::bedrock::server::inventory_transaction::UseItemTransactionData;
use pumpkin_protocol::codec::item_nbt::project_item_nbt_for_client;

// UseOnContext retains the admitted hand/stack and hit result through useItemOn callbacks.
struct AdmittedItemUse<'a> {
    player: &'a Arc<Player>,
    server: Arc<Server>,
    world: Arc<World>,
    data: &'a UseItemTransactionData,
    hand: Hand,
    selected: u8,
    before: ItemStack,
    hands: &'a LiveHands,
    block: &'static Block,
    face: BlockDirection,
}

impl AdmittedItemUse<'_> {
    fn live_stack(&self, before: &ItemStack) -> Option<ItemStack> {
        BedrockClient::stack_for_hand_use(self.player, self.hand, self.selected, before, self.hands)
    }

    fn finish(&self, after: &ItemStack) {
        BedrockClient::finish_block_use(
            self.player,
            self.hand,
            self.selected,
            &self.before,
            after,
            self.hands,
        );
    }

    const fn hit(&self) -> BlockHitResult<'_> {
        BlockHitResult {
            face: &self.face,
            cursor_pos: &self.data.click_position,
        }
    }
}

impl BedrockClient {
    pub(super) fn process_admitted_item(
        &self,
        player: &Arc<Player>,
        data: &UseItemTransactionData,
        selected: u8,
        stack: Option<ItemStack>,
        hands: &LiveHands,
    ) -> bool {
        let world = player.world();
        let block = world.get_block(&data.block_position);
        let Some(server) = world.server.upgrade() else {
            return false;
        };
        // Mirrors Java ServerGamePacketListenerImpl.handleUseItemOn before block-use hooks.
        if data.action_type.0 == 0 && world.is_in_spawn_protection(player, &data.block_position) {
            self.correct_spawn_protected_block_use(
                player,
                data,
                selected,
                match data.hand {
                    HandSlot::Mainhand => Hand::Right,
                    HandSlot::Offhand => Hand::Left,
                },
            );
            return true;
        }
        if player.is_spectator() {
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
        if !matches!(data.action_type.0, 0 | 1) {
            return true;
        }
        let Some(before) = stack else {
            self.correct_rejected_block_use(player, data);
            return false;
        };
        let context = AdmittedItemUse {
            player,
            server,
            world,
            data,
            selected,
            before,
            hands,
            block,
            hand: match data.hand {
                HandSlot::Mainhand => Hand::Right,
                HandSlot::Offhand => Hand::Left,
            },
            face: BlockDirection::try_from(i32::from(data.block_face))
                .unwrap_or(BlockDirection::Up),
        };
        let _mutation = if data.action_type.0 == 0 {
            let Some(mutation) = context
                .world
                .block_entity_mutation_permit(data.block_position)
            else {
                self.correct_rejected_block_use(player, data);
                return false;
            };
            Some(mutation)
        } else {
            None
        };
        // ServerPlayerGameMode.useItemOn/useItem dispatch the retained server hand.
        if data.action_type.0 == 0 {
            self.use_admitted_item_on(&context)
        } else {
            self.use_admitted_item(&context)
        }
    }

    fn correct_spawn_protected_block_use(
        &self,
        player: &Player,
        data: &UseItemTransactionData,
        selected: u8,
        hand: Hand,
    ) {
        let face =
            BlockDirection::try_from(i32::from(data.block_face)).unwrap_or(BlockDirection::Up);
        self.correct_denied_placement(player, data.block_position, face);

        let world = player.world();
        let offset = face.to_offset();
        let adjacent = pumpkin_util::math::position::BlockPos::new(
            data.block_position.0.x.wrapping_add(offset.x),
            data.block_position.0.y.wrapping_add(offset.y),
            data.block_position.0.z.wrapping_add(offset.z),
        );
        for position in [data.block_position, adjacent] {
            let state_id = world.get_block_state_id(&position);
            if let Some(block_entity) = world.get_block_entity(&position)
                && let Some(actor_data) = block_entity.bedrock_block_actor_data(state_id)
            {
                self.try_enqueue_client_packet(
                    &pumpkin_protocol::bedrock::client::block_actor_data::CBlockActorData::new(
                        position,
                        project_item_nbt_for_client(actor_data),
                    ),
                );
            }
        }

        let slot = super::hand_inventory_slot(hand, selected);
        player.sync_hand_slot(slot, player.inventory().get_stack(slot));
    }

    fn use_admitted_item_on(&self, context: &AdmittedItemUse<'_>) -> bool {
        let Some(mut held) = context.live_stack(&context.before) else {
            self.correct_rejected_block_use(context.player, context.data);
            return false;
        };
        let equipment = if context.hand == Hand::Right {
            EquipmentSlot::MAIN_HAND
        } else {
            EquipmentSlot::OFF_HAND
        };
        let result = context.server.block_registry.use_with_item(
            context.block,
            context.player,
            &context.data.block_position,
            &context.hit(),
            &mut held,
            &equipment,
            &context.server,
            &context.world,
        );
        if result.consumes_action() {
            context.finish(&held);
            return true;
        }
        match self.try_default_block_action(context, &held, &result) {
            None => return false,
            Some(true) => return true,
            Some(false) => {}
        }
        let Some(denied) = self.use_admitted_stack_on_block(context, held) else {
            return false;
        };
        if denied {
            self.correct_denied_placement(
                context.player,
                context.data.block_position,
                context.face,
            );
        }
        true
    }

    fn try_default_block_action(
        &self,
        context: &AdmittedItemUse<'_>,
        held: &ItemStack,
        result: &BlockActionResult,
    ) -> Option<bool> {
        // ServerPlayerGameMode.useItemOn only tries useWithoutItem for MAIN_HAND.
        if !matches!(result, BlockActionResult::PassToDefaultBlockAction)
            || context.hand != Hand::Right
        {
            return Some(false);
        }
        if context.live_stack(&context.before).is_none() {
            self.correct_rejected_block_use(context.player, context.data);
            return None;
        }
        let result = context.server.block_registry.on_use(
            context.block,
            context.player,
            &context.data.block_position,
            &context.hit(),
            &context.server,
            &context.world,
        );
        if result.consumes_action() {
            context.finish(held);
            return Some(true);
        }
        Some(false)
    }

    fn use_admitted_stack_on_block(
        &self,
        context: &AdmittedItemUse<'_>,
        mut stack: ItemStack,
    ) -> Option<bool> {
        if stack.is_empty() {
            context.finish(&stack);
            return Some(false);
        }
        context.player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(stack.item.id),
            1,
        );
        if context.live_stack(&context.before).is_none() {
            self.correct_rejected_block_use(context.player, context.data);
            return None;
        }
        let result = context.server.item_registry.use_on_block_with_hand(
            &mut stack,
            context.player,
            context.data.block_position,
            context.face,
            context.data.click_position,
            context.block,
            &context.server,
            context.hand,
        );
        let denied = if should_try_block_placement(&result)
            && let Some(block) = Block::from_item_id(stack.item.id)
        {
            let denied = Self::place_admitted_block(context, block);
            // BlockItem.place has already claimed/refunded the source; only publish live hands.
            Self::sync_live_hands(
                context.player,
                context.hand,
                context.selected,
                context.hands,
            );
            denied
        } else {
            context.finish(&stack);
            matches!(result, BlockActionResult::Fail)
        };
        Some(denied)
    }

    fn place_admitted_block(context: &AdmittedItemUse<'_>, block: &'static Block) -> bool {
        // BlockItem.place receives the retained UseOnContext stack and requested hand.
        let packet = pumpkin_protocol::java::server::play::SUseItemOn {
            hand: VarInt(i32::from(context.hand == Hand::Left)),
            position: context.data.block_position,
            face: VarInt(i32::from(context.data.block_face)),
            cursor_pos: context.data.click_position,
            inside_block: false,
            is_against_world_border: false,
            sequence: VarInt(0),
        };
        match context.server.block_registry.place_block_with_stack(
            context.player,
            block,
            &context.server,
            &packet,
            context.data.block_position,
            context.face,
            &context.before,
            super::hand_inventory_slot(context.hand, context.selected),
        ) {
            Ok(Some(_)) => false,
            Ok(None) | Err(_) => true,
        }
    }

    fn use_admitted_item(&self, context: &AdmittedItemUse<'_>) -> bool {
        let Some(mut held) = context.live_stack(&context.before) else {
            return false;
        };
        // Consumable/BucketItem/BottleItem record their own successful uses.
        if !held.is_empty() && !context.server.item_registry.records_item_use_stat(&held) {
            context.player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Used,
                i32::from(held.item.id),
                1,
            );
        }
        if context.live_stack(&context.before).is_none() {
            return false;
        }
        let mut event = PlayerInteractEvent::new(
            context.player,
            InteractAction::RightClickAir,
            &Block::AIR,
            None,
        );
        let stack_for_use = held.clone();
        if !Self::item_on_cooldown(context.player, &held)
            && !self.prepare_air_use(context, &mut held)
        {
            return false;
        }
        context
            .server
            .plugin_manager
            .fire_blocking(&context.server, &mut event);
        if !event.cancelled {
            if context.live_stack(&held).is_none() {
                return false;
            }
            context
                .server
                .item_registry
                .on_use(&stack_for_use, context.player, context.hand);
        }
        Self::sync_live_hands(
            context.player,
            context.hand,
            context.selected,
            context.hands,
        );
        !event.cancelled
    }

    fn item_on_cooldown(player: &Player, held: &ItemStack) -> bool {
        held.get_use_cooldown().is_some_and(|cooldown| {
            let group = cooldown
                .cooldown_group
                .clone()
                .unwrap_or_else(|| held.item.registry_key.to_string());
            player.is_on_cooldown(&group)
        })
    }

    fn prepare_air_use(&self, context: &AdmittedItemUse<'_>, held: &mut ItemStack) -> bool {
        let already_using = context
            .player
            .living_entity
            .item_use_time
            .load(Ordering::Relaxed)
            > 0
            && context
                .player
                .living_entity
                .item_in_use
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(|item| item.are_items_and_components_equal(held));
        if !already_using
            && (held.get_data_component::<ConsumableImpl>().is_some()
                || held.get_data_component::<BlocksAttacksImpl>().is_some())
        {
            if held
                .get_data_component::<FoodImpl>()
                .is_none_or(|food| context.player.can_eat(food.can_always_eat))
            {
                context.player.living_entity.set_active_hand(
                    context.hand,
                    held.clone(),
                    held.get_max_use_time(),
                );
            } else {
                self.correct_rejected_food_use(context.player);
            }
        }
        Self::equip_air_use(context, held)
    }

    fn equip_air_use(context: &AdmittedItemUse<'_>, held: &mut ItemStack) -> bool {
        let Some(equippable) = held.get_data_component::<EquippableImpl>() else {
            return true;
        };
        let slot = equippable.slot;
        let change = {
            let equipment = context
                .player
                .inventory()
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            !equipment.get(slot).are_items_and_components_equal(held)
        };
        if !change {
            return true;
        }
        let Some((new_held, equipped, mut displaced)) =
            Self::equip_admitted_hand(context.player, context.hand, context.selected, held, slot)
        else {
            Self::sync_live_hands(
                context.player,
                context.hand,
                context.selected,
                context.hands,
            );
            return false;
        };
        *held = new_held;
        // Equippable.swapWithEquipmentSlot returns displaced equipment after the owner operation unlocks.
        context
            .player
            .inventory()
            .insert_stack_anywhere(&mut displaced);
        if !displaced.is_empty() {
            context.player.drop_item(displaced);
        }
        context.player.enqueue_equipment_change(slot, &equipped);
        true
    }
}
