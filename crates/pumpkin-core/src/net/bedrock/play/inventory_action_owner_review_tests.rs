use super::*;
use pumpkin_data::block_properties::LeverLikeProperties;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_block_use_runs_only_for_mainhand() -> TestResult {
    for hand in [Hand::Left, Hand::Right] {
        let mut fixture = HandUseFixture::new().await;
        let world = fixture.world.world();
        let position = BlockPos::new(8, 64, 8);
        for (position, block) in [
            (BlockPos::new(8, 63, 8), &Block::STONE),
            (position, &Block::LEVER),
        ] {
            world.set_block_state(
                &position,
                block.default_state.id,
                pumpkin_world::world::BlockFlags::FORCE_STATE,
            );
        }
        let main = ItemStack::new(1, &Item::IRON_SWORD);
        let off = ItemStack::new(3, &Item::STICK);
        let player = &fixture.owner.player;
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, main.clone());
        player
            .inventory()
            .set_stack_in_hand(Hand::Left, off.clone());
        let held = player.inventory().get_stack_in_hand(hand);
        let (container_id, _, container_name) = hand_address(hand);
        let correction = CInventorySlot {
            container_id: VarUInt(container_id as u32),
            slot: VarUInt(if hand == Hand::Right {
                u32::from(player.inventory().get_selected_slot())
            } else {
                0
            }),
            full_container_name: Some(FullContainerName {
                container_name,
                dynamic_id: None,
            }),
            storage_item: None,
            item: NetworkItemStackDescriptor::from(&held),
        };
        let correction = fixture.owner.client().serialize_packet(&correction)?;
        let packet = click_block_packet(
            &fixture.owner.player,
            if hand == Hand::Left {
                HandSlot::Offhand
            } else {
                HandSlot::Mainhand
            },
            position,
            &held,
            Vec::new(),
        );
        fixture.drain_packets();
        fixture
            .owner
            .client()
            .handle_inventory_action(&fixture.owner.player, packet);
        fixture
            .verify_and_close(|fixture| {
                // ServerPlayerGameMode.useItemOn calls LeverBlock.useWithoutItem only for MAIN_HAND.
                assert_eq!(
                    LeverLikeProperties::from_state_id(world.get_block_state_id(&position)).powered,
                    hand == Hand::Right,
                );
                assert!(
                    fixture
                        .owner
                        .player
                        .inventory()
                        .held_item()
                        .are_equal(&main)
                );
                assert!(
                    fixture
                        .owner
                        .player
                        .inventory()
                        .off_hand_item()
                        .are_equal(&off)
                );
                assert!(
                    !fixture.owner.take_packets().contains(&correction),
                    "unchanged block use does not publish an inventory correction"
                );
                Ok(())
            })
            .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_fallback_preserves_distinct_mainhand_item() -> TestResult {
    for placement in [false, true] {
        let mut fixture = HandUseFixture::new().await;
        let world = fixture.world.world();
        let position = BlockPos::new(8, 64, 8);
        world.set_block_state(
            &position,
            Block::STONE.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        let main = ItemStack::new(2, &Item::DIAMOND);
        let off = if placement {
            glowstone_with_modifier(3, Hand::Left)
        } else {
            ItemStack::new(3, &Item::STICK)
        };
        let player = &fixture.owner.player;
        player.inventory().set_selected_slot(4);
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, main.clone());
        player
            .inventory()
            .set_stack_in_hand(Hand::Left, off.clone());
        player
            .living_entity
            .set_attribute_base(&Attributes::LUCK, 2.0);
        player
            .living_entity
            .send_equipment_changes(&[(EquipmentSlot::OFF_HAND, off.clone())]);
        fixture.owner.player.tick(&fixture.world.server);
        fixture.drain_packets();
        let packet = click_block_packet(
            &fixture.owner.player,
            HandSlot::Offhand,
            position,
            &off,
            Vec::new(),
        );
        fixture
            .owner
            .client()
            .handle_inventory_action(&fixture.owner.player, packet);
        fixture
            .verify_and_close(|fixture| {
                assert!(
                    fixture
                        .owner
                        .player
                        .inventory()
                        .held_item()
                        .are_equal(&main)
                );
                let mut expected = off.clone();
                if placement {
                    expected.decrement(1);
                }
                assert!(
                    fixture
                        .owner
                        .player
                        .inventory()
                        .off_hand_item()
                        .are_equal(&expected)
                );
                assert_eq!(
                    world.get_block(&BlockPos::new(8, 65, 8)),
                    if placement {
                        &Block::GLOWSTONE
                    } else {
                        &Block::AIR
                    }
                );
                if placement {
                    assert_packets(fixture, Hand::Left, &expected, true)?;
                } else {
                    let owner_packets = fixture.owner.take_packets();
                    assert_owner_packet(fixture, &owner_packets, Hand::Left, &expected)?;
                    callback::assert_observer_hand(fixture, Hand::Left, &expected)?;
                }
                Ok(())
            })
            .await?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ForgedUse {
    Block,
    Actions,
    EmptyHand,
    Air,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_last_egg_retains_authoritative_projectile_item() -> TestResult {
    use crate::entity::projectile::egg::EggEntity;

    for hand in [Hand::Left, Hand::Right] {
        for with_actions in [false, true] {
            let mut fixture = HandUseFixture::new().await;
            let world = fixture.world.world();
            let player = &fixture.owner.player;
            player.inventory().set_selected_slot(4);
            for unused in [Hand::Right, Hand::Left] {
                player
                    .inventory()
                    .set_stack_in_hand(unused, ItemStack::new(2, &Item::DIAMOND));
            }
            let mut before = ItemStack::new(1, &Item::BROWN_EGG);
            before.set_custom_name("Server egg".to_owned());
            player.inventory().set_stack_in_hand(hand, before.clone());
            let (container, slot, _) = hand_address(hand);
            let actions = if with_actions {
                vec![action(
                    Some(container),
                    slot,
                    NetworkItemDescriptor::from(&before),
                    NetworkItemDescriptor::default(),
                )]
            } else {
                Vec::new()
            };
            let mut packet = click_block_packet(
                &fixture.owner.player,
                if hand == Hand::Left {
                    HandSlot::Offhand
                } else {
                    HandSlot::Mainhand
                },
                BlockPos::new(8, 64, 8),
                &before,
                actions,
            );
            if let TransactionData::UseItem(data) = &mut packet.transaction_data {
                data.action_type = VarInt(1);
            }
            fixture.drain_packets();
            fixture
                .owner
                .client()
                .handle_inventory_action(&fixture.owner.player, packet);
            fixture
                .verify_and_close(|fixture| {
                    // EggItem.use gives ThrownEgg the original hand's item before consuming it.
                    let entities = world.entities.load();
                    let eggs: Vec<_> = entities
                        .iter()
                        .filter_map(|entity| {
                            let entity: &dyn std::any::Any = entity.as_ref();
                            entity.downcast_ref::<EggEntity>()
                        })
                        .collect();
                    assert_eq!(eggs.len(), 1);
                    for egg in eggs {
                        assert!(
                            egg.item_stack
                                .read()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .are_equal(&before),
                            "the last egg must retain the server's variant and components"
                        );
                    }
                    let inventory = fixture.owner.player.inventory();
                    assert!(inventory.get_stack_in_hand(hand).is_empty());
                    let unused = if hand == Hand::Left {
                        inventory.held_item()
                    } else {
                        inventory.off_hand_item()
                    };
                    assert!(unused.are_equal(&ItemStack::new(2, &Item::DIAMOND)));
                    Ok(())
                })
                .await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_bucket_exchange_uses_authoritative_original_item() -> TestResult {
    for hand in [Hand::Left, Hand::Right] {
        for with_actions in [false, true] {
            let mut fixture = HandUseFixture::new().await;
            let world = fixture.world.world();
            let position = BlockPos::new(8, 64, 8);
            world.set_block_state(
                &position,
                Block::CAULDRON.default_state.id,
                pumpkin_world::world::BlockFlags::FORCE_STATE,
            );
            let player = &fixture.owner.player;
            player.inventory().set_selected_slot(4);
            for unused in [Hand::Right, Hand::Left] {
                player
                    .inventory()
                    .set_stack_in_hand(unused, ItemStack::new(2, &Item::DIAMOND));
            }
            let before = ItemStack::new(1, &Item::WATER_BUCKET);
            let after = ItemStack::new(1, &Item::BUCKET);
            player.inventory().set_stack_in_hand(hand, before.clone());
            let (container, slot, _) = hand_address(hand);
            let actions = if with_actions {
                vec![action(
                    Some(container),
                    slot,
                    NetworkItemDescriptor::from(&before),
                    NetworkItemDescriptor::from(&after),
                )]
            } else {
                Vec::new()
            };
            let packet = click_block_packet(
                &fixture.owner.player,
                if hand == Hand::Left {
                    HandSlot::Offhand
                } else {
                    HandSlot::Mainhand
                },
                position,
                &before,
                actions,
            );
            fixture.drain_packets();
            fixture
                .owner
                .client()
                .handle_inventory_action(&fixture.owner.player, packet);
            fixture
                .verify_and_close(|fixture| {
                    // CauldronInteractions.emptyBucket fills water before replacing the originating hand.
                    assert_eq!(world.get_block(&position), &Block::WATER_CAULDRON);
                    assert_eq!(
                        pumpkin_data::block_properties::WaterCauldronLikeProperties::from_state_id(
                            world.get_block_state_id(&position)
                        )
                        .level,
                        3,
                    );
                    let inventory = fixture.owner.player.inventory();
                    assert!(inventory.get_stack_in_hand(hand).are_equal(&after));
                    let unused = if hand == Hand::Left {
                        inventory.held_item()
                    } else {
                        inventory.off_hand_item()
                    };
                    assert!(unused.are_equal(&ItemStack::new(2, &Item::DIAMOND)));
                    Ok(())
                })
                .await?;
        }
    }
    Ok(())
}

async fn run_forged_use(hand: Hand, scenario: ForgedUse) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::RESPAWN_ANCHOR.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let player = &fixture.owner.player;
    player.inventory().set_selected_slot(4);
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::DIAMOND));
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::new(3, &Item::STICK));
    if matches!(scenario, ForgedUse::EmptyHand) {
        player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::EMPTY.clone());
    }
    let main = player.inventory().held_item();
    let off = player.inventory().off_hand_item();
    let held = player.inventory().get_stack_in_hand(hand);
    let forged = ItemStack::new(2, &Item::GLOWSTONE);
    let (window, slot, _) = hand_address(hand);
    let actions = if matches!(scenario, ForgedUse::Actions) {
        // The actions must not replace the real hand before descriptor admission.
        vec![action(
            Some(window),
            slot,
            NetworkItemDescriptor::from(&forged),
            NetworkItemDescriptor::from(&ItemStack::new(1, &Item::GLOWSTONE)),
        )]
    } else {
        Vec::new()
    };
    let mut packet = click_block_packet(
        &fixture.owner.player,
        if hand == Hand::Left {
            HandSlot::Offhand
        } else {
            HandSlot::Mainhand
        },
        position,
        &forged,
        actions,
    );
    if matches!(scenario, ForgedUse::Air)
        && let TransactionData::UseItem(data) = &mut packet.transaction_data
    {
        data.action_type = VarInt(1);
    }
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);
    fixture
        .verify_and_close(|fixture| {
            // ServerGamePacketListenerImpl obtains the real held stack before gameplay callbacks.
            assert_eq!(
                RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&position))
                    .charges,
                0,
                "forged glowstone must not charge"
            );
            assert!(
                fixture
                    .owner
                    .player
                    .inventory()
                    .held_item()
                    .are_equal(&main)
            );
            assert!(
                fixture
                    .owner
                    .player
                    .inventory()
                    .off_hand_item()
                    .are_equal(&off)
            );
            assert!(fixture.java_observer.take_packets().is_empty());
            assert!(fixture.bedrock_observer.take_packets().is_empty());
            let packets = fixture.owner.take_packets();
            assert_owner_packet(fixture, &packets, hand, &held)
        })
        .await
}

macro_rules! forged_use_test {
    ($name:ident, $scenario:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() -> TestResult {
            for hand in [Hand::Left, Hand::Right] {
                run_forged_use(hand, ForgedUse::$scenario).await?;
            }
            Ok(())
        }
    };
}

forged_use_test!(forged_descriptor_cannot_charge_anchor, Block);
forged_use_test!(forged_descriptor_cannot_replace_hand_via_actions, Actions);
forged_use_test!(forged_descriptor_cannot_charge_with_empty_hand, EmptyHand);
forged_use_test!(forged_air_use_cannot_create_item, Air);
