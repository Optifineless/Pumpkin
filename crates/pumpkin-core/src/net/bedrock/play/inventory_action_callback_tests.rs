use super::*;
use crate::{
    entity::projectile::egg::EggEntity,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::server::packet::PacketSentEvent,
        block::block_place::BlockPlaceEvent,
    },
    server::Server,
};
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_inventory::Inventory;
use pumpkin_protocol::MultiVersionJavaPacket;
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

/// Replaces the owner's hand while the observer's equipment packet is translated.
struct ReplaceDuringEquipment {
    owner: Arc<Player>,
    hand: Hand,
    replacement: ItemStack,
    fired: AtomicBool,
}

impl EventHandler<PacketSentEvent> for ReplaceDuringEquipment {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.owner
                .inventory()
                .set_stack_in_hand(self.hand, self.replacement.clone());
        }
        Box::pin(async {})
    }
}

pub(super) fn thrown_eggs(world: &crate::world::World) -> usize {
    world
        .entities
        .load()
        .iter()
        .filter(|entity| {
            let entity: &dyn std::any::Any = entity.as_ref();
            entity.is::<EggEntity>()
        })
        .count()
}

pub(super) fn pre_use_equipment_actions(player: &Player, hand: Hand) -> Vec<InventoryAction> {
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.inventory().set_stack(0, diamond.clone());
    let (container, slot, _) = hand_address(if hand == Hand::Right {
        Hand::Left
    } else {
        Hand::Right
    });
    vec![
        action(
            Some(WINDOW_ID_INVENTORY),
            0,
            NetworkItemDescriptor::from(&diamond),
            NetworkItemDescriptor::default(),
        ),
        action(
            Some(container),
            slot,
            NetworkItemDescriptor::default(),
            NetworkItemDescriptor::from(&diamond),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn equipment_callback_replacement_survives_click_air_reconciliation() -> TestResult {
    for hand in [Hand::Left, Hand::Right] {
        let mut fixture = HandUseFixture::new().await;
        let player = fixture.owner.player.clone();
        player.inventory().set_selected_slot(4);
        let mut before = ItemStack::new(2, &Item::BROWN_EGG);
        before.set_custom_name("Server egg".to_owned());
        player.inventory().set_stack_in_hand(hand, before.clone());
        let mut replacement = ItemStack::new(8, &Item::BROWN_EGG);
        replacement.set_custom_name("Callback egg".to_owned());
        // PacketSentEvent runs synchronously for a tracking Java client on an older protocol.
        let java = fixture.java_observer.client();
        java.set_player(fixture.java_observer.player.clone());
        java.version.store(JavaMinecraftVersion::V_26_2);
        fixture
            .world
            .server
            .plugin_manager
            .register::<PacketSentEvent, _>(
                Arc::new(ReplaceDuringEquipment {
                    owner: player.clone(),
                    hand,
                    replacement: replacement.clone(),
                    fired: AtomicBool::new(false),
                }),
                EventPriority::Normal,
                true,
            );
        let (container, slot, _) = hand_address(hand);
        let mut actions = pre_use_equipment_actions(&player, hand);
        actions.insert(
            0,
            action(
                Some(container),
                slot,
                NetworkItemDescriptor::from(&before),
                NetworkItemDescriptor::from(&before.copy_with_count(1)),
            ),
        );
        let mut packet = click_block_packet(
            &fixture.owner.player,
            if hand == Hand::Left {
                HandSlot::Offhand
            } else {
                HandSlot::Mainhand
            },
            BlockPos::new(8, 64, 8),
            &before,
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
        let world = fixture.world.world();
        fixture
            .verify_and_close(|fixture| {
                let thrown = thrown_eggs(&world);
                assert_eq!(thrown, 0, "a replaced originating stack must skip use");
                let actual = fixture.owner.player.inventory().get_stack_in_hand(hand);
                assert!(
                    actual.are_items_and_components_equal(&replacement),
                    "the pre-callback snapshot overwrote the callback's stack"
                );
                assert_eq!(
                    usize::from(actual.item_count),
                    8,
                    "the callback's replacement is not consumed"
                );
                assert_eq!(actual.uid, replacement.uid);
                let owner_packets = fixture.owner.take_packets();
                assert_owner_packet(fixture, &owner_packets, hand, &actual)?;
                assert_observer_hand(fixture, hand, &actual)
            })
            .await?;
    }
    Ok(())
}

pub(super) struct SelectSlot(pub(super) Arc<AtomicUsize>);

impl EventHandler<BlockPlaceEvent> for SelectSlot {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, SeqCst);
        event.player.inventory().set_selected_slot(8);
        Box::pin(async {})
    }
}

// ItemAttributeModifiers for the main hand, read from the stack's own components.
fn attack_damage_bonus(stack: &ItemStack) -> f64 {
    stack
        .get_data_component::<AttributeModifiersImpl>()
        .map(|component| {
            component
                .attribute_modifiers
                .iter()
                .filter(|modifier| {
                    modifier.r#type.id == Attributes::ATTACK_DAMAGE.id
                        && matches!(modifier.operation, Operation::AddValue)
                })
                .map(|modifier| modifier.amount)
                .sum()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn placement_callback_selection_publishes_the_live_main_hand() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let player = fixture.owner.player.clone();
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    player.inventory().set_selected_slot(4);
    let blocks = ItemStack::new(2, &Item::STONE);
    player.inventory().set_stack(4, blocks.clone());
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player.inventory().set_stack(8, sword.clone());
    player
        .living_entity
        .send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, blocks.clone())]);
    let base = player
        .living_entity
        .get_attribute_value(&Attributes::ATTACK_DAMAGE);
    let sword_bonus = attack_damage_bonus(&sword);
    assert!(sword_bonus > 0.0);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(
            Arc::new(SelectSlot(calls.clone())),
            EventPriority::Normal,
            true,
        );
    fixture.drain_packets();
    fixture.owner.client().handle_inventory_action(
        &player,
        click_block_packet(
            &fixture.owner.player,
            HandSlot::Mainhand,
            position,
            &blocks,
            Vec::new(),
        ),
    );
    fixture
        .verify_and_close(|fixture| {
            let player = fixture.owner.player.clone();
            assert_eq!(calls.load(SeqCst), 1, "real placement callback");
            assert_eq!(world.get_block(&BlockPos::new(8, 65, 8)), &Block::STONE);
            assert_eq!(player.inventory().get_selected_slot(), 8);
            let source = player.inventory().get_stack(4);
            assert_eq!(source.item_count, 1, "originating slot pays once");
            assert_eq!(
                player
                    .living_entity
                    .get_attribute_value(&Attributes::ATTACK_DAMAGE),
                base + sword_bonus,
                "cached attack damage must follow the callback-selected sword"
            );
            let owner_packets = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner_packets, Hand::Right, &source)?;
            let entity_id = player.entity_id();
            let java = CSetEquipment::new(
                entity_id.into(),
                vec![(
                    EquipmentSlot::MAIN_HAND.discriminant(),
                    ItemStackSerializer::from(sword.clone()),
                )],
            );
            let java = fixture.java_observer.client().serialize_packet(&java)?;
            let java_packets = fixture.java_observer.take_packets();
            assert_eq!(
                java_packets.iter().filter(|p| **p == java).count(),
                1,
                "Java observer selected-sword equipment"
            );
            let bedrock = CMobEquipment {
                target_runtime_id: (entity_id as u64).into(),
                item: NetworkItemStackDescriptor::from(&sword),
                slot: 8,
                selected_slot: 8,
                container_id: WINDOW_ID_INVENTORY as u8,
            };
            let bedrock = fixture
                .bedrock_observer
                .client()
                .serialize_packet(&bedrock)?;
            let bedrock_packets = fixture.bedrock_observer.take_packets();
            assert_eq!(
                bedrock_packets.iter().filter(|p| **p == bedrock).count(),
                1,
                "Bedrock observer selected-sword equipment"
            );
            Ok(())
        })
        .await
}

