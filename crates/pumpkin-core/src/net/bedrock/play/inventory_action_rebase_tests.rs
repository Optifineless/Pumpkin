use super::*;
use crate::entity::projectile::egg::EggEntity;
use pumpkin_protocol::bedrock::server::inventory_transaction::NormalTransactionData;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn noop_action_preserves_uid_and_following_pinned_use() -> TestResult {
    for (hand, named) in [
        (Hand::Left, false),
        (Hand::Right, false),
        (Hand::Left, true),
        (Hand::Right, true),
    ] {
        let fixture = HandUseFixture::new().await;
        let player = &fixture.owner.player;
        player.inventory().set_selected_slot(4);
        let mut before = ItemStack::new(2, &Item::GLOWSTONE);
        if named {
            before.set_custom_name("Server glowstone".to_owned());
        }
        player.inventory().set_stack_in_hand(hand, before.clone());
        let (container, slot, _) = hand_address(hand);
        let noop = || {
            action(
                Some(container),
                slot,
                NetworkItemDescriptor::from(&before),
                NetworkItemDescriptor::from(&before),
            )
        };
        let position = BlockPos::new(8, 64, 8);
        let packet_hand = if hand == Hand::Left {
            HandSlot::Offhand
        } else {
            HandSlot::Mainhand
        };
        let mut packet = click_block_packet(
            &fixture.owner.player,
            packet_hand,
            position,
            &before,
            vec![noop()],
        );
        packet.transaction_type = VarUInt(0);
        packet.transaction_data = TransactionData::Normal(NormalTransactionData);
        fixture
            .owner
            .client()
            .handle_inventory_action(player, packet);
        let unchanged = player.inventory().get_stack_in_hand(hand);
        let world = fixture.world.world();
        world.set_block_state(
            &position,
            Block::RESPAWN_ANCHOR.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        // Exercise the same no-op before the actual pinned writeback, not just a getter.
        fixture.owner.client().handle_inventory_action(
            player,
            click_block_packet(
                &fixture.owner.player,
                packet_hand,
                position,
                &before,
                vec![noop()],
            ),
        );
        fixture
            .verify_and_close(|fixture| {
                assert_eq!(
                    unchanged.uid, before.uid,
                    "no-op must preserve stack identity"
                );
                assert!(unchanged.are_equal(&before));
                let after = fixture.owner.player.inventory().get_stack_in_hand(hand);
                assert_eq!(after.item_count, 1, "one glowstone pays for one charge");
                assert_eq!(after.uid, before.uid);
                assert!(after.are_items_and_components_equal(&before));
                assert_eq!(
                    RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&position))
                        .charges,
                    1
                );
                Ok(())
            })
            .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_two_egg_air_use_consumes_once() -> TestResult {
    for hand in [Hand::Left, Hand::Right] {
        let fixture = HandUseFixture::new().await;
        let player = &fixture.owner.player;
        player.inventory().set_selected_slot(4);
        let mut before = ItemStack::new(2, &Item::BROWN_EGG);
        before.set_custom_name("Server egg".to_owned());
        player.inventory().set_stack_in_hand(hand, before.clone());
        let (container, slot, _) = hand_address(hand);
        let mut packet = click_block_packet(
            &fixture.owner.player,
            if hand == Hand::Left {
                HandSlot::Offhand
            } else {
                HandSlot::Mainhand
            },
            BlockPos::new(8, 64, 8),
            &before,
            vec![action(
                Some(container),
                slot,
                NetworkItemDescriptor::from(&before),
                NetworkItemDescriptor::from(&before.copy_with_count(1)),
            )],
        );
        if let TransactionData::UseItem(data) = &mut packet.transaction_data {
            data.action_type = VarInt(1);
        }
        fixture
            .owner
            .client()
            .handle_inventory_action(player, packet);
        let world = fixture.world.world();
        fixture
            .verify_and_close(|fixture| {
                let after = fixture.owner.player.inventory().get_stack_in_hand(hand);
                assert_eq!(
                    after.item_count, 1,
                    "one throw must consume exactly one egg"
                );
                assert_eq!(after.uid, before.uid);
                assert!(after.are_items_and_components_equal(&before));
                let entities = world.entities.load();
                let eggs: Vec<_> = entities
                    .iter()
                    .filter_map(|entity| {
                        let entity: &dyn std::any::Any = entity.as_ref();
                        entity.downcast_ref::<EggEntity>()
                    })
                    .collect();
                assert_eq!(eggs.len(), 1, "one click must spawn exactly one projectile");
                assert!(
                    eggs[0]
                        .item_stack
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .are_items_and_components_equal(&before)
                );
                Ok(())
            })
            .await?;
    }
    Ok(())
}

// Retain the PR's existing compare-and-replace unit coverage without constructing a raw lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_hand_snapshot_cannot_replace_new_identity_or_changed_count() -> TestResult {
    use pumpkin_inventory::Inventory;

    let fixture = HandUseFixture::new().await;
    fixture
        .verify_and_close(|fixture| {
            let inventory = fixture.owner.player.inventory();
            for slot in [4, PlayerInventory::OFF_HAND_SLOT] {
                let before = ItemStack::new(2, &Item::STONE);
                let after = before.copy_with_count(1);
                let replacement = ItemStack::new(2, &Item::STONE);
                inventory.set_stack(slot, replacement.clone());
                assert!(!inventory.replace_stack_if_unchanged(slot, &before, after.clone()));
                assert_eq!(inventory.get_stack(slot).uid, replacement.uid);

                let mut changed = before.clone();
                changed.set_count(3);
                inventory.set_stack(slot, changed);
                assert!(!inventory.replace_stack_if_unchanged(slot, &before, after.clone()));
                assert_eq!(inventory.get_stack(slot).item_count, 3);

                inventory.set_stack(slot, before.clone());
                assert!(inventory.replace_stack_if_unchanged(slot, &before, after));
                assert_eq!(inventory.get_stack(slot).item_count, 1);
            }
            Ok(())
        })
        .await
}
