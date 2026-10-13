use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_release_prediction_does_not_remove_active_bow() -> TestResult {
    for hand in [Hand::Right, Hand::Left] {
        let mut fixture = HandUseFixture::new().await;
        let player = fixture.owner.player.clone();
        player.inventory().set_selected_slot(4);
        let bow = ItemStack::new(1, &Item::BOW);
        player.inventory().set_stack_in_hand(hand, bow.clone());
        player.living_entity.set_active_hand(
            hand,
            bow.clone(),
            crate::item::items::bow::BowItem::USE_DURATION,
        );
        let (container, slot, _) = hand_address(hand);
        let mut packet = click_block_packet(
            &player,
            HandSlot::Mainhand,
            BlockPos::new(8, 64, 8),
            &bow,
            vec![action(
                Some(container),
                slot,
                NetworkItemDescriptor::from(&bow),
                NetworkItemDescriptor::default(),
            )],
        );
        packet.transaction_type = VarUInt(4);
        packet.transaction_data = TransactionData::ReleaseItem(ReleaseItemTransactionData {
            action_type: VarInt(0),
            hot_bar_slot: VarInt(4),
            item_in_hand: NetworkItemDescriptor::from(&bow),
            head_position: Vector3::new(8.5, 64.0, 8.5),
        });
        fixture.drain_packets();
        fixture
            .owner
            .client()
            .handle_inventory_action(&player, packet);
        fixture
            .verify_and_close(|fixture| {
                assert!(
                    player.inventory().get_stack_in_hand(hand).are_equal(&bow),
                    "release prediction is advisory"
                );
                assert!(player.living_entity.active_hand.lock().unwrap().is_none());
                let owner = fixture.owner.take_packets();
                assert_owner_packet(fixture, &owner, hand, &bow)?;
                assert_observer_hand(fixture, hand, &bow)
            })
            .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_inactive_offhand_release_prediction_is_advisory() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let main = ItemStack::new(2, &Item::DIAMOND);
    let bow = ItemStack::new(1, &Item::BOW);
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, main.clone());
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, bow.clone());
    assert!(player.living_entity.active_hand.lock().unwrap().is_none());
    let mut packet = click_block_packet(
        &player,
        HandSlot::Offhand,
        BlockPos::new(8, 64, 8),
        &bow,
        vec![action(
            Some(WINDOW_ID_OFF_HAND),
            0,
            NetworkItemDescriptor::from(&bow),
            NetworkItemDescriptor::default(),
        )],
    );
    packet.transaction_type = VarUInt(4);
    packet.transaction_data = TransactionData::ReleaseItem(ReleaseItemTransactionData {
        action_type: VarInt(0),
        hot_bar_slot: VarInt(4),
        item_in_hand: NetworkItemDescriptor::from(&bow),
        head_position: Vector3::new(8.5, 64.0, 8.5),
    });
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            let off = player.inventory().off_hand_item();
            assert_eq!(
                off.item_count, 1,
                "inactive release cannot debit the offhand"
            );
            assert!(off.are_equal(&bow));
            assert_eq!(off.uid, bow.uid);
            assert!(player.inventory().held_item().are_equal(&main));
            assert!(player.living_entity.active_hand.lock().unwrap().is_none());
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Left, &bow)?;
            assert_observer_hand(fixture, Hand::Left, &bow)
        })
        .await
}

struct StartUsingDuringEquipment {
    player: Arc<Player>,
    fired: AtomicBool,
}
impl EventHandler<PacketSentEvent> for StartUsingDuringEquipment {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.player.living_entity.clear_active_hand();
            let bow = self.player.inventory().held_item();
            server.item_registry.on_use(&bow, &self.player, Hand::Right);
        }
        Box::pin(async {})
    }
}

