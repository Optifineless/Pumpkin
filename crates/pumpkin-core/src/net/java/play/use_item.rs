#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_data::data_component_impl::{EnchantmentsImpl, IDSet};
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::Taggable;
use pumpkin_inventory::Inventory;
use pumpkin_util::version::JavaMinecraftVersion;

enum SwappableItemUseResult {
    NotHandled,
    Pass,
    Fail,
    Success,
}

struct ItemUseHit {
    bottle_use: bool,
    position: Option<pumpkin_util::math::position::BlockPos>,
}

fn item_use_hit(
    world: &Arc<World>,
    player: &Player,
    stack: &ItemStack,
    yaw: f32,
    pitch: f32,
) -> ItemUseHit {
    let bottle_use = stack.item == &Item::GLASS_BOTTLE;
    let position = if bottle_use {
        let main_hand = player.inventory().get_stack_in_hand(Hand::Right);
        crate::item::items::glass_bottle::GlassBottleItem::use_hit(
            world, player, &main_hand, yaw, pitch,
        )
    } else {
        world
            .raycast(
                player.eye_position(),
                player.eye_position().add(
                    &(Vector3::rotation_vector(f64::from(pitch), f64::from(yaw))
                        * player.block_interaction_range()),
                ),
                |pos, world| {
                    let block = world.get_block(pos);
                    block != &Block::AIR && block != &Block::WATER && block != &Block::LAVA
                },
            )
            .map(|(position, _)| position)
    };
    ItemUseHit {
        bottle_use,
        position,
    }
}

