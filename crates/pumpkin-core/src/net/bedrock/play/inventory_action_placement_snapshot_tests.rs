use super::*;
use crate::{
    plugin::{BoxFuture, EventHandler, EventPriority, block::block_place::BlockPlaceEvent},
    server::Server,
    world::World,
};
use pumpkin_data::data_component_impl::{ContainerImpl, LockImpl};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observer_selected_mainhand_slot_after_real_action() -> TestResult {
    run_hand_use(BlockUse::Anchor, Hand::Right, true, 1).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observer_selected_offhand_slot_after_real_action() -> TestResult {
    run_hand_use(BlockUse::Anchor, Hand::Left, true, 1).await
}

fn prepare_container_use(
    fixture: &HandUseFixture,
    item: &'static Item,
    count: u8,
) -> (SInventoryTransaction, ItemStack, ItemStack, LockImpl) {
    let position = BlockPos::new(8, 64, 8);
    fixture.world.world().set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let mut main = ItemStack::new(1, &Item::SHULKER_BOX);
    main.set_data_component(ContainerImpl {
        items: vec![(0, ItemStack::new(7, &Item::IRON_INGOT))],
    });
    let mut off = ItemStack::new(count, item);
    off.set_data_component(ContainerImpl {
        items: vec![(0, ItemStack::new(13, &Item::DIAMOND))],
    });
    let mut components = NbtCompound::new();
    components.put_string("minecraft:custom_name", "Placement key".to_owned());
    let mut predicate = NbtCompound::new();
    predicate.put_compound("components", components);
    let lock = LockImpl { predicate };
    off.set_data_component(lock.clone());
    let player = &fixture.owner.player;
    player.inventory().set_selected_slot(4);
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, main.clone());
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, off.clone());
    let after = if count == 1 {
        ItemStack::EMPTY.clone()
    } else {
        let mut after = off.clone();
        after.decrement(1);
        after
    };
    let mut packet = click_block_packet(
        &fixture.owner.player,
        HandSlot::Offhand,
        position,
        &off,
        vec![action(
            Some(WINDOW_ID_OFF_HAND),
            0,
            NetworkItemDescriptor::from(&off),
            NetworkItemDescriptor::from(&after),
        )],
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.hot_bar_slot = VarInt(4);
    }
    (packet, main, after, lock)
}

fn assert_placed_diamonds(world: &World) -> TestResult {
    let placed = BlockPos::new(8, 65, 8);
    assert_eq!(world.get_block(&placed), &Block::SHULKER_BOX);
    let inventory = world
        .get_block_entity(&placed)
        .and_then(crate::block::entities::BlockEntity::get_inventory)
        .ok_or("placed shulker inventory")?;
    // BlockItem.updateBlockEntityComponents consumes the retained UseOnContext stack.
    let contents = inventory.get_stack(0);
    assert_eq!(
        (contents.item.id, contents.item_count),
        (Item::DIAMOND.id, 13),
        "predicted placement discarded the originating container contents",
    );
    Ok(())
}

async fn predicted_container_placement(
    item: &'static Item,
    block: &'static Block,
    count: u8,
) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let (packet, main, after, lock) = prepare_container_use(&fixture, item, count);
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);
    fixture
        .verify_and_close(|fixture| {
            let player = &fixture.owner.player;
            let actual_main = player.inventory().held_item();
            assert!(actual_main.are_equal(&main));
            assert_eq!(actual_main.uid, main.uid, "unused main-hand identity");
            let actual_off = player.inventory().off_hand_item();
            assert!(
                actual_off.are_equal(&after),
                "server use consumes once and retains the original components"
            );
            let placed = BlockPos::new(8, 65, 8);
            assert_eq!(world.get_block(&placed), block);
            if count == 1 {
                assert_placed_diamonds(&world)?;
                let java_packets = fixture.java_observer.take_packets();
                equipment::assert_empty_offhand_packets(fixture, &java_packets)
            } else {
                let entity = world
                    .get_block_entity(&placed)
                    .ok_or("placed container entity")?;
                assert_eq!(
                    entity.container_lock(),
                    Some(lock),
                    "predicted placement discarded the originating container lock",
                );
                let owner_packets = fixture.owner.take_packets();
                assert_owner_packet(fixture, &owner_packets, Hand::Left, &actual_off)
            }
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_last_offhand_shulker_keeps_original_contents() -> TestResult {
    predicted_container_placement(&Item::SHULKER_BOX, &Block::SHULKER_BOX, 1).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_nonempty_offhand_chest_keeps_original_lock() -> TestResult {
    // A stackable container retains its components in the authoritative remainder.
    predicted_container_placement(&Item::CHEST, &Block::CHEST, 2).await
}

struct PlacementEdit {
    replacement: ItemStack,
    calls: Arc<AtomicUsize>,
    cancelled: bool,
}

impl EventHandler<BlockPlaceEvent> for PlacementEdit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, SeqCst);
        // These supported setters also prove that the placement hook holds no hand lock.
        event.player.inventory().set_selected_slot(8);
        event
            .player
            .inventory()
            .set_stack_in_hand(Hand::Left, self.replacement.clone());
        event.cancelled = self.cancelled;
        Box::pin(async {})
    }
}