pub(super) fn assert_observer_hand(
    fixture: &mut HandUseFixture,
    hand: Hand,
    expected: &ItemStack,
) -> TestResult {
    let entity_id = fixture.owner.player.entity_id();
    let java = CSetEquipment::new(
        entity_id.into(),
        vec![(
            equipment_slot(hand).discriminant(),
            ItemStackSerializer::from(expected.clone()),
        )],
    );
    let java = fixture.java_observer.client().serialize_packet(&java)?;
    let java_packets = fixture.java_observer.take_packets();
    assert_eq!(
        java_packets.iter().filter(|p| **p == java).count(),
        1,
        "Java observer authoritative hand"
    );
    let (container_id, slot, _) = hand_address(hand);
    let bedrock = CMobEquipment {
        target_runtime_id: (entity_id as u64).into(),
        item: NetworkItemStackDescriptor::from(expected),
        slot: slot as u8,
        selected_slot: 4,
        container_id: container_id as u8,
    };
    let bedrock = fixture
        .bedrock_observer
        .client()
        .serialize_packet(&bedrock)?;
    let bedrock_packets = fixture.bedrock_observer.take_packets();
    assert_eq!(
        bedrock_packets.iter().filter(|p| **p == bedrock).count(),
        1,
        "Bedrock observer authoritative hand"
    );
    Ok(())
}

pub(super) struct CancelPlacement;

