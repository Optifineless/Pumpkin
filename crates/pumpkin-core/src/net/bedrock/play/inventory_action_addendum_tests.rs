use super::*;
use crate::{
    plugin::{BoxFuture, EventHandler, EventPriority, player::item_held::PlayerItemHeldEvent},
    server::Server,
};
use pumpkin_protocol::bedrock::client::{player_hotbar::CPlayerHotbar, update_block::CUpdateBlock};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

pub(super) fn assert_block_corrections(
    fixture: &HandUseFixture,
    packets: &[bytes::Bytes],
    clicked: BlockPos,
) -> TestResult {
    let world = fixture.world.world();
    for position in [
        clicked,
        BlockPos(clicked.0 + BlockDirection::Up.to_offset()),
    ] {
        let packet = CUpdateBlock::new(
            position,
            pumpkin_data::BlockState::to_be_network_id(world.get_block_state_id(&position)),
        );
        let encoded = fixture.owner.client().serialize_packet(&packet)?;
        assert!(
            packets.contains(&encoded),
            "authoritative block correction at {position:?}"
        );
    }
    Ok(())
}

struct HeldSelection {
    calls: Arc<AtomicUsize>,
    cancel: bool,
}

impl EventHandler<PlayerItemHeldEvent> for HeldSelection {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerItemHeldEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!((event.previous_slot, event.new_slot), (4, 8));
        self.calls.fetch_add(1, SeqCst);
        // Cancel even after selecting the requested slot: cancellation must still win.
        if self.cancel {
            event.player.inventory().set_selected_slot(8);
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

fn assert_selection_packets(
    fixture: &mut HandUseFixture,
    selected: u8,
    accepted: bool,
) -> TestResult {
    let owner = fixture.owner.take_packets();
    let hotbar = CPlayerHotbar {
        selected_slot: VarUInt(u32::from(selected)),
        container_id: 0,
        should_select_slot: true,
    };
    assert!(
        owner.contains(&fixture.owner.client().serialize_packet(&hotbar)?),
        "owner selection correction"
    );
    if accepted {
        assert_owner_inventory_slot(
            &owner,
            WINDOW_ID_INVENTORY,
            8,
            &fixture.owner.player.inventory().get_stack(8),
            false,
        )?;
    } else {
        let content = CInventoryContent {
            container_id: VarUInt(0),
            slots: (0..PlayerInventory::MAIN_SIZE)
                .map(|slot| {
                    NetworkItemStackDescriptor::from(
                        &fixture.owner.player.inventory().get_stack(slot),
                    )
                })
                .collect(),
            full_container_name: FullContainerName {
                container_name: ContainerName::Inventory,
                dynamic_id: None,
            },
            storage_item: NetworkItemStackDescriptor::default(),
        };
        assert!(
            owner.contains(&fixture.owner.client().serialize_packet(&content)?),
            "rejected prediction inventory correction"
        );
    }
    let held = fixture.owner.player.inventory().held_item();
    let java = CSetEquipment::new(
        fixture.owner.player.entity_id().into(),
        vec![(
            EquipmentSlot::MAIN_HAND.discriminant(),
            ItemStackSerializer::from(held.clone()),
        )],
    );
    let encoded = fixture.java_observer.client().serialize_packet(&java)?;
    assert_eq!(
        fixture
            .java_observer
            .take_packets()
            .iter()
            .filter(|p| **p == encoded)
            .count(),
        usize::from(fixture.hand_changed(Hand::Right))
    );
    let bedrock = CMobEquipment {
        target_runtime_id: (fixture.owner.player.entity_id() as u64).into(),
        item: NetworkItemStackDescriptor::from(&held),
        slot: selected,
        selected_slot: selected,
        container_id: 0,
    };
    let encoded = fixture
        .bedrock_observer
        .client()
        .serialize_packet(&bedrock)?;
    assert_eq!(
        fixture
            .bedrock_observer
            .take_packets()
            .iter()
            .filter(|p| **p == encoded)
            .count(),
        usize::from(fixture.hand_changed(Hand::Right))
    );
    Ok(())
}

async fn transaction_selection(slot: i32, cancel: bool, identical_items: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    let world = fixture.world.world();
    let clicked = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &clicked,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    player.inventory().set_selected_slot(4);
    let original = ItemStack::new(
        7,
        if identical_items {
            &Item::STONE
        } else {
            &Item::DIAMOND
        },
    );
    let stone = ItemStack::new(64, &Item::STONE);
    player.inventory().set_stack(4, original.clone());
    player.inventory().set_stack(8, stone.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<PlayerItemHeldEvent, _>(
            Arc::new(HeldSelection {
                calls: calls.clone(),
                cancel,
            }),
            EventPriority::Normal,
            true,
        );
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        clicked,
        &stone,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            8,
            NetworkItemDescriptor::from(&stone),
            NetworkItemDescriptor::from(&stone.copy_with_count(63)),
        )],
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.hot_bar_slot = VarInt(slot);
    }
    fixture.drain_packets();
    // No SMobEquipment packet has arrived yet.
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            let valid = (0..=8).contains(&slot);
            let accepted = valid && !cancel;
            let selected = if valid { 8 } else { 4 };
            assert_eq!(
                world.get_block(&BlockPos::new(8, 65, 8)),
                if accepted { &Block::STONE } else { &Block::AIR }
            );
            let source = player.inventory().get_stack(4);
            assert!(source.are_equal(&original), "previous hotbar stack changed");
            assert_eq!(source.uid, original.uid);
            assert_eq!(player.inventory().get_selected_slot(), selected);
            assert_eq!(calls.load(SeqCst), usize::from(valid), "held-item event");
            let mut expected = stone.clone();
            if accepted {
                expected.decrement(1);
            }
            let actual = player.inventory().get_stack(8);
            assert!(
                actual.are_equal(&expected),
                "transaction hotbar stack must be charged exactly once"
            );
            assert_eq!(actual.uid, stone.uid);
            assert_selection_packets(fixture, selected, accepted)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn use_transaction_selects_hotbar_before_admission() -> TestResult {
    transaction_selection(8, false, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn use_transaction_rejects_invalid_hotbar_with_correction() -> TestResult {
    for slot in [-1, 9, i32::MAX] {
        transaction_selection(slot, false, false).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn use_transaction_respects_cancelled_hotbar_change() -> TestResult {
    transaction_selection(8, true, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn use_transaction_charges_requested_slot_with_identical_item_ids() -> TestResult {
    transaction_selection(8, false, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_placement_correction_wraps_extreme_coordinates_like_java() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.gamemode.store(GameMode::Adventure);
    player.inventory().set_selected_slot(4);
    let stone = ItemStack::new(1, &Item::STONE);
    player.inventory().set_stack(4, stone.clone());
    let clicked = BlockPos::new(i32::MAX, 64, 8);
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        clicked,
        &stone,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(&stone),
            NetworkItemDescriptor::default(),
        )],
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.block_face = 5; // East; Java BlockPos.relative wraps MAX to MIN.
    }
    fixture.drain_packets();
    fixture
        .verify_and_close(|fixture| {
            fixture
                .owner
                .client()
                .handle_inventory_action(&player, packet);
            let actual = player.inventory().held_item();
            assert!(actual.are_equal(&stone));
            assert_eq!(actual.uid, stone.uid);
            let owner = fixture.owner.take_packets();
            for position in [clicked, BlockPos::new(i32::MIN, 64, 8)] {
                let correction = CUpdateBlock::new(
                    position,
                    pumpkin_data::BlockState::to_be_network_id(Block::AIR.default_state.id),
                );
                assert!(
                    owner.contains(&fixture.owner.client().serialize_packet(&correction)?),
                    "Adventure rejection must correct {position:?}"
                );
            }
            Ok(())
        })
        .await
}
