use super::callback::{assert_observer_hand, pre_use_equipment_actions, thrown_eggs};
use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::{
        player::player_interact_entity_event::PlayerInteractEntityEvent,
        server::packet::PacketSentEvent,
    },
    block::block_place::BlockPlaceEvent,
};
use crate::server::Server;
use pumpkin_data::{entity::EntityType, packet::CURRENT_MC_VERSION};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    MultiVersionJavaPacket,
    bedrock::server::inventory_transaction::{
        ReleaseItemTransactionData, UseItemOnEntityTransactionData,
    },
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

struct CancelEntity;
impl EventHandler<PlayerInteractEntityEvent> for CancelEntity {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

async fn feed_with_prediction(count: u8, cancelled: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let mut wheat = ItemStack::new(count, &Item::WHEAT);
    wheat.set_custom_name("Server wheat".to_owned());
    player.inventory().set_stack(4, wheat.clone());
    let cow = fixture.world.mob(&EntityType::COW);
    if cancelled {
        fixture
            .world
            .server
            .plugin_manager
            .register::<PlayerInteractEntityEvent, _>(
                Arc::new(CancelEntity),
                EventPriority::Normal,
                true,
            );
    }
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &wheat,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(&wheat),
            NetworkItemDescriptor::from(&wheat.copy_with_count(count - 1)),
        )],
    );
    packet.transaction_type = VarUInt(3);
    packet.transaction_data = TransactionData::UseItemOnEntity(UseItemOnEntityTransactionData {
        target_entity_runtime_id: (cow.get_entity().entity_id as u64).into(),
        action_type: VarInt(0),
        hot_bar_slot: VarInt(4),
        item_in_hand: NetworkItemDescriptor::from(&wheat),
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
            let mob = cow.get_mob().ok_or("cow mob")?.get_mob_entity();
            assert_eq!(
                mob.is_in_love(),
                !cancelled,
                "last wheat must still feed the cow"
            );
            let mut expected = wheat.clone();
            expected.decrement(u8::from(!cancelled));
            if expected.is_empty() {
                expected = ItemStack::EMPTY.clone();
            }
            let actual = player.inventory().held_item();
            assert!(
                actual.are_equal(&expected),
                "entity prediction must not debit the server hand"
            );
            if !expected.is_empty() {
                assert_eq!(actual.uid, wheat.uid);
            }
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Right, &expected)?;
            assert_observer_hand(fixture, Hand::Right, &expected)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_entity_two_to_one_consumes_once() -> TestResult {
    feed_with_prediction(2, false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_entity_last_wheat_interacts() -> TestResult {
    feed_with_prediction(1, false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_cancelled_entity_use_preserves_wheat() -> TestResult {
    feed_with_prediction(2, true).await
}

struct ReplaceOpposite {
    player: Arc<Player>,
    hand: Hand,
    replacement: ItemStack,
    fired: AtomicBool,
}
impl EventHandler<BlockPlaceEvent> for ReplaceOpposite {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        self.fired.store(true, SeqCst);
        self.player
            .inventory()
            .set_stack_in_hand(self.hand, self.replacement.clone());
        Box::pin(async {})
    }
}

pub(super) fn attack_damage(player: &Arc<Player>, victim: &Arc<Player>) -> f32 {
    player
        .get_entity()
        .on_ground
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let health = victim.living_entity.health.load();
    player.attack(&(victim.clone() as Arc<dyn EntityBase>));
    health - victim.living_entity.health.load()
}

async fn placement_opposite(hand: Hand) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    let (position, before) = prepare_hand_use(&fixture, BlockUse::Placement, hand, 2);
    let opposite = if hand == Hand::Right {
        Hand::Left
    } else {
        Hand::Right
    };
    let initial = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player
        .inventory()
        .set_stack_in_hand(opposite, initial.clone());
    player
        .living_entity
        .send_equipment_changes(&[(equipment_slot(opposite), initial)]);
    fixture.owner.player.tick(&fixture.world.server);
    let replacement = if hand == Hand::Right {
        ItemStack::new(3, &Item::EMERALD)
    } else {
        ItemStack::EMPTY.clone()
    };
    let control = Arc::new(ReplaceOpposite {
        player: player.clone(),
        hand: opposite,
        replacement: replacement.clone(),
        fired: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(control.clone(), EventPriority::Normal, true);
    fixture.drain_packets();
    fixture.owner.client().handle_inventory_action(
        &player,
        click_block_packet(
            &player,
            if hand == Hand::Right {
                HandSlot::Mainhand
            } else {
                HandSlot::Offhand
            },
            position,
            &before,
            Vec::new(),
        ),
    );
    fixture
        .verify_and_close(|fixture| {
            assert!(control.fired.load(SeqCst));
            assert_eq!(player.inventory().get_selected_slot(), 4);
            assert_eq!(
                fixture.world.world().get_block(&BlockPos::new(8, 65, 8)),
                &Block::GLOWSTONE
            );
            assert!(
                player
                    .inventory()
                    .get_stack_in_hand(opposite)
                    .are_equal(&replacement)
            );
            assert_eq!(
                attack_damage(&player, &fixture.java_observer.player),
                1.0,
                "real attack must use the live main hand"
            );
            let mut expected = before.clone();
            expected.decrement(1);
            let main = if hand == Hand::Right {
                expected.clone()
            } else {
                replacement.clone()
            };
            let off = if hand == Hand::Left {
                expected.clone()
            } else {
                replacement.clone()
            };
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Left, &off)?;
            assert_owner_packet(fixture, &owner, Hand::Right, &main)?;
            assert_both_hand_packets(fixture, &player, main, off)?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_main_placement_publishes_offhand() -> TestResult {
    placement_opposite(Hand::Right).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_offhand_placement_clears_mainhand_damage() -> TestResult {
    placement_opposite(Hand::Left).await
}

fn assert_both_hand_packets(
    fixture: &mut HandUseFixture,
    player: &Player,
    main: ItemStack,
    off: ItemStack,
) -> TestResult {
    let java = CSetEquipment::new(
        player.entity_id().into(),
        vec![
            (
                EquipmentSlot::MAIN_HAND.discriminant(),
                ItemStackSerializer::from(main.clone()),
            ),
            (
                EquipmentSlot::OFF_HAND.discriminant(),
                ItemStackSerializer::from(off.clone()),
            ),
        ],
    );
    let encoded = fixture.java_observer.client().serialize_packet(&java)?;
    let packets = fixture.java_observer.take_packets();
    if !packets.contains(&encoded) {
        // A publication callback can change the opposite hand after the first batch.
        for (slot, stack) in [
            (EquipmentSlot::MAIN_HAND, &main),
            (EquipmentSlot::OFF_HAND, &off),
        ] {
            let packet = CSetEquipment::new(
                player.entity_id().into(),
                vec![(
                    slot.discriminant(),
                    ItemStackSerializer::from(stack.clone()),
                )],
            );
            let encoded = fixture.java_observer.client().serialize_packet(&packet)?;
            assert!(
                packets.contains(&encoded),
                "Java must publish each live hand after the callback"
            );
        }
    }
    let packets = fixture.bedrock_observer.take_packets();
    for (hand, stack) in [(Hand::Right, main), (Hand::Left, off)] {
        let packet = CMobEquipment {
            target_runtime_id: (player.entity_id() as u64).into(),
            item: NetworkItemStackDescriptor::from(&stack),
            slot: if hand == Hand::Right { 4 } else { 0 },
            selected_slot: 4,
            container_id: if hand == Hand::Right {
                WINDOW_ID_INVENTORY as u8
            } else {
                WINDOW_ID_OFF_HAND as u8
            },
        };
        let encoded = fixture
            .bedrock_observer
            .client()
            .serialize_packet(&packet)?;
        assert!(
            packets.contains(&encoded),
            "Bedrock must publish both changed hands"
        );
    }
    Ok(())
}

struct SelectOnEquipment {
    player: Arc<Player>,
    fired: AtomicBool,
}
impl EventHandler<PacketSentEvent> for SelectOnEquipment {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.player.inventory().set_selected_slot(8);
        }
        Box::pin(async {})
    }
}
fn selection_callback(fixture: &HandUseFixture) -> Arc<SelectOnEquipment> {
    let java = fixture.java_observer.client();
    java.set_player(fixture.java_observer.player.clone());
    java.version.store(JavaMinecraftVersion::V_26_2);
    let control = Arc::new(SelectOnEquipment {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_equipment_publication_commits_before_callback() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    helmet.set_data_component(AttributeModifiersImpl {
        attribute_modifiers: Cow::Owned(vec![Modifier {
            r#type: &Attributes::ATTACK_DAMAGE,
            id: "test:head_damage",
            amount: 13.0,
            operation: Operation::AddValue,
            slot: AttributeModifierSlot::Head,
        }]),
    });
    player.inventory().set_stack(4, helmet.clone());
    let control = selection_callback(&fixture);
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &helmet,
        Vec::new(),
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.action_type = VarInt(1);
    }
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert!(control.fired.load(SeqCst));
            let equipped = player
                .inventory()
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::HEAD);
            let expected_damage = 14.0;
            assert_eq!(
                attack_damage(&player, &fixture.java_observer.player),
                expected_damage,
                "real attack must follow actual equipment, with no phantom modifier"
            );
            assert!(
                equipped.are_equal(&helmet),
                "equipment must commit before its announcement"
            );
            assert!(player.inventory().get_stack(4).is_empty());
            assert_eq!(player.inventory().get_selected_slot(), 8);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_offhand_use_survives_main_selection_callback() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let eggs = ItemStack::new(2, &Item::BROWN_EGG);
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, eggs.clone());
    let actions = pre_use_equipment_actions(&player, Hand::Left);
    let control = selection_callback(&fixture);
    let mut packet = click_block_packet(
        &player,
        HandSlot::Offhand,
        BlockPos::new(8, 64, 8),
        &eggs,
        actions,
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.action_type = VarInt(1);
    }
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert!(control.fired.load(SeqCst));
            assert_eq!(player.inventory().get_selected_slot(), 8);
            assert_eq!(
                thrown_eggs(&fixture.world.world()),
                1,
                "unchanged offhand use must complete once"
            );
            assert_eq!(player.inventory().off_hand_item().item_count, 1);
            assert!(player.inventory().get_stack(4).is_empty());
            assert_eq!(player.inventory().get_stack(0).item, &Item::DIAMOND);
            Ok(())
        })
        .await
}

#[path = "inventory_action_release_tests.rs"]
mod release;

impl EventHandler<PacketSentEvent> for ReplaceOpposite {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.player
                .inventory()
                .set_stack_in_hand(self.hand, self.replacement.clone());
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4c_pre_use_callback_publishes_opposite_hand() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let eggs = ItemStack::new(2, &Item::BROWN_EGG);
    player.inventory().set_stack(4, eggs.clone());
    let actions = pre_use_equipment_actions(&player, Hand::Right);
    let mut replacement = ItemStack::new(3, &Item::EMERALD);
    replacement.set_data_component(AttributeModifiersImpl {
        attribute_modifiers: Cow::Owned(vec![Modifier {
            r#type: &Attributes::ATTACK_DAMAGE,
            id: "test:pre_use_offhand",
            amount: 5.0,
            operation: Operation::AddValue,
            slot: AttributeModifierSlot::OffHand,
        }]),
    });
    let java = fixture.java_observer.client();
    java.set_player(fixture.java_observer.player.clone());
    java.version.store(JavaMinecraftVersion::V_26_2);
    let control = Arc::new(ReplaceOpposite {
        player: player.clone(),
        hand: Hand::Left,
        replacement: replacement.clone(),
        fired: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(control.clone(), EventPriority::Normal, true);
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &eggs,
        actions,
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.action_type = VarInt(1);
    }
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert!(
                control.fired.load(SeqCst),
                "real pre-use publication callback"
            );
            assert_eq!(thrown_eggs(&fixture.world.world()), 1);
            assert_eq!(player.inventory().held_item().item_count, 1);
            assert_eq!(
                attack_damage(&player, &fixture.java_observer.player),
                6.0,
                "real attack must use the pre-use callback's live offhand modifier"
            );
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Left, &replacement)?;
            assert_both_hand_packets(
                fixture,
                &player,
                player.inventory().held_item(),
                replacement,
            )
        })
        .await
}
