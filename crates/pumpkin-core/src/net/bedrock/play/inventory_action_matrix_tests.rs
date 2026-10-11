use super::*;
use pumpkin_data::{
    AttributeModifierSlot,
    attributes::Attributes,
    data_component_impl::{AttributeModifiersImpl, Modifier, Operation},
};
use pumpkin_protocol::{
    bedrock::{
        client::{CMobEquipment, inventory_slot::CInventorySlot},
        network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
    },
    codec::item_stack_seralizer::ItemStackSerializer,
};
use std::borrow::Cow;

#[path = "inventory_action_equipment_tests.rs"]
mod equipment;

#[path = "inventory_action_owner_review_tests.rs"]
mod owner_review;

#[path = "inventory_action_placement_snapshot_tests.rs"]
mod placement_snapshot;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
enum BlockUse {
    Anchor,
    Placement,
}

struct HandUseFixture {
    world: DeathTestWorld,
    owner: TestBedrockPlayer,
    java_observer: TestPlayer,
    bedrock_observer: TestBedrockPlayer,
}

impl HandUseFixture {
    async fn new() -> Self {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(&world, proto(&Biome::PLAINS, &Block::AIR));
        let owner = TestBedrockPlayer::new(&world).await;
        owner
            .player
            .permission_lvl
            .store(pumpkin_util::permission::PermissionLvl::Four);
        let java_observer = TestPlayer::new(&world);
        let bedrock_observer = TestBedrockPlayer::new(&world).await;
        world.players.store(Arc::new(vec![
            owner.player.clone(),
            java_observer.player.clone(),
            bedrock_observer.player.clone(),
        ]));
        let tracked = world
            .entity_tracker
            .get_tracked_entity(owner.player.entity_id());
        assert!(tracked.is_some());
        if let Some(tracked) = tracked {
            tracked.seen_by.insert(java_observer.player.gameprofile.id);
            tracked
                .seen_by
                .insert(bedrock_observer.player.gameprofile.id);
        }
        Self {
            world: fixture,
            owner,
            java_observer,
            bedrock_observer,
        }
    }

    fn drain_packets(&mut self) {
        self.owner.take_packets();
        self.java_observer.take_packets();
        self.bedrock_observer.take_packets();
    }

    async fn close(self) {
        self.owner.close().await;
        self.bedrock_observer.close().await;
        self.world.server.shutdown().await;
    }