fn item_interact_event(
    player: &Arc<Player>,
    world: &World,
    position: Option<pumpkin_util::math::position::BlockPos>,
) -> PlayerInteractEvent {
    position.map_or_else(
        || PlayerInteractEvent::new(player, InteractAction::RightClickAir, &Block::AIR, None),
        |hit_pos| {
            PlayerInteractEvent::new(
                player,
                InteractAction::RightClickBlock,
                world.get_block(&hit_pos),
                Some(hit_pos),
            )
        },
    )
}

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

        if self.reject_spawn_protected_bucket_use(player, &item_in_hand, hand, use_item, server) {
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

        let (use_yaw, use_pitch) = if self.version.load() >= JavaMinecraftVersion::V_1_21 {
            (use_item.yaw, use_item.pitch)
        } else {
            player.rotation()
        };
        let stack_for_interact = inventory.get_stack_in_hand(hand);
        let world = player.world();
        let hit = item_use_hit(&world, player, &stack_for_interact, use_yaw, use_pitch);
        let event = item_interact_event(player, &world, hit.position);

        send_cancellable_blocking! {{
            server;
            event;
            'after: {
                if hit.bottle_use {
                    if crate::entity::item_use::item_use_allowed(&stack_for_interact, |group| {
                        player.is_on_cooldown(group)
                    }) {
                        crate::item::items::glass_bottle::GlassBottleItem::use_packet_snapshot(
                            player,
                            hand,
                            &stack_for_interact,
                            hit.position,
                        );
                    }
                    return;
                }
                item_in_hand = inventory.get_stack_in_hand(hand);
                let stack_for_use = item_in_hand.clone();
                if stack_for_use.is_empty() || !crate::entity::item_use::item_use_allowed(
                    &stack_for_use, |group| player.is_on_cooldown(group),
                ) {
                    return;
                }
                // Do not use a bottle installed by the interaction callback for this packet.
                if stack_for_use.item == &Item::GLASS_BOTTLE {
                    return;
                }
                match Self::prepare_hand_item_for_use(player, hand, &mut item_in_hand) {
                    SwappableItemUseResult::NotHandled => {
                        server.item_registry.on_use_with_rotation(
                            &stack_for_use,
                            player,
                            use_yaw,
                            use_pitch,
                            hand,
                        );
                    }
                    SwappableItemUseResult::Pass
                    | SwappableItemUseResult::Fail
                    | SwappableItemUseResult::Success => {}
                }
            }
        }}
    }

    fn reject_spawn_protected_bucket_use(
        &self,
        player: &Arc<Player>,
        stack: &ItemStack,
        hand: Hand,
        use_item: &SUseItem,
        server: &Arc<Server>,
    ) -> bool {
        // BucketItem.use checks mayInteract on the bucket-mode ray hit before bucket effects.
        let is_bucket = server
            .item_registry
            .get_pumpkin_item(stack.item.id)
            .is_some_and(|behaviour| {
                let behaviour = behaviour.as_any();
                behaviour.is::<crate::item::items::bucket::EmptyBucketItem>()
                    || behaviour.is::<crate::item::items::bucket::FilledBucketItem>()
            });
        if !is_bucket {
            return false;
        }

        let (yaw, pitch) = if self.version.load() >= JavaMinecraftVersion::V_1_21 {
            (use_item.yaw, use_item.pitch)
        } else {
            player.rotation()
        };
        let world = player.world();
        let Some((position, face)) =
            crate::item::items::bucket::bucket_use_hit(&world, player, stack, yaw, pitch)
        else {
            return false;
        };
        if !world.is_in_spawn_protection(player, &position) {
            return false;
        }

        Self::correct_spawn_protected_use(player, &world, position, face, hand);
        true
    }

    fn prepare_hand_item_for_use(
        player: &Arc<Player>,
        hand: Hand,
        held: &mut ItemStack,
    ) -> SwappableItemUseResult {
        let inventory = player.inventory();

        let consumable = held.get_data_component::<ConsumableImpl>().is_some();
        if consumable {
            Self::start_consuming(player, hand, held);
        } else if held.get_data_component::<BlocksAttacksImpl>().is_some()
            && held
                .get_data_component::<FoodImpl>()
                .is_none_or(|food| player.can_eat(food.can_always_eat))
        {
            player
                .living_entity
                .set_active_hand(hand, held.clone(), held.get_max_use_time());
        }
        let Some(equippable) = held
            .get_data_component::<EquippableImpl>()
            .filter(|equippable| equippable.swappable)
        else {
            return SwappableItemUseResult::NotHandled;
        };
        let slot = equippable.slot.clone();

        // Item.use returns Equippable.swapWithEquipmentSlot's PASS/FAIL/SUCCESS directly.
        if !can_be_equipped_by(equippable, player.get_entity().entity_type) {
            return SwappableItemUseResult::Pass;
        }

        let owner = inventory.slot_transaction(0);
        let guard = owner.lock();
        let current_hand = inventory.get_stack_in_hand(hand);
        if current_hand.uid != held.uid || !current_hand.are_equal(held) {
            return SwappableItemUseResult::Fail;
        }
        // The equipment lock has to be released before touching the hand again:
        // the off hand lives in the same map, so holding it here would deadlock.
        let current_equipped = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&slot);
        if current_equipped.are_items_and_components_equal(held)
            || (!player.is_creative()
                && current_equipped
                    .get_data_component::<EnchantmentsImpl>()
                    .is_some_and(|enchantments| {
                        enchantments.enchantment.iter().any(|(enchantment, level)| {
                            *level > 0 && enchantment.effects.prevent_armor_change
                        })
                    }))
        {
            return SwappableItemUseResult::Fail;
        }

        let used = held.item.id;
        let gamemode = player.gamemode.load();
        let mut displaced = ItemStack::EMPTY.clone();
        // Equippable.swapWithEquipmentSlot consumes one item for every stack larger than one.
        let equipped = if held.item_count > 1 {
            displaced = current_equipped;
            let equipped = held.copy_with_count(1);
            held.decrement_unless_creative(gamemode, 1);
            equipped
        } else {
            let equipped = held.clone();
            if current_equipped.is_empty() {
                held.decrement_unless_creative(gamemode, 1);
            } else {
                *held = current_equipped;
            }
            equipped
        };
        let equipped_notification = equipped.clone();
        inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&slot, equipped);
        inventory.set_stack_in_hand(hand, held.clone());
        // PlayerInventory insertion only mutates local slots under this same reentrant owner.
        if !displaced.is_empty() {
            inventory.insert_stack_anywhere(&mut displaced);
        }
        drop(guard);

        // The stacked branch drops displaced equipment only when inventory had no room.
        if !displaced.is_empty() {
            player.drop_item(displaced);
        }
        player.enqueue_equipment_change(&slot, &equipped_notification);
        // Equippable.swapWithEquipmentSlot awards the original item's successful use.
        player.increment_stat(StatisticCategory::Used, i32::from(used), 1);
        SwappableItemUseResult::Success
    }
}

