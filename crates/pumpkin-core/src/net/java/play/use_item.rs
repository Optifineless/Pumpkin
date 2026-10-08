#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_util::version::JavaMinecraftVersion;

impl JavaClient {
    pub fn handle_use_item(&self, player: &Arc<Player>, use_item: &SUseItem, server: &Arc<Server>) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();

        let inventory = player.inventory();
        let Ok(hand) = Hand::from_packet_id(use_item.hand.0) else {
            self.try_kick(&TextComponent::text("InvalidHand"));
            return;
        };
        if self.version.load() >= JavaMinecraftVersion::V_1_21
            && (!use_item.yaw.is_finite() || !use_item.pitch.is_finite())
        {
            self.try_kick(&TextComponent::text("Invalid item use rotation"));
            return;
        }
        self.update_sequence(use_item.sequence.0);

        let mut item_in_hand = inventory.get_stack_in_hand(hand);
        // ServerPlayerGameMode.useItem rejects cooldowns before starting any item use.
        if !crate::entity::item_use::item_use_allowed(&item_in_hand, |group| {
            player.is_on_cooldown(group)
        }) {
            return;
        }

        let mut consume_event =
            crate::plugin::api::events::player::player_item_consume::PlayerItemConsumeEvent::new(
                player.clone(),
                item_in_hand.item.registry_key.to_string(),
            );
        server
            .plugin_manager
            .fire_blocking(server, &mut consume_event);
        if consume_event.cancelled {
            return;
        }

        let hit_result = player.world().raycast(
            player.eye_position(),
            player.eye_position().add(
                &(Vector3::rotation_vector(f64::from(use_item.pitch), f64::from(use_item.yaw))
                    * 4.5),
            ),
            |pos, world| {
                let block = world.get_block(pos);
                block != &Block::AIR && block != &Block::WATER && block != &Block::LAVA
            },
        );

        let event = if let Some((hit_pos, _hit_dir)) = hit_result {
            PlayerInteractEvent::new(
                player,
                InteractAction::RightClickBlock,
                player.world().get_block(&hit_pos),
                Some(hit_pos),
            )
        } else {
            PlayerInteractEvent::new(player, InteractAction::RightClickAir, &Block::AIR, None)
        };
        let (use_yaw, use_pitch) = if self.version.load() >= JavaMinecraftVersion::V_1_21 {
            (use_item.yaw, use_item.pitch)
        } else {
            player.rotation()
        };

        send_cancellable_blocking! {{
            server;
            event;
            'after: {
                item_in_hand = inventory.get_stack_in_hand(hand);
                let stack_for_use = item_in_hand.clone();
                if stack_for_use.is_empty() || !crate::entity::item_use::item_use_allowed(
                    &stack_for_use, |group| player.is_on_cooldown(group),
                ) {
                    return;
                }
                Self::prepare_hand_item_for_use(player, hand, &mut item_in_hand);
                server
                    .item_registry
                    .on_use_with_rotation(&stack_for_use, player, use_yaw, use_pitch, hand);
            }
        }}
    }

    fn prepare_hand_item_for_use(player: &Arc<Player>, hand: Hand, held: &mut ItemStack) {
        let inventory = player.inventory();

        let consumable = held.get_data_component::<ConsumableImpl>().is_some();
        if (consumable || held.get_data_component::<BlocksAttacksImpl>().is_some())
            && held
                .get_data_component::<FoodImpl>()
                .is_none_or(|food| player.can_eat(food.can_always_eat))
        {
            if consumable && held.get_max_use_time() == 0 {
                player
                    .living_entity
                    .consume_instantly(player.as_ref(), hand, held);
            } else {
                player
                    .living_entity
                    .set_active_hand(hand, held.clone(), held.get_max_use_time());
            }
        }
        let equipment_slot = held
            .get_data_component::<EquippableImpl>()
            .filter(|equippable| equippable.swappable)
            .map(|equippable| equippable.slot.clone());
        if let Some(slot) = equipment_slot {
            // The equipment lock has to be released before touching the hand again:
            // the off hand lives in the same map, so holding it here would deadlock.
            let current_equipped = inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&slot);
            if current_equipped.are_items_and_components_equal(held) {
                return;
            }

            player.enqueue_equipment_change(&slot, held);
            // Equippable.swapWithEquipmentSlot awards the original item's successful use.
            player.increment_stat(StatisticCategory::Used, i32::from(held.item.id), 1);

            let equipped = if current_equipped.is_empty() {
                let equipped = held.clone();
                held.decrement_unless_creative(player.gamemode.load(), 1);
                equipped
            } else {
                std::mem::replace(held, current_equipped)
            };
            inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(&slot, equipped);
            inventory.set_stack_in_hand(hand, held.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hand_use_instant_consumable_keeps_another_hands_use_active() {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let bow = ItemStack::new(1, &Item::BOW);
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, bow.clone());
        player.living_entity.set_active_hand(Hand::Right, bow, 10);
        let mut milk = ItemStack::new(1, &Item::MILK_BUCKET);
        milk.get_data_component_mut::<ConsumableImpl>()
            .unwrap()
            .consume_seconds = 0.0;
        player
            .inventory()
            .set_stack_in_hand(Hand::Left, milk.clone());
        JavaClient::prepare_hand_item_for_use(player, Hand::Left, &mut milk);
        assert_eq!(player.inventory().off_hand_item().item, &Item::BUCKET);
        assert_eq!(
            *player.living_entity.active_hand.lock().unwrap(),
            Some(Hand::Right)
        );
        assert_eq!(
            player.living_entity.item_use_time.load(Ordering::Relaxed),
            10
        );
        assert!(world.level.shutdown().await.is_ok());
    }
}