    async fn verify_and_close(
        mut self,
        verify: impl FnOnce(&mut Self) -> TestResult,
    ) -> TestResult {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify(&mut self)));
        self.close().await;
        match result {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

fn equipment_slot(hand: Hand) -> EquipmentSlot {
    match hand {
        Hand::Right => EquipmentSlot::MAIN_HAND,
        Hand::Left => EquipmentSlot::OFF_HAND,
    }
}

fn hand_address(hand: Hand) -> (i32, u32, ContainerName) {
    match hand {
        Hand::Right => (WINDOW_ID_INVENTORY, 4, ContainerName::Inventory),
        Hand::Left => (WINDOW_ID_OFF_HAND, 0, ContainerName::Offhand),
    }
}

fn glowstone_with_modifier(count: u8, hand: Hand) -> ItemStack {
    let mut stack = ItemStack::new(count, &Item::GLOWSTONE);
    stack.set_data_component(AttributeModifiersImpl {
        attribute_modifiers: Cow::Owned(vec![Modifier {
            r#type: &Attributes::LUCK,
            id: "test:consumed_hand",
            amount: 9.0,
            operation: Operation::AddValue,
            slot: match hand {
                Hand::Right => AttributeModifierSlot::MainHand,
                Hand::Left => AttributeModifierSlot::OffHand,
            },
        }]),
    });
    stack
}

fn assert_hands(player: &Player, hand: Hand, count: u8) {
    let main = player.inventory().held_item();
    let off = player.inventory().off_hand_item();
    let expected = match hand {
        Hand::Right => (count - 1, 2),
        Hand::Left => (2, count - 1),
    };
    assert_eq!((main.item_count, off.item_count), expected);
    let (used, unused) = if hand == Hand::Right {
        (&main, &off)
    } else {
        (&off, &main)
    };
    assert!(used.is_empty() || used.item.id == Item::GLOWSTONE.id);
    assert_eq!(unused.item.id, Item::DIAMOND.id);
    assert_eq!(u32::from(used.item_count) + 1, u32::from(count));
    assert_eq!(unused.item_count, 2);
}

fn assert_owner_packet(
    fixture: &HandUseFixture,
    owner_packets: &[bytes::Bytes],
    hand: Hand,
    expected: &ItemStack,
) -> TestResult {
    let (container_id, slot, container_name) = hand_address(hand);
    let owner_packet = CInventorySlot {
        container_id: VarUInt(container_id as u32),
        slot: VarUInt(slot),
        full_container_name: Some(FullContainerName {
            container_name,
            dynamic_id: None,
        }),
        storage_item: None,
        item: NetworkItemStackDescriptor::from(expected),
    };
    let encoded = fixture.owner.client().serialize_packet(&owner_packet)?;
    let slot_updates = owner_packets.iter().filter(|p| **p == encoded).count();
    if hand == Hand::Left {
        assert_eq!(slot_updates, 1, "owner off-hand correction");
    } else {
        let mut slots = vec![NetworkItemStackDescriptor::default(); PlayerInventory::MAIN_SIZE];
        slots[4] = NetworkItemStackDescriptor::from(expected);
        let content = CInventoryContent {
            container_id: VarUInt(WINDOW_ID_INVENTORY as u32),
            slots,
            full_container_name: FullContainerName {
                container_name: ContainerName::Inventory,
                dynamic_id: None,
            },
            storage_item: NetworkItemStackDescriptor::default(),
        };
        let encoded = fixture.owner.client().serialize_packet(&content)?;
        let content_updates = owner_packets.iter().filter(|p| **p == encoded).count();
        assert!(
            slot_updates <= 1 && (slot_updates == 1 || content_updates == 1),
            "owner main-hand correction"
        );
    }
    Ok(())
}

fn assert_packets(
    fixture: &mut HandUseFixture,
    hand: Hand,
    expected: &ItemStack,
    retains_modifier: bool,
) -> TestResult {
    let owner_packets = fixture.owner.take_packets();
    let java_packets = fixture.java_observer.take_packets();
    let bedrock_packets = fixture.bedrock_observer.take_packets();
    assert_owner_packet(fixture, &owner_packets, hand, expected)?;
    let (container_id, _, _) = hand_address(hand);

    let entity_id = fixture.owner.player.entity_id();
    let java_packet = CSetEquipment::new(
        entity_id.into(),
        vec![(
            equipment_slot(hand).discriminant(),
            ItemStackSerializer::from(expected.clone()),
        )],
    );
    let encoded = fixture
        .java_observer
        .client()
        .serialize_packet(&java_packet)?;
    assert_eq!(
        java_packets.iter().filter(|p| **p == encoded).count(),
        1,
        "Java observer equipment"
    );

    let bedrock_packet = CMobEquipment {
        target_runtime_id: (entity_id as u64).into(),
        item: NetworkItemStackDescriptor::from(expected),
        slot: if hand == Hand::Right { 4 } else { 0 },
        selected_slot: 4,
        container_id: container_id as u8,
    };
    let encoded = fixture
        .bedrock_observer
        .client()
        .serialize_packet(&bedrock_packet)?;
    assert_eq!(
        bedrock_packets.iter().filter(|p| **p == encoded).count(),
        1,
        "Bedrock observer equipment"
    );
    assert_attribute_packets(
        fixture,
        &owner_packets,
        &java_packets,
        &bedrock_packets,
        retains_modifier,
    )?;
    Ok(())
}

fn assert_attribute_packets(
    fixture: &HandUseFixture,
    owner_packets: &[bytes::Bytes],
    java_packets: &[bytes::Bytes],
    bedrock_packets: &[bytes::Bytes],
    retains_modifier: bool,
) -> TestResult {
    use pumpkin_protocol::bedrock::client::update_attributes::{
        AttributeData, CUpdateAttributes as BedrockAttributes,
    };
    use pumpkin_protocol::java::client::play::{AttributeModifier, CUpdateAttributes, Property};
    let entity_id = fixture.owner.player.entity_id();
    let modifiers = if retains_modifier {
        vec![AttributeModifier::new("test:consumed_hand".into(), 9.0, 0)]
    } else {
        Vec::new()
    };
    let java = CUpdateAttributes::new(
        entity_id.into(),
        vec![Property::new(
            VarInt(i32::from(Attributes::LUCK.id)),
            2.0,
            modifiers,
        )],
    );
    let expected = fixture.java_observer.client().serialize_packet(&java)?;
    assert_eq!(
        java_packets.iter().filter(|p| **p == expected).count(),
        1,
        "Java modifier refresh"
    );
    let bedrock = BedrockAttributes {
        target_runtime_id: (entity_id as u64).into(),
        attribute_list: vec![AttributeData {
            min_value: 0.0,
            max_value: f32::MAX,
            current_value: if retains_modifier { 11.0 } else { 2.0 },
            default_min_value: 0.0,
            default_max_value: f32::MAX,
            default_value: 2.0,
            name: "minecraft:luck".into(),
            modifiers: Vec::new(),
        }],
        tick: 0u64.into(),
    };
    let expected = fixture.owner.client().serialize_packet(&bedrock)?;
    assert_eq!(
        owner_packets.iter().filter(|p| **p == expected).count(),
        1,
        "owner modifier refresh"
    );
    let expected = fixture
        .bedrock_observer
        .client()
        .serialize_packet(&bedrock)?;
    assert_eq!(
        bedrock_packets.iter().filter(|p| **p == expected).count(),
        1,
        "Bedrock modifier refresh"
    );
    Ok(())
}

fn prepare_hand_use(
    fixture: &HandUseFixture,
    kind: BlockUse,
    hand: Hand,
    count: u8,
) -> (BlockPos, ItemStack) {
    let world = fixture.world.world();
    let position = BlockPos::new(8, 64, 8);
    let clicked_block = match kind {
        BlockUse::Anchor => &Block::RESPAWN_ANCHOR,
        BlockUse::Placement => &Block::STONE,
    };
    world.set_block_state(
        &position,
        clicked_block.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let player = &fixture.owner.player;
    player.inventory().set_selected_slot(4);
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::DIAMOND));
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::new(2, &Item::DIAMOND));
    let before = glowstone_with_modifier(count, hand);
    player.inventory().set_stack_in_hand(hand, before.clone());
    player
        .living_entity
        .set_attribute_base(&Attributes::LUCK, 2.0);
    player
        .living_entity
        .send_equipment_changes(&[(equipment_slot(hand), before.clone())]);
    assert_eq!(
        player.living_entity.get_attribute_value(&Attributes::LUCK),
        11.0
    );

    (position, before)
}