async fn callback_container_placement(cancelled: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let (packet, main, _, _) = prepare_container_use(&fixture, &Item::SHULKER_BOX, 1);
    let hotbar = ItemStack::new(2, &Item::EMERALD);
    fixture
        .owner
        .player
        .inventory()
        .set_stack(8, hotbar.clone());
    let mut replacement = ItemStack::new(1, &Item::SHULKER_BOX);
    replacement.set_data_component(ContainerImpl {
        items: vec![(0, ItemStack::new(17, &Item::GOLD_INGOT))],
    });
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(
            Arc::new(PlacementEdit {
                replacement: replacement.clone(),
                calls: calls.clone(),
                cancelled,
            }),
            EventPriority::Normal,
            true,
        );
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);
    fixture
        .verify_and_close(|fixture| {
            let player = &fixture.owner.player;
            assert_eq!(calls.load(SeqCst), 1, "real placement callback");
            assert_eq!(player.inventory().get_selected_slot(), 8);
            let source_main = player.inventory().get_stack(4);
            assert!(source_main.are_equal(&main));
            assert_eq!(source_main.uid, main.uid);
            let selected = player.inventory().get_stack(8);
            assert!(selected.are_equal(&hotbar));
            assert_eq!(selected.uid, hotbar.uid, "newly selected slot overwritten");
            let actual = player.inventory().off_hand_item();
            assert!(
                actual.are_equal(&replacement),
                "callback replacement overwritten"
            );
            assert_eq!(actual.uid, replacement.uid, "callback replacement identity");
            if cancelled {
                let placed = BlockPos::new(8, 65, 8);
                assert_eq!(world.get_block(&placed), &Block::AIR);
                assert!(world.get_block_entity(&placed).is_none());
                Ok(())
            } else {
                assert_placed_diamonds(&world)
            }
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_shulker_callback_replacement_keeps_original_placement_contents() -> TestResult {
    callback_container_placement(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_shulker_cancel_preserves_callback_replacement_without_placement() -> TestResult {
    callback_container_placement(true).await
}

// Force callback re-entry through the real placement handler, without prediction actions.
async fn no_action_callback_placement(hand: Hand, identical_replacement: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let (mut packet, _, _, _) = prepare_container_use(&fixture, &Item::SHULKER_BOX, 1);
    packet.actions.clear();
    let before = fixture.owner.player.inventory().off_hand_item();
    if hand == Hand::Right {
        fixture
            .owner
            .player
            .inventory()
            .set_stack(4, before.clone());
        if let TransactionData::UseItem(data) = &mut packet.transaction_data {
            data.hand = HandSlot::Mainhand;
        }
    }
    let hotbar = ItemStack::new(2, &Item::DIAMOND);
    fixture
        .owner
        .player
        .inventory()
        .set_stack(8, hotbar.clone());
    let replacement = if identical_replacement {
        // Equal components/count, but a different identity must still win.
        let mut replacement = before.clone();
        replacement.uid = ItemStack::new(1, &Item::SHULKER_BOX).uid;
        replacement
    } else {
        ItemStack::new(3, &Item::EMERALD)
    };
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(
            Arc::new(PlacementEdit {
                replacement: replacement.clone(),
                calls: calls.clone(),
                cancelled: false,
            }),
            EventPriority::Normal,
            true,
        );
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);
    fixture
        .verify_and_close(|fixture| {
            let player = &fixture.owner.player;
            assert_eq!(calls.load(SeqCst), 1);
            assert_eq!(player.inventory().get_selected_slot(), 8);
            assert!(
                player.inventory().get_stack(8).are_equal(&hotbar),
                "callback selected slot overwritten"
            );
            assert_eq!(player.inventory().get_stack(8).uid, hotbar.uid);
            if hand == Hand::Right {
                assert!(
                    player.inventory().get_stack(4).is_empty(),
                    "originating slot was not charged"
                );
            } else {
                let actual = player.inventory().off_hand_item();
                assert!(
                    actual.are_equal(&replacement),
                    "callback replacement overwritten"
                );
                assert_eq!(actual.uid, replacement.uid);
            }
            assert_placed_diamonds(&world)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_action_placement_pins_originating_hotbar_slot() -> TestResult {
    no_action_callback_placement(Hand::Right, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_action_placement_respects_offhand_callback_replacement() -> TestResult {
    no_action_callback_placement(Hand::Left, false).await?;
    no_action_callback_placement(Hand::Left, true).await
}
