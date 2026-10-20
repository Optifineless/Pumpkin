use super::*;
use crate::plugin::{BoxFuture, EventHandler, EventPriority, block::block_place::BlockPlaceEvent};
use pumpkin_data::data_component_impl::ContainerImpl;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

const CLICKED: BlockPos = BlockPos::new(8, 64, 8);
const PLACED: BlockPos = BlockPos::new(8, 65, 8);

#[derive(Clone, Copy)]
enum Edit {
    Move,
    Replace,
    Occupy,
    None,
}

struct PlacementCallback {
    edit: Edit,
    source: usize,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<BlockPlaceEvent> for PlacementCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, SeqCst);
        let inventory = event.player.inventory();
        match self.edit {
            Edit::Move => {
                let stack = inventory.remove_stack(self.source);
                inventory.set_stack(8, stack);
                inventory.set_selected_slot(8);
            }
            Edit::Replace => {
                let mut stack = inventory.get_stack(self.source);
                stack.uid = ItemStack::new(1, stack.item).uid;
                inventory.set_stack(self.source, stack);
            }
            Edit::Occupy => {
                event.player.world().set_block_state(
                    &PLACED,
                    Block::OBSIDIAN.default_state.id,
                    pumpkin_world::world::BlockFlags::FORCE_STATE,
                );
            }
            Edit::None => {}
        }
        Box::pin(async {})
    }
}