async fn run_hand_use(kind: BlockUse, hand: Hand, actions: bool, count: u8) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let (position, before) = prepare_hand_use(&fixture, kind, hand, count);

    // Existing descriptor_to_stack reconstructs components from game data for action writes.
    let after_action = if count == 1 {
        ItemStack::EMPTY.clone()
    } else {
        ItemStack::new(count - 1, &Item::GLOWSTONE)
    };
    let (container_id, slot, _) = hand_address(hand);
    let actions = if actions {
        vec![action(
            Some(container_id),
            slot,
            NetworkItemDescriptor::from(&before),
            NetworkItemDescriptor::from(&after_action),
        )]
    } else {
        Vec::new()
    };
    let supplied_by_actions = !actions.is_empty();
    let mut packet = click_block_packet(
        match hand {
            Hand::Right => HandSlot::Mainhand,
            Hand::Left => HandSlot::Offhand,
        },
        position,
        &before,
        actions,
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.hot_bar_slot = VarInt(4);
    }
    fixture.drain_packets();

    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);

    fixture
        .verify_and_close(|fixture| {
            match kind {
                BlockUse::Anchor => assert_eq!(
                    RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&position))
                        .charges,
                    1
                ),
                BlockUse::Placement => {
                    assert_eq!(world.get_block(&BlockPos::new(8, 65, 8)), &Block::GLOWSTONE);
                }
            }
            let player = &fixture.owner.player;
            assert_hands(player, hand, count);
            let mut expected = if supplied_by_actions {
                after_action
            } else {
                let mut after = before;
                after.decrement(1);
                after
            };
            let actual = player.inventory().get_stack_in_hand(hand);
            assert!((actual.is_empty() && expected.is_empty()) || actual.are_equal(&expected));
            if supplied_by_actions {
                // Action descriptors omit stack IDs; decoding allocates a new server ID.
                expected.uid = actual.uid;
            }
            let expected_luck = if supplied_by_actions || count == 1 {
                2.0
            } else {
                11.0
            };
            assert_eq!(
                player.living_entity.get_attribute_value(&Attributes::LUCK),
                expected_luck,
                "equipment modifiers must match the authoritative hand after consumption"
            );
            assert_packets(fixture, hand, &expected, !supplied_by_actions && count > 1)
        })
        .await
}

