use super::*;
use crate::entity::passive::horse::HorseEntity;
use pumpkin_data::entity::EntityType;
use pumpkin_protocol::bedrock::server::inventory_transaction::UseItemOnEntityTransactionData;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_horses_s2_saddle_payment_refused_bedrock_prediction() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let mut saddle = ItemStack::new(1, &Item::SADDLE);
    saddle.set_custom_name("Keep this saddle".to_owned());
    player.inventory().set_stack(4, saddle.clone());
    let entity = fixture.world.mob(&EntityType::HORSE);
    let horse = entity
        .cast_any()
        .downcast_ref::<HorseEntity>()
        .ok_or("horse")?;
    horse.set_tame(true);
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &saddle,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(&saddle),
            NetworkItemDescriptor::from(ItemStack::EMPTY),
        )],
    );
    packet.transaction_type = VarUInt(3);
    packet.transaction_data = TransactionData::UseItemOnEntity(UseItemOnEntityTransactionData {
        target_entity_runtime_id: (entity.get_entity().entity_id as u64).into(),
        action_type: VarInt(0),
        hot_bar_slot: VarInt(4),
        item_in_hand: NetworkItemDescriptor::from(&saddle),
        player_position: Vector3::new(8.5, 64.0, 8.5),
        click_position: Vector3::new(0.0, 0.0, 0.0),
    });
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            let actual = player.inventory().held_item();
            assert!(
                actual.are_equal(&saddle),
                "horse saddle use lost inventory stock"
            );
            assert_eq!(actual.uid, saddle.uid);
            assert!(!horse.is_saddled(), "refused saddle changed horse flags");
            let packets = fixture.owner.take_packets();
            assert_owner_inventory_slot(&packets, WINDOW_ID_INVENTORY, 4, &saddle, false)
        })
        .await
}