impl EventHandler<BlockPlaceEvent> for CancelPlacement {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_last_item_placement_restores_the_predicted_stack() -> TestResult {
    use pumpkin_data::data_component_impl::ContainerImpl;
    for hand in [Hand::Left, Hand::Right] {
        let mut fixture = HandUseFixture::new().await;
        let world = fixture.world.world();
        let player = fixture.owner.player.clone();
        let position = BlockPos::new(8, 64, 8);
        world.set_block_state(
            &position,
            Block::STONE.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        player.inventory().set_selected_slot(4);
        let mut original = ItemStack::new(1, &Item::SHULKER_BOX);
        original.set_data_component(ContainerImpl {
            items: vec![(0, ItemStack::new(13, &Item::DIAMOND))],
        });
        player.inventory().set_stack_in_hand(hand, original.clone());
        fixture
            .world
            .server
            .plugin_manager
            .register::<BlockPlaceEvent, _>(Arc::new(CancelPlacement), EventPriority::Normal, true);
        let (container, slot, _) = hand_address(hand);
        let packet = click_block_packet(
            &fixture.owner.player,
            if hand == Hand::Left {
                HandSlot::Offhand
            } else {
                HandSlot::Mainhand
            },
            position,
            &original,
            vec![action(
                Some(container),
                slot,
                NetworkItemDescriptor::from(&original),
                NetworkItemDescriptor::default(),
            )],
        );
        fixture.drain_packets();
        fixture
            .owner
            .client()
            .handle_inventory_action(&player, packet);
        fixture
            .verify_and_close(|fixture| {
                assert_eq!(world.get_block(&BlockPos::new(8, 65, 8)), &Block::AIR);
                let actual = fixture.owner.player.inventory().get_stack_in_hand(hand);
                assert!(
                    actual.are_equal(&original),
                    "a denied placement must not consume the predicted last item"
                );
                assert_eq!(actual.uid, original.uid);
                let owner_packets = fixture.owner.take_packets();
                assert_owner_packet(fixture, &owner_packets, hand, &actual)?;
                super::addendum::assert_block_corrections(fixture, &owner_packets, position)?;
                assert_observer_hand(fixture, hand, &actual)
            })
            .await?;
    }
    Ok(())
}

struct CancelInteract;

impl EventHandler<PlayerInteractEvent> for CancelInteract {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

fn named_egg_with_modifier(count: u8, hand: Hand) -> ItemStack {
    let mut stack = ItemStack::new(count, &Item::BROWN_EGG);
    stack.set_custom_name("Server egg".to_owned());
    if let Some(modifiers) = glowstone_with_modifier(count, hand)
        .get_data_component::<AttributeModifiersImpl>()
        .cloned()
    {
        stack.set_data_component(modifiers);
    }
    stack
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_two_egg_air_use_republishes_the_authoritative_hand() -> TestResult {
    for cancelled in [false, true] {
        for hand in [Hand::Left, Hand::Right] {
            let mut fixture = HandUseFixture::new().await;
            let player = fixture.owner.player.clone();
            player.inventory().set_selected_slot(4);
            let before = named_egg_with_modifier(2, hand);
            player.inventory().set_stack_in_hand(hand, before.clone());
            player
                .living_entity
                .set_attribute_base(&Attributes::LUCK, 2.0);
            player
                .living_entity
                .send_equipment_changes(&[(equipment_slot(hand), before.clone())]);
            let modified_luck = player.living_entity.get_attribute_value(&Attributes::LUCK);
            assert!(modified_luck > 2.0);
            if cancelled {
                fixture
                    .world
                    .server
                    .plugin_manager
                    .register::<PlayerInteractEvent, _>(
                        Arc::new(CancelInteract),
                        EventPriority::Normal,
                        true,
                    );
            }
            let (container, slot, _) = hand_address(hand);
            let mut packet = click_block_packet(
                &fixture.owner.player,
                if hand == Hand::Left {
                    HandSlot::Offhand
                } else {
                    HandSlot::Mainhand
                },
                BlockPos::new(8, 64, 8),
                &before,
                vec![action(
                    Some(container),
                    slot,
                    NetworkItemDescriptor::from(&before),
                    NetworkItemDescriptor::from(&before.copy_with_count(1)),
                )],
            );
            if let TransactionData::UseItem(data) = &mut packet.transaction_data {
                data.action_type = VarInt(1);
            }
            fixture.drain_packets();
            fixture
                .owner
                .client()
                .handle_inventory_action(&player, packet);
            let world = fixture.world.world();
            fixture
                .verify_and_close(|fixture| {
                    assert_eq!(thrown_eggs(&world), usize::from(!cancelled));
                    let actual = fixture.owner.player.inventory().get_stack_in_hand(hand);
                    assert_eq!(actual.item_count, if cancelled { 2 } else { 1 });
                    assert!(actual.are_items_and_components_equal(&before));
                    assert_eq!(
                        fixture
                            .owner
                            .player
                            .living_entity
                            .get_attribute_value(&Attributes::LUCK),
                        modified_luck,
                        "hand modifiers must follow the authoritative stack"
                    );
                    let owner_packets = fixture.owner.take_packets();
                    assert_owner_packet(fixture, &owner_packets, hand, &actual)?;
                    assert_observer_hand(fixture, hand, &actual)
                })
                .await?;
        }
    }
    Ok(())
}