macro_rules! hand_use_test {
    ($name:ident, $kind:ident, $actions:literal, $count:literal) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() -> TestResult {
            run_hand_use(BlockUse::$kind, Hand::Left, $actions, $count).await?;
            run_hand_use(BlockUse::$kind, Hand::Right, $actions, $count).await
        }
    };
}

hand_use_test!(anchor_no_actions_stack, Anchor, false, 64);
hand_use_test!(anchor_no_actions_last, Anchor, false, 1);
hand_use_test!(anchor_actions_stack, Anchor, true, 64);
hand_use_test!(anchor_actions_last, Anchor, true, 1);
hand_use_test!(placement_no_actions_stack, Placement, false, 64);
hand_use_test!(placement_no_actions_last, Placement, false, 1);
hand_use_test!(placement_actions_stack, Placement, true, 64);
hand_use_test!(placement_actions_last, Placement, true, 1);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_item_break_status_uses_offhand() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let mut tool = ItemStack::new(1, &Item::FLINT_AND_STEEL);
    let max_damage = tool.get_max_damage().ok_or("tool must be damageable")?;
    tool.set_damage(max_damage - 1);
    let main = ItemStack::new(2, &Item::DIAMOND);
    fixture
        .owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, main.clone());
    fixture
        .owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, tool.clone());
    fixture.drain_packets();
    let packet = click_block_packet(HandSlot::Offhand, position, &tool, Vec::new());

    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);

    fixture
        .verify_and_close(|fixture| {
            assert_eq!(world.get_block(&BlockPos::new(8, 65, 8)), &Block::FIRE);
            assert!(fixture.owner.player.inventory().off_hand_item().is_empty());
            assert!(
                fixture
                    .owner
                    .player
                    .inventory()
                    .held_item()
                    .are_equal(&main)
            );
            let packet = pumpkin_protocol::java::client::play::CEntityStatus::new(
                fixture.owner.player.entity_id(),
                crate::entity::equipment_break_status(&EquipmentSlot::OFF_HAND) as i8,
            );
            let expected = fixture.java_observer.client().serialize_packet(&packet)?;
            let java_packets = fixture.java_observer.take_packets();
            assert!(java_packets.contains(&expected), "off-hand break status");
            equipment::assert_empty_offhand_packets(fixture, &java_packets)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_placement_keeps_the_originating_box_contents() -> TestResult {
    use pumpkin_data::data_component_impl::ContainerImpl;
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let mut main = ItemStack::new(1, &Item::SHULKER_BOX);
    main.set_data_component(ContainerImpl {
        items: vec![(0, ItemStack::new(7, &Item::IRON_INGOT))],
    });
    let mut off = ItemStack::new(1, &Item::SHULKER_BOX);
    off.set_data_component(ContainerImpl {
        items: vec![(0, ItemStack::new(13, &Item::DIAMOND))],
    });
    fixture
        .owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, main.clone());
    fixture
        .owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, off.clone());
    let packet = click_block_packet(HandSlot::Offhand, position, &off, Vec::new());
    fixture.drain_packets();

    fixture
        .owner
        .client()
        .handle_inventory_action(&fixture.owner.player, packet);

    fixture
        .verify_and_close(|fixture| {
            assert!(
                fixture
                    .owner
                    .player
                    .inventory()
                    .held_item()
                    .are_equal(&main)
            );
            assert!(fixture.owner.player.inventory().off_hand_item().is_empty());
            let placed = BlockPos::new(8, 65, 8);
            assert_eq!(world.get_block(&placed), &Block::SHULKER_BOX);
            let inventory = world
                .get_block_entity(&placed)
                .and_then(crate::block::entities::BlockEntity::get_inventory)
                .ok_or("placed shulker inventory")?;
            // BlockItem.updateBlockEntityComponents uses the stack from the placement hand.
            assert!(
                inventory
                    .get_stack(0)
                    .are_equal(&ItemStack::new(13, &Item::DIAMOND))
            );
            let java_packets = fixture.java_observer.take_packets();
            equipment::assert_empty_offhand_packets(fixture, &java_packets)
        })
        .await
}
