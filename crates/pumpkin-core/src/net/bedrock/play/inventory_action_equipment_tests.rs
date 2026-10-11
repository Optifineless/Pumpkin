use super::*;
use pumpkin_protocol::bedrock::server::inventory_transaction::NormalTransactionData;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_moves_a_shared_modifier_between_hands() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = &fixture.owner.player;
    player.inventory().set_selected_slot(4);
    let sword = ItemStack::new(1, &Item::IRON_SWORD);
    let attack = sword
        .get_data_component::<AttributeModifiersImpl>()
        .and_then(|component| {
            component
                .attribute_modifiers
                .iter()
                .find(|modifier| modifier.r#type == &Attributes::ATTACK_DAMAGE)
        })
        .ok_or("sword attack modifier")?;
    assert_eq!(attack.operation, Operation::AddValue);
    let expected_attack = 2.0 + attack.amount;
    let mut off = sword.clone();
    let mut off_modifier = attack.clone();
    off_modifier.slot = AttributeModifierSlot::OffHand;
    off_modifier.amount = 9.0;
    off.set_data_component(AttributeModifiersImpl {
        attribute_modifiers: Cow::Owned(vec![off_modifier]),
    });
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, off.clone());
    player
        .living_entity
        .set_attribute_base(&Attributes::ATTACK_DAMAGE, 2.0);
    player
        .living_entity
        .send_equipment_changes(&[(EquipmentSlot::OFF_HAND, off.clone())]);
    assert_eq!(
        player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE),
        11.0
    );
    let actions = vec![
        action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(ItemStack::EMPTY),
            NetworkItemDescriptor::from(&sword),
        ),
        action(
            Some(WINDOW_ID_OFF_HAND),
            0,
            NetworkItemDescriptor::from(&off),
            NetworkItemDescriptor::from(ItemStack::EMPTY),
        ),
    ];
    let mut packet = click_block_packet(HandSlot::Mainhand, BlockPos::new(8, 64, 8), &off, actions);
    packet.transaction_type = VarUInt(0);
    packet.transaction_data = TransactionData::Normal(NormalTransactionData);
    fixture.drain_packets();

    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);

    fixture
        .verify_and_close(|fixture| {
            let player = &fixture.owner.player;
            assert!(player.inventory().held_item().are_equal(&sword));
            assert!(player.inventory().off_hand_item().is_empty());
            assert_eq!(player.inventory().count_item(&Item::IRON_SWORD), 1);
            // LivingEntity.collectEquipmentChanges removes both old hands before installing new ones.
            assert_eq!(
                player
                    .living_entity
                    .get_attribute_value(&Attributes::ATTACK_DAMAGE),
                expected_attack
            );
            let packets = fixture.owner.take_packets();
            assert_owner_packet(fixture, &packets, Hand::Left, ItemStack::EMPTY)
        })
        .await
}