fn prepare(
    fixture: &HandUseFixture,
    player: &Arc<Player>,
    hand: Hand,
    edit: Edit,
    creative: bool,
) -> (ItemStack, Arc<AtomicUsize>, usize) {
    player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
    player.inventory().set_selected_slot(4);
    if creative {
        player.gamemode.store(pumpkin_util::GameMode::Creative);
    }
    fixture.world.world().set_block_state(
        &CLICKED,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let item = if matches!(edit, Edit::Replace) {
        &Item::STONE
    } else {
        &Item::SHULKER_BOX
    };
    let mut stack = ItemStack::new(1, item);
    if item == &Item::SHULKER_BOX {
        stack.set_data_component(ContainerImpl {
            items: vec![(0, ItemStack::new(13, &Item::DIAMOND))],
        });
    }
    let source = if hand == Hand::Right {
        4
    } else {
        PlayerInventory::OFF_HAND_SLOT
    };
    player.inventory().set_stack(source, stack.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(
            Arc::new(PlacementCallback {
                edit,
                source,
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    (stack, calls, source)
}

fn verify_stock(
    fixture: &HandUseFixture,
    player: &Player,
    stack: &ItemStack,
    source: usize,
    edit: Edit,
    creative: bool,
) -> bool {
    let inventory = player.inventory();
    let rejected = matches!(edit, Edit::Move | Edit::Replace | Edit::Occupy);
    match edit {
        Edit::Move => {
            assert!(inventory.get_stack(source).is_empty());
            assert_eq!(inventory.get_selected_slot(), 8);
            let moved = inventory.get_stack(8);
            assert!(moved.are_equal(stack));
            assert_eq!(moved.uid, stack.uid);
        }
        Edit::Replace => {
            let replacement = inventory.get_stack(source);
            assert!(replacement.are_equal(stack), "replacement stock consumed");
            assert_ne!(replacement.uid, stack.uid);
        }
        Edit::Occupy => {
            assert!(
                inventory.get_stack(source).are_equal(stack),
                "claim was not refunded"
            );
            assert_eq!(
                inventory.get_stack(source).uid,
                stack.uid,
                "refund source identity"
            );
        }
        Edit::None => {
            if creative {
                assert!(inventory.get_stack(source).are_equal(stack));
                assert_eq!(inventory.get_stack(source).uid, stack.uid);
            } else {
                assert!(inventory.get_stack(source).is_empty());
            }
        }
    }
    let world = fixture.world.world();
    if matches!(edit, Edit::Move) {
        let source_diamonds = inventory
            .get_stack(8)
            .get_data_component::<ContainerImpl>()
            .unwrap()
            .items[0]
            .1
            .item_count;
        let placed_diamonds = world
            .get_block_entity(&PLACED)
            .and_then(crate::block::entities::BlockEntity::get_inventory)
            .map_or(0, |contents| contents.get_stack(0).item_count);
        assert_eq!(
            u16::from(source_diamonds) + u16::from(placed_diamonds),
            13,
            "callback move duplicated nested diamonds between the source box and placed box"
        );
    }
    if rejected {
        assert_eq!(
            world.get_block(&PLACED),
            if matches!(edit, Edit::Occupy) {
                &Block::OBSIDIAN
            } else {
                &Block::AIR
            },
            "unpaid placement installed a block (and duplicated the filled shulker)"
        );
        assert!(world.get_block_entity(&PLACED).is_none());
    } else {
        assert_eq!(world.get_block(&PLACED), &Block::SHULKER_BOX);
        let contents = world
            .get_block_entity(&PLACED)
            .unwrap()
            .get_inventory()
            .unwrap()
            .get_stack(0);
        assert_eq!(
            (contents.item.id, contents.item_count),
            (Item::DIAMOND.id, 13)
        );
        assert_eq!(
            player.stats.lock().unwrap().get(
                pumpkin_data::statistic::StatisticCategory::Used,
                i32::from(stack.item.id)
            ),
            1
        );
    }
    rejected
}

async fn bedrock_placement(hand: Hand, edit: Edit, creative: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let (stack, calls, source) = prepare(&fixture, &fixture.owner.player, hand, edit, creative);
    let packet = click_block_packet(
        &fixture.owner.player,
        if hand == Hand::Right {
            HandSlot::Mainhand
        } else {
            HandSlot::Offhand
        },
        CLICKED,
        &stack,
        vec![action(
            Some(if hand == Hand::Right {
                WINDOW_ID_INVENTORY
            } else {
                WINDOW_ID_OFF_HAND
            }),
            if hand == Hand::Right { 4 } else { 0 },
            NetworkItemDescriptor::from(&stack),
            NetworkItemDescriptor::from(ItemStack::EMPTY),
        )],
    );
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert_eq!(calls.load(SeqCst), 1);
            if verify_stock(
                fixture,
                &fixture.owner.player,
                &stack,
                source,
                edit,
                creative,
            ) {
                let packets = fixture.owner.take_packets();
                addendum::assert_block_corrections(fixture, &packets, CLICKED)?;
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_bedrock_moved_filled_shulker_rejects_before_install() -> TestResult {
    bedrock_placement(Hand::Right, Edit::Move, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_bedrock_offhand_moved_shulker_rejects_before_install() -> TestResult {
    bedrock_placement(Hand::Left, Edit::Move, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_bedrock_replaced_uid_plain_block_rejects_before_install() -> TestResult {
    bedrock_placement(Hand::Right, Edit::Replace, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_bedrock_occupied_target_refunds_claim() -> TestResult {
    bedrock_placement(Hand::Right, Edit::Occupy, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_bedrock_normal_shulker_pays_once() -> TestResult {
    bedrock_placement(Hand::Right, Edit::None, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_bedrock_creative_shulker_keeps_stock() -> TestResult {
    bedrock_placement(Hand::Right, Edit::None, true).await
}

async fn java_placement(hand: Hand, edit: Edit, creative: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.java_observer.player.clone();
    let (stack, calls, source) = prepare(&fixture, &player, hand, edit, creative);
    let packet = pumpkin_protocol::java::server::play::SUseItemOn {
        hand: VarInt(i32::from(hand == Hand::Left)),
        position: CLICKED,
        face: VarInt(1),
        cursor_pos: Vector3::new(0.5, 1.0, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(1),
    };
    fixture.drain_packets();
    fixture
        .java_observer
        .client()
        .handle_use_item_on(&player, &packet, &fixture.world.server)?;
    fixture
        .verify_and_close(|fixture| {
            assert_eq!(calls.load(SeqCst), 1, "real Java placement callback");
            verify_stock(fixture, &player, &stack, source, edit, creative);
            let packets = fixture.java_observer.take_packets();
            for position in [CLICKED, PLACED] {
                let correction = pumpkin_protocol::java::client::play::CBlockUpdate::new(
                    position,
                    VarInt(i32::from(
                        fixture.world.world().get_block_state_id(&position).as_u16(),
                    )),
                );
                assert!(
                    packets.contains(
                        &fixture
                            .java_observer
                            .client()
                            .serialize_packet(&correction)?
                    )
                );
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_java_moved_filled_shulker_rejects_before_install() -> TestResult {
    java_placement(Hand::Right, Edit::Move, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_java_offhand_moved_shulker_rejects_before_install() -> TestResult {
    java_placement(Hand::Left, Edit::Move, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_java_replaced_uid_plain_block_rejects_before_install() -> TestResult {
    java_placement(Hand::Right, Edit::Replace, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_java_occupied_target_refunds_claim() -> TestResult {
    java_placement(Hand::Right, Edit::Occupy, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_java_normal_shulker_pays_once() -> TestResult {
    java_placement(Hand::Right, Edit::None, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r6_java_creative_shulker_keeps_stock() -> TestResult {
    java_placement(Hand::Right, Edit::None, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g06_bedrock_door_placement_control() -> TestResult {
    let fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    let lower = BlockPos::new(8, 64, 8);
    let world = fixture.world.world();
    for position in [lower.down(), lower, lower.up()] {
        world.set_block_state(
            &position,
            if position == lower.down() {
                Block::STONE.default_state.id
            } else {
                Block::AIR.default_state.id
            },
            pumpkin_world::world::BlockFlags::FORCE_STATE
                | pumpkin_world::world::BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
    player.get_entity().set_rotation(180.0, 0.0);
    player.inventory().set_selected_slot(4);
    let stack = ItemStack::new(2, &Item::OAK_DOOR);
    player.inventory().set_stack(4, stack.clone());
    let packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        lower.down(),
        &stack,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(&stack),
            NetworkItemDescriptor::from(ItemStack::EMPTY),
        )],
    );
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);

    fixture
        .verify_and_close(|fixture| {
            assert_eq!(world.get_block(&lower), &Block::OAK_DOOR);
            assert_eq!(world.get_block(&lower.up()), &Block::OAK_DOOR);
            let lower_state = pumpkin_data::block_properties::OakDoorLikeProperties::from_state_id(
                world.get_block_state_id(&lower),
            );
            assert_eq!(
                lower_state.facing,
                pumpkin_data::block_properties::HorizontalFacing::North,
            );
            assert_eq!(
                fixture.owner.player.inventory().get_stack(4).item_count,
                1,
                "Bedrock placement pays once through its handler"
            );
            Ok(())
        })
        .await
}