/// Mirrors Equippable.canBeEquippedBy's optional allowed-entity set.
fn can_be_equipped_by(equippable: &EquippableImpl, entity_type: &'static EntityType) -> bool {
    match &equippable.allowed_entities {
        None => true,
        Some(IDSet::Tag(tag)) => entity_type.is_tagged_with(tag).unwrap_or(false),
        Some(IDSet::IDs(types)) => types.contains(&entity_type),
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn java_use_item_does_not_equip_or_duplicate_llama_carpet_on_player() {
        use pumpkin_protocol::codec::var_int::VarInt;

        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let inventory = player.inventory();
        inventory.set_slot(0, ItemStack::new(2, &Item::BLACK_CARPET));
        inventory.set_slot(1, ItemStack::new(1, &Item::WHITE_CARPET));

        let black_use = SUseItem {
            hand: VarInt(0),
            sequence: VarInt(0),
            yaw: 0.0,
            pitch: 0.0,
        };
        fixture
            .client()
            .handle_use_item(player, &black_use, &server);

        // Put the remaining black carpet away, then use the white carpet as in issue #90.
        inventory.set_slot(2, inventory.get_slot(0));
        inventory.set_slot(0, ItemStack::EMPTY.clone());
        inventory
            .selected_slot
            .store(1, std::sync::atomic::Ordering::Relaxed);
        let white_use = SUseItem {
            sequence: VarInt(1),
            ..black_use
        };
        fixture
            .client()
            .handle_use_item(player, &white_use, &server);

        assert_eq!(inventory.count_item(&Item::BLACK_CARPET), 2);
        let body = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        assert!(body.is_empty());
        assert_eq!(
            inventory.get_stack_in_hand(Hand::Right).item,
            &Item::WHITE_CARPET
        );

        assert!(world.level.shutdown().await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn java_use_item_equips_only_one_item_from_a_permitted_stack() {
        use pumpkin_data::data_component::DataComponent;
        use pumpkin_protocol::codec::var_int::VarInt;

        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let mut equippable = ItemStack::new(1, &Item::BLACK_CARPET)
            .get_data_component::<EquippableImpl>()
            .expect("vanilla carpet has an equippable component")
            .clone();
        equippable.allowed_entities = None;
        let permitted_stack = ItemStack::new_with_component(
            2,
            &Item::BLACK_CARPET,
            vec![(
                DataComponent::Equippable,
                Some(Box::new(equippable) as Box<_>),
            )],
        );
        player.inventory().set_slot(0, permitted_stack.clone());

        let use_item = SUseItem {
            hand: VarInt(0),
            sequence: VarInt(0),
            yaw: 0.0,
            pitch: 0.0,
        };
        fixture.client().handle_use_item(player, &use_item, &server);

        let inventory = player.inventory();
        let hand = inventory.get_stack_in_hand(Hand::Right);
        let body = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        assert_eq!(hand.item_count, 1);
        assert_eq!(body.item_count, 1);
        assert_eq!(inventory.count_item(&Item::BLACK_CARPET), 1);
        assert_eq!(
            inventory.count_item(&Item::BLACK_CARPET) + u32::from(body.item_count),
            2
        );

        // With occupied equipment, the same stack branch keeps one in hand and stores the old item.
        inventory.set_slot(0, ItemStack::EMPTY.clone());
        inventory.set_slot(1, permitted_stack);
        inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&EquipmentSlot::BODY, ItemStack::new(1, &Item::BREAD));
        inventory
            .selected_slot
            .store(1, std::sync::atomic::Ordering::Relaxed);
        let swap_item = SUseItem {
            sequence: VarInt(1),
            ..use_item
        };
        fixture
            .client()
            .handle_use_item(player, &swap_item, &server);

        let hand = inventory.get_stack_in_hand(Hand::Right);
        let body = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        assert_eq!(hand.item_count, 1);
        assert_eq!(body.item_count, 1);
        assert!(inventory.contains_item(&Item::BREAD));
        assert_eq!(
            inventory.count_item(&Item::BLACK_CARPET) + u32::from(body.item_count),
            2
        );

        assert!(world.level.shutdown().await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn java_use_item_cannot_swap_bound_equipment_in_survival() {
        use pumpkin_data::data_component::DataComponent;
        use pumpkin_data::data_component_impl::EnchantmentsImpl;
        use pumpkin_data::enchantment::Enchantment;
        use pumpkin_protocol::codec::var_int::VarInt;
        use std::borrow::Cow;

        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let mut bound_equippable = ItemStack::new(1, &Item::BLACK_CARPET)
            .get_data_component::<EquippableImpl>()
            .expect("vanilla carpet has an equippable component")
            .clone();
        bound_equippable.allowed_entities = None;
        let mut bound_equipment = ItemStack::new_with_component(
            1,
            &Item::BLACK_CARPET,
            vec![(
                DataComponent::Equippable,
                Some(Box::new(bound_equippable) as Box<_>),
            )],
        );
        bound_equipment.set_data_component(EnchantmentsImpl {
            enchantment: Cow::Owned(vec![(&Enchantment::BINDING_CURSE, 1)]),
        });
        player
            .inventory()
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&EquipmentSlot::BODY, bound_equipment);

        let mut held_equippable = ItemStack::new(1, &Item::WHITE_CARPET)
            .get_data_component::<EquippableImpl>()
            .expect("vanilla carpet has an equippable component")
            .clone();
        held_equippable.allowed_entities = None;
        player.inventory().set_slot(
            0,
            ItemStack::new_with_component(
                1,
                &Item::WHITE_CARPET,
                vec![(
                    DataComponent::Equippable,
                    Some(Box::new(held_equippable) as Box<_>),
                )],
            ),
        );

        let use_item = SUseItem {
            hand: VarInt(0),
            sequence: VarInt(0),
            yaw: 0.0,
            pitch: 0.0,
        };
        fixture.client().handle_use_item(player, &use_item, &server);

        let inventory = player.inventory();
        assert_eq!(
            inventory.get_stack_in_hand(Hand::Right).item,
            &Item::WHITE_CARPET
        );
        let body = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        assert_eq!(body.item, &Item::BLACK_CARPET);
        assert_eq!(body.get_enchantment_level(&Enchantment::BINDING_CURSE), 1);

        assert!(world.level.shutdown().await.is_ok());
    }
}
