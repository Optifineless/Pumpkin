use super::*;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use pumpkin_protocol::{
    bedrock::{
        network_item::NetworkItemDescriptor,
        server::inventory_transaction::{
            HandSlot, InventoryAction, SInventoryTransaction, TransactionData,
            UseItemTransactionData, WINDOW_ID_OFF_HAND,
        },
    },
    codec::var_uint::VarUInt,
};

fn use_on(f: &Fixture, actor: &TestBedrockPlayer) {
    let before = actor.player.inventory().off_hand_item();
    let mut after = before.clone();
    after.decrement(1);
    actor.client().handle_inventory_action(
        &actor.player,
        SInventoryTransaction {
            legacy_request_id: VarInt(0),
            legacy_set_item_slots: Vec::new(),
            has_value: true,
            transaction_type: VarUInt(2),
            actions: vec![InventoryAction {
                source_type: 0,
                window_id: Some(WINDOW_ID_OFF_HAND),
                source_flags: None,
                inventory_slot: 0,
                old_item: NetworkItemDescriptor::from(&before),
                new_item: NetworkItemDescriptor::from(&after),
            }],
            transaction_data: TransactionData::UseItem(UseItemTransactionData {
                action_type: VarInt(0),
                trigger_type: 0,
                block_position: RAIL,
                block_face: 1,
                hot_bar_slot: VarInt(0),
                hand: HandSlot::Offhand,
                item_in_hand: NetworkItemDescriptor::from(&before),
                player_position: Vector3::new(10.5, 65.0, 20.5),
                click_position: Vector3::new(0.5, 1.0, 0.5),
                block_runtime_id: VarUInt(0),
                client_prediction: 0,
                client_cooldown_state: 0,
            }),
        },
    );
    // Tracker packets to a real Java observer exercise the same Bedrock-originated spawn.
    assert!(
        f.world
            .players
            .load()
            .iter()
            .any(|p| Arc::ptr_eq(p, &actor.player))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_bedrock_offhand_placement_and_overlap() {
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    let mut actor = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        TestBedrockPlayer::new(&f.world),
    )
    .await
    .unwrap();
    actor
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    actor
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 65.0, 20.5));
    actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::new(2, &Item::CHEST_MINECART));
    actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(3, &Item::DIAMOND));
    f.world
        .players
        .store(Arc::new(vec![actor.player.clone(), f.actor.player.clone()]));
    f.actor.take_packets();
    use_on(&f, &actor);
    assert_eq!(f.carts().len(), 1);
    assert_geometry(&f.carts()[0], (10.5, 64.1, 20.5, 270.0, 0.0));
    assert_eq!(actor.player.inventory().off_hand_item().item_count, 1);
    assert_eq!(actor.player.inventory().held_item().item_count, 3);
    assert_eq!(f.packets().0.len(), 1);
    assert_hand_packet(
        &actor.take_packets(),
        &actor.player.inventory().off_hand_item(),
    );
    use_on(&f, &actor);
    assert_eq!(
        f.carts().len(),
        1,
        "Bedrock occupied rail must reject a second cart"
    );
    assert_eq!(actor.player.inventory().off_hand_item().item_count, 1);
    assert!(f.packets().0.is_empty());
    assert_hand_packet(
        &actor.take_packets(),
        &actor.player.inventory().off_hand_item(),
    );
    actor.close().await;
    f.close().await;
}

fn assert_hand_packet(packets: &[bytes::Bytes], expected: &ItemStack) {
    use pumpkin_protocol::{
        bedrock::network_item::{FullContainerName, NetworkItemStackDescriptor},
        serial::PacketRead,
    };
    let mut latest = None;
    for packet in packets {
        let mut read = packet.as_ref();
        assert_eq!(
            u8::read(&mut read).unwrap(),
            pumpkin_protocol::bedrock::BEDROCK_GAME_PACKET
        );
        let length = VarUInt::read(&mut read).unwrap().0 as usize;
        assert_eq!(length, read.len());
        let id = VarUInt::read(&mut read).unwrap().0 & 0x3ff;
        if id == 50 {
            let window = VarUInt::read(&mut read).unwrap().0;
            let slot = VarUInt::read(&mut read).unwrap().0;
            if window != WINDOW_ID_OFF_HAND as u32 || slot != 0 {
                continue;
            }
            let _name = Option::<FullContainerName>::read(&mut read).unwrap();
            let _storage = Option::<NetworkItemStackDescriptor>::read(&mut read).unwrap();
            latest = Some(NetworkItemStackDescriptor::read(&mut read).unwrap());
        } else if id == 49 {
            let window = VarUInt::read(&mut read).unwrap().0;
            let count = VarUInt::read(&mut read).unwrap().0;
            for slot in 0..count {
                let stack = NetworkItemStackDescriptor::read(&mut read).unwrap();
                if window == WINDOW_ID_OFF_HAND as u32 && slot == 0 {
                    latest = Some(stack);
                }
            }
        }
    }
    let actual = latest.expect("decoded Bedrock authoritative offhand correction");
    let expected = NetworkItemStackDescriptor::from(expected);
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.stack_size, expected.stack_size);
    assert_eq!(actual.net_id, expected.net_id);
}
