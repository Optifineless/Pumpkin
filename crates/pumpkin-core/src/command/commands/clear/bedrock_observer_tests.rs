#![expect(
    clippy::unwrap_used,
    reason = "The real clear-command regression requires successful fixture setup"
)]

use super::tests::ClearFixture;
use crate::entity::EntityBase;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack};
use pumpkin_protocol::bedrock::{client::CMobEquipment, network_item::NetworkItemStackDescriptor};
use std::sync::Arc;

async fn tracked_observer(fixture: &ClearFixture) -> TestBedrockPlayer {
    let world = fixture.target.player.world();
    let observer = TestBedrockPlayer::new(&world).await;
    observer.player.watched_section.store(
        pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
            fixture.target.player.get_entity().chunk_pos.load(),
            std::num::NonZeroU8::new(2).unwrap(),
        ),
    );
    observer
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(fixture.target.player.get_entity().chunk_pos.load());
    world.players.store(Arc::new(vec![
        fixture.target.player.clone(),
        fixture.observer.player.clone(),
        observer.player.clone(),
    ]));
    world
        .entity_tracker
        .get_tracked_entity(fixture.target.player.entity_id())
        .unwrap()
        .update_player(&observer.player, &world);
    observer
}

#[tokio::test]
async fn clear_offhand_sends_bedrock_observer_offhand_container() {
    let fixture = ClearFixture::new();
    let mut observer = tracked_observer(&fixture).await;
    let main_hand = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.equip(&[
        (EquipmentSlot::MAIN_HAND, main_hand.clone()),
        (EquipmentSlot::OFF_HAND, ItemStack::new(1, &Item::SHIELD)),
    ]);
    let runtime_id = (fixture.target.player.entity_id() as u64).into();
    let main_packet = observer
        .client()
        .serialize_packet(&CMobEquipment {
            target_runtime_id: runtime_id,
            item: NetworkItemStackDescriptor::from(&main_hand),
            slot: 0,
            selected_slot: 0,
            container_id: 0,
        })
        .unwrap();
    assert!(
        observer.take_packets().contains(&main_packet),
        "Bedrock equipment observer was not paired through the real tracker"
    );

    // Bedrock's offhand container is 119; armor is the distinct container 120.
    let empty_offhand = CMobEquipment {
        target_runtime_id: runtime_id,
        item: NetworkItemStackDescriptor::default(),
        slot: 0,
        selected_slot: 0,
        container_id: 119,
    };
    let expected = observer.client().serialize_packet(&empty_offhand).unwrap();
    let wrong_armor = observer
        .client()
        .serialize_packet(&CMobEquipment {
            container_id: 120,
            ..empty_offhand
        })
        .unwrap();

    assert_eq!(fixture.run_clear("clear @s minecraft:shield 0").unwrap(), 1);
    assert_eq!(
        fixture.target.player.inventory.off_hand_item().item.id,
        Item::SHIELD.id
    );
    let count_only_packets = observer.take_packets();
    assert!(!count_only_packets.contains(&expected));
    assert!(!count_only_packets.contains(&wrong_armor));

    assert_eq!(fixture.run_clear("clear @s minecraft:shield").unwrap(), 1);
    assert!(fixture.target.player.inventory.off_hand_item().is_empty());
    assert!(
        fixture
            .target
            .player
            .inventory
            .held_item()
            .are_equal(&main_hand)
    );
    let packets = observer.take_packets();
    assert!(
        packets.contains(&expected),
        "Bedrock observer did not receive cleared offhand in container 119"
    );
    assert!(!packets.contains(&wrong_armor));
    observer.close().await;
}
