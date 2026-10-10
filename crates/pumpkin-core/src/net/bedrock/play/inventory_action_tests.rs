use super::*;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::{bedrock::combat_test_support::TestBedrockPlayer, java::combat_test_support::TestPlayer},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, biome::Biome, block_properties::RespawnAnchorLikeProperties, item::Item,
};
use pumpkin_protocol::{
    bedrock::{
        network_item::NetworkItemDescriptor,
        server::inventory_transaction::{
            HandSlot, InventoryAction, SInventoryTransaction, TransactionData,
            UseItemTransactionData, WINDOW_ID_INVENTORY, WINDOW_ID_OFF_HAND,
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
    java::client::play::CSetEquipment,
};
use pumpkin_util::{Hand, math::position::BlockPos, math::vector3::Vector3};

fn descriptor(id: i32, stack_size: u16) -> NetworkItemDescriptor {
    NetworkItemDescriptor {
        id: VarInt(id),
        stack_size,
        ..NetworkItemDescriptor::default()
    }
}

fn action(
    window_id: Option<i32>,
    inventory_slot: u32,
    old_item: NetworkItemDescriptor,
    new_item: NetworkItemDescriptor,
) -> InventoryAction {
    InventoryAction {
        source_type: 0,
        window_id,
        source_flags: None,
        inventory_slot,
        old_item,
        new_item,
    }
}

fn click_block_packet(
    hand: HandSlot,
    position: BlockPos,
    held_item: &ItemStack,
    actions: Vec<InventoryAction>,
) -> SInventoryTransaction {
    SInventoryTransaction {
        legacy_request_id: VarInt(0),
        legacy_set_item_slots: Vec::new(),
        has_value: true,
        actions,
        transaction_type: VarUInt(2),
        transaction_data: TransactionData::UseItem(UseItemTransactionData {
            action_type: VarInt(0),
            trigger_type: 0,
            block_position: position,
            block_face: 1,
            hot_bar_slot: VarInt(0),
            hand,
            item_in_hand: NetworkItemDescriptor::from(held_item),
            player_position: Vector3::new(8.5, 64.0, 8.5),
            click_position: Vector3::new(0.5, 1.0, 0.5),
            block_runtime_id: VarUInt(0),
            client_prediction: 0,
            client_cooldown_state: 0,
        }),
    }
}

#[test]
fn selected_hand_transaction_action_is_not_consumed_twice() {
    assert!(transaction_consumes_hand(
        &[action(
            Some(WINDOW_ID_INVENTORY),
            4,
            descriptor(5, 2),
            descriptor(5, 1)
        )],
        Hand::Right,
        4
    ));
    assert!(transaction_consumes_hand(
        &[action(
            Some(WINDOW_ID_INVENTORY),
            4,
            descriptor(5, 1),
            descriptor(0, 0)
        )],
        Hand::Right,
        4
    ));
    assert!(!transaction_consumes_hand(
        &[action(
            Some(WINDOW_ID_INVENTORY),
            3,
            descriptor(5, 2),
            descriptor(5, 1)
        )],
        Hand::Right,
        4
    ));
    assert!(transaction_consumes_hand(
        &[action(
            Some(WINDOW_ID_OFF_HAND),
            0,
            descriptor(5, 2),
            descriptor(5, 1)
        )],
        Hand::Left,
        4
    ));
    assert!(!transaction_consumes_hand(
        &[action(
            Some(WINDOW_ID_OFF_HAND),
            0,
            descriptor(5, 2),
            descriptor(5, 1)
        )],
        Hand::Right,
        4
    ));
    assert!(!transaction_consumes_hand(
        &[action(
            Some(WINDOW_ID_INVENTORY),
            4,
            descriptor(5, 1),
            descriptor(5, 1)
        )],
        Hand::Right,
        4
    ));
    assert!(!transaction_consumes_hand(
        &[action(None, 4, descriptor(5, 2), descriptor(5, 1))],
        Hand::Right,
        4
    ));
}

#[test]
fn consumed_hand_stack_is_written_back_only_when_server_authoritative() {
    let before = ItemStack::new(2, &Item::GLOWSTONE);
    let after = ItemStack::new(1, &Item::GLOWSTONE);

    assert!(should_write_back_consumed_stack(
        &before, &after, true, false
    ));
    assert!(!should_write_back_consumed_stack(
        &before, &after, true, true
    ));
    assert!(!should_write_back_consumed_stack(
        &before, &after, false, false
    ));
    assert!(!should_write_back_consumed_stack(
        &before, &before, true, false
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_anchor_charge_consumes_only_the_offhand_transaction_stack() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let bedrock = TestBedrockPlayer::new(&world).await;
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::RESPAWN_ANCHOR.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let glowstone = ItemStack::new(2, &Item::GLOWSTONE);
    bedrock
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, glowstone.clone());
    bedrock
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, glowstone.clone());
    let after_transaction = ItemStack::new(1, &Item::GLOWSTONE);
    let packet = click_block_packet(
        HandSlot::Offhand,
        position,
        &glowstone,
        vec![action(
            Some(WINDOW_ID_OFF_HAND),
            0,
            NetworkItemDescriptor::from(&glowstone),
            NetworkItemDescriptor::from(&after_transaction),
        )],
    );

    bedrock
        .client()
        .handle_inventory_action(&bedrock.player, packet);

    let anchor = RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&position));
    assert_eq!(anchor.charges, 1);
    assert_eq!(bedrock.player.inventory().held_item().item_count, 2);
    assert_eq!(bedrock.player.inventory().off_hand_item().item_count, 1);

    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consuming_last_block_use_item_sends_equipment_to_tracker() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let bedrock = TestBedrockPlayer::new(&world).await;
    let mut tracker = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        bedrock.player.clone(),
        tracker.player.clone(),
    ]));
    if let Some(tracked) = world
        .entity_tracker
        .get_tracked_entity(bedrock.player.get_entity().entity_id)
    {
        tracked.seen_by.insert(tracker.player.gameprofile.id);
    }

    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::RESPAWN_ANCHOR.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let glowstone = ItemStack::new(1, &Item::GLOWSTONE);
    bedrock
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, glowstone.clone());
    tracker.take_packets();
    let packet = click_block_packet(HandSlot::Mainhand, position, &glowstone, Vec::new());

    bedrock
        .client()
        .handle_inventory_action(&bedrock.player, packet);

    assert!(bedrock.player.inventory().held_item().is_empty());
    let expected_equipment = CSetEquipment::new(
        bedrock.player.get_entity().entity_id.into(),
        vec![(
            EquipmentSlot::MAIN_HAND.discriminant(),
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(
                ItemStack::EMPTY.clone(),
            ),
        )],
    );
    let expected_packet = tracker.client().serialize_packet(&expected_equipment);
    assert!(expected_packet.is_ok());
    let packets = tracker.take_packets();
    if let Ok(expected_packet) = expected_packet {
        assert!(
            packets.iter().any(|packet| packet == &expected_packet),
            "tracking player did not receive the empty main-hand equipment update"
        );
    }

    bedrock.close().await;
    fixture.server.shutdown().await;
}