fn start_using_callback(fixture: &HandUseFixture) -> Arc<StartUsingDuringEquipment> {
    let java = fixture.java_observer.client();
    java.set_player(fixture.java_observer.player.clone());
    java.version.store(JavaMinecraftVersion::V_26_2);
    let control = Arc::new(StartUsingDuringEquipment {
        player: fixture.owner.player.clone(),
        fired: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(control.clone(), EventPriority::Normal, true);
    control
}

fn release_packet(
    player: &Player,
    stack: &ItemStack,
    actions: Vec<InventoryAction>,
) -> SInventoryTransaction {
    let mut packet = click_block_packet(
        player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        stack,
        actions,
    );
    packet.transaction_type = VarUInt(4);
    packet.transaction_data = TransactionData::ReleaseItem(ReleaseItemTransactionData {
        action_type: VarInt(0),
        hot_bar_slot: VarInt(i32::from(player.inventory().get_selected_slot())),
        item_in_hand: NetworkItemDescriptor::from(stack),
        head_position: Vector3::new(8.5, 64.0, 8.5),
    });
    packet
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_inactive_release_preserves_callback_started_use() -> TestResult {
    use pumpkin_protocol::bedrock::server::inventory_transaction::WINDOW_ID_ARMOUR;
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let bow = ItemStack::new(1, &Item::BOW);
    let off = ItemStack::new(2, &Item::DIAMOND);
    let helmet = ItemStack::new(1, &Item::IRON_HELMET);
    player.inventory().set_stack(4, bow.clone());
    player
        .inventory()
        .set_stack(9, ItemStack::new(2, &Item::ARROW));
    player.inventory().set_stack(0, helmet.clone());
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, off.clone());
    let control = start_using_callback(&fixture);
    let packet = release_packet(
        &player,
        &off,
        vec![
            action(
                Some(WINDOW_ID_OFF_HAND),
                0,
                NetworkItemDescriptor::from(&off),
                NetworkItemDescriptor::default(),
            ),
            action(
                Some(WINDOW_ID_INVENTORY),
                0,
                NetworkItemDescriptor::from(&helmet),
                NetworkItemDescriptor::default(),
            ),
            action(
                Some(WINDOW_ID_ARMOUR),
                0,
                NetworkItemDescriptor::default(),
                NetworkItemDescriptor::from(&helmet),
            ),
        ],
    );
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|_| {
            assert!(
                control.fired.load(SeqCst),
                "real pre-release equipment callback"
            );
            assert_eq!(
                *player.living_entity.active_hand.lock().unwrap(),
                Some(Hand::Right),
                "a release admitted without an active hand must preserve the callback's new use"
            );
            assert_eq!(
                player
                    .living_entity
                    .item_use_time
                    .load(std::sync::atomic::Ordering::Relaxed),
                crate::item::items::bow::BowItem::USE_DURATION
            );
            assert!(player.inventory().off_hand_item().are_equal(&off));
            assert!(player.inventory().held_item().are_equal(&bow));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_release_callback_preserves_new_draw_after_one_shot() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let bow = ItemStack::new(1, &Item::BOW);
    player.inventory().set_stack(4, bow.clone());
    player
        .inventory()
        .set_stack(9, ItemStack::new(2, &Item::ARROW));
    let mut press = click_block_packet(
        &player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &bow,
        Vec::new(),
    );
    if let TransactionData::UseItem(data) = &mut press.transaction_data {
        data.action_type = VarInt(1);
    }
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, press);
    player.living_entity.item_use_time.store(
        crate::item::items::bow::BowItem::USE_DURATION / 2,
        std::sync::atomic::Ordering::Relaxed,
    );
    let control = start_using_callback(&fixture);
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, release_packet(&player, &bow, Vec::new()));
    fixture
        .verify_and_close(|fixture| {
            assert!(
                control.fired.load(SeqCst),
                "real release equipment callback"
            );
            assert_eq!(
                *player.living_entity.active_hand.lock().unwrap(),
                Some(Hand::Right),
                "old release cleanup must preserve the callback's new draw"
            );
            assert_eq!(
                player
                    .living_entity
                    .item_use_time
                    .load(std::sync::atomic::Ordering::Relaxed),
                crate::item::items::bow::BowItem::USE_DURATION
            );
            assert_eq!(
                player.inventory().get_stack(9).item_count,
                1,
                "one real shot consumes one arrow"
            );
            let held = player.inventory().held_item();
            assert_eq!(held.get_damage(), 1, "one real shot damages the bow once");
            assert!(
                player
                    .living_entity
                    .item_in_use
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|active| active.are_equal(&held))
            );
            let arrows = fixture
                .world
                .world()
                .entities
                .load()
                .iter()
                .filter(|entity| {
                    entity
                        .cast_any()
                        .is::<crate::entity::projectile::arrow::ArrowEntity>()
                })
                .count();
            assert_eq!(arrows, 1, "exactly one real arrow projectile");
            Ok(())
        })
        .await
}
