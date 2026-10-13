use super::*;
use pumpkin_protocol::bedrock::server::inventory_transaction::NormalTransactionData;
use pumpkin_world::world::BlockFlags;

fn use_offhand(stack: &ItemStack, action: i32, position: BlockPos) -> TransactionData {
    let mut data = use_item(stack, action, position);
    if let TransactionData::UseItem(data) = &mut data {
        data.hand = HandSlot::Offhand;
    }
    data
}

fn assert_single_slot(
    fixture: &mut TestBedrockPlayer,
    window: u32,
    slot: u32,
    item: &'static Item,
    amount: u8,
) {
    let packets = fixture.take_packets();
    assert!(packets.iter().all(|p| packet_payload(p, 49).is_none()));
    let slots: Vec<_> = packets
        .iter()
        .filter_map(|p| packet_payload(p, 50))
        .collect();
    assert_eq!(slots.len(), 1, "expected only the changed hand slot");
    let mut bytes = slots[0];
    assert_eq!(VarUInt::read(&mut bytes).unwrap().0, window);
    assert_eq!(VarUInt::read(&mut bytes).unwrap().0, slot);
    assert!(bool::read(&mut bytes).unwrap());
    let name = FullContainerName::read(&mut bytes).unwrap();
    assert_eq!(
        name.container_name,
        if window == 119 {
            ContainerName::Offhand
        } else {
            ContainerName::Inventory
        }
    );
    assert!(!bool::read(&mut bytes).unwrap());
    let stack = NetworkItemStackDescriptor::read(&mut bytes).unwrap();
    assert_eq!(
        stack.id,
        NetworkItemStackDescriptor::from(&ItemStack::new(amount, item)).id
    );
    assert_eq!(stack.stack_size, u16::from(amount));
}

async fn observer(
    world: &Arc<crate::world::World>,
    actor: &TestBedrockPlayer,
) -> TestBedrockPlayer {
    let mut observer = TestBedrockPlayer::new(world).await;
    world.players.store(Arc::new(vec![
        actor.player.clone(),
        observer.player.clone(),
    ]));
    // Pair through the tracker so its Bedrock watcher count matches the recipient set.
    let chunk = actor.player.get_entity().chunk_pos.load();
    observer.player.watched_section.store(
        pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
            chunk,
            std::num::NonZeroU8::new(2).unwrap(),
        ),
    );
    observer
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(chunk);
    let tracked = world
        .entity_tracker
        .entity_map
        .get(&actor.player.entity_id())
        .unwrap()
        .clone();
    tracked.update_player(&observer.player, world);
    observer.take_packets();
    observer
}

fn assert_no_equipment(observer: &mut TestBedrockPlayer) {
    assert!(
        observer
            .take_packets()
            .iter()
            .all(|p| packet_payload(p, 31).is_none())
    );
}

fn assert_equipment(
    observer: &mut TestBedrockPlayer,
    container: u8,
    item: &'static Item,
    amount: u8,
) {
    let packets = observer.take_packets();
    let equipment: Vec<_> = packets
        .iter()
        .filter_map(|p| packet_payload(p, 31))
        .collect();
    assert_eq!(
        equipment.len(),
        1,
        "only the changed hand equipment must be published"
    );
    let mut bytes = equipment[0];
    VarULong::read(&mut bytes).unwrap();
    let stack = NetworkItemStackDescriptor::read(&mut bytes).unwrap();
    assert_eq!(
        stack.id,
        NetworkItemStackDescriptor::from(&ItemStack::new(amount, item)).id
    );
    assert_eq!(stack.stack_size, u16::from(amount));
    u8::read(&mut bytes).unwrap();
    u8::read(&mut bytes).unwrap();
    assert_eq!(
        u8::read(&mut bytes).unwrap(),
        container,
        "equipment must name the changed hand"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_offhand_uses_block_beside_occupied_main_hand() {
    use pumpkin_data::block_properties::LeverLikeProperties;
    let server = DeathTestWorld::new().await;
    let world = server.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut fixture = TestBedrockPlayer::new(&world).await;
    let position = BlockPos::new(10, 64, 8);
    world.set_block_state(
        &position,
        Block::LEVER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let main = ItemStack::new(12, &Item::STONE);
    fixture.player.inventory().set_held_item(main.clone());
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![],
        use_offhand(ItemStack::EMPTY, 0, position),
    );
    // ServerPlayerGameMode.useItemOn only tries default block use for the main hand.
    assert!(!LeverLikeProperties::from_state_id(world.get_block_state_id(&position)).powered);
    assert!(fixture.player.inventory().held_item().are_equal(&main));
    assert!(fixture.player.inventory().off_hand_item().is_empty());
    assert_conserved(before, &fixture.player, 0);
    assert!(
        fixture
            .take_packets()
            .iter()
            .all(|p| packet_payload(p, 49).is_none() && packet_payload(p, 50).is_none())
    );
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_block_placement_corrects_only_offhand_and_equipment() {
    let server = DeathTestWorld::new().await;
    let world = server.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut fixture = TestBedrockPlayer::new(&world).await;
    fixture.player.permission_lvl.store(PermissionLvl::Four);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let position = BlockPos::new(10, 63, 8);
    let main = ItemStack::new(12, &Item::DIAMOND);
    fixture.player.inventory().set_held_item(main.clone());
    let held = ItemStack::new(2, &Item::TORCH);
    fixture
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, held.clone());
    let mut observer = observer(&world, &fixture).await;
    let before = stock(&fixture.player, &[]);
    send_use(&mut fixture, vec![], use_offhand(&held, 0, position));
    assert_eq!(world.get_block(&position.up()), &Block::TORCH);
    assert!(fixture.player.inventory().held_item().are_equal(&main));
    assert_eq!(fixture.player.inventory().off_hand_item().item_count, 1);
    assert_conserved(before, &fixture.player, 1);
    assert_single_slot(&mut fixture, 119, 0, &Item::TORCH, 1);
    assert_no_equipment(&mut observer);
    fixture.player.tick(&server.server);
    assert_single_slot(&mut fixture, 119, 0, &Item::TORCH, 1);
    assert_equipment(&mut observer, 119, &Item::TORCH, 1);
    fixture.player.tick(&server.server);
    assert_no_equipment(&mut observer);
    observer.close().await;
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_consuming_block_callback_preserves_main_hand() {
    use pumpkin_data::block_properties::RespawnAnchorLikeProperties;
    let server = DeathTestWorld::new().await;
    let world = server.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut fixture = TestBedrockPlayer::new(&world).await;
    let position = BlockPos::new(10, 64, 8);
    world.set_block_state(
        &position,
        Block::RESPAWN_ANCHOR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let main = ItemStack::new(12, &Item::DIAMOND);
    fixture.player.inventory().set_held_item(main.clone());
    let held = ItemStack::new(2, &Item::GLOWSTONE);
    fixture
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, held.clone());
    let mut observer = observer(&world, &fixture).await;
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![],
        use_offhand(&held.copy_with_count(1), 0, position),
    );
    assert_eq!(
        RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&position)).charges,
        1
    );
    assert!(fixture.player.inventory().held_item().are_equal(&main));
    assert_eq!(fixture.player.inventory().off_hand_item().item_count, 1);
    assert_conserved(before, &fixture.player, 1);
    assert_single_slot(&mut fixture, 119, 0, &Item::GLOWSTONE, 1);
    assert_no_equipment(&mut observer);
    fixture.player.tick(&server.server);
    assert_single_slot(&mut fixture, 119, 0, &Item::GLOWSTONE, 1);
    assert_equipment(&mut observer, 119, &Item::GLOWSTONE, 1);
    fixture.player.tick(&server.server);
    assert_no_equipment(&mut observer);
    observer.close().await;
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_air_use_ignores_predicted_zero_count() {
    let server = DeathTestWorld::new().await;
    let world = server.world();
    let mut fixture = TestBedrockPlayer::new(&world).await;
    let main = ItemStack::new(12, &Item::DIAMOND);
    fixture.player.inventory().set_held_item(main.clone());
    let held = ItemStack::new(2, &Item::EGG);
    fixture
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, held.clone());
    let mut data = use_offhand(&held, 1, BlockPos::new(8, 64, 8));
    if let TransactionData::UseItem(data) = &mut data {
        data.item_in_hand.stack_size = 0;
    }
    let mut observer = observer(&world, &fixture).await;
    let before = stock(&fixture.player, &[]);
    send_use(&mut fixture, vec![], data);
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|e| e.get_entity().entity_type == &EntityType::EGG)
            .count(),
        1
    );
    assert!(fixture.player.inventory().held_item().are_equal(&main));
    assert_eq!(fixture.player.inventory().off_hand_item().item_count, 1);
    assert_conserved(before, &fixture.player, 1);
    assert_single_slot(&mut fixture, 119, 0, &Item::EGG, 1);
    assert_no_equipment(&mut observer);
    fixture.player.tick(&server.server);
    assert_single_slot(&mut fixture, 119, 0, &Item::EGG, 1);
    assert_equipment(&mut observer, 119, &Item::EGG, 1);
    fixture.player.tick(&server.server);
    assert_no_equipment(&mut observer);
    observer.close().await;
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_transactions_send_no_inventory_or_equipment_packets() {
    let server = DeathTestWorld::new().await;
    let world = server.world();
    let mut fixture = TestBedrockPlayer::new(&world).await;
    let held = ItemStack::new(12, &Item::STONE);
    fixture.player.inventory().set_held_item(held.clone());
    let mut observer = observer(&world, &fixture).await;
    for data in [
        TransactionData::Normal(NormalTransactionData),
        TransactionData::UseItemOnEntity(UseItemOnEntityTransactionData {
            target_entity_runtime_id: VarULong(i32::MAX as u64),
            action_type: VarInt(0),
            hot_bar_slot: VarInt(0),
            item_in_hand: NetworkItemDescriptor::from(&held),
            player_position: Vector3::new(8.5, 64.0, 8.5),
            click_position: Vector3::default(),
        }),
        use_item(&held, 1, BlockPos::new(8, 64, 8)),
        use_item(&held, 1, BlockPos::new(8, 64, 8)),
    ] {
        send_use(&mut fixture, vec![], data);
        assert!(
            fixture
                .take_packets()
                .iter()
                .all(|p| packet_payload(p, 49).is_none() && packet_payload(p, 50).is_none())
        );
        assert!(
            observer
                .take_packets()
                .iter()
                .all(|p| packet_payload(p, 31).is_none())
        );
    }
    assert!(fixture.player.inventory().held_item().are_equal(&held));
    observer.close().await;
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_hand_consumption_updates_only_selected_slot() {
    let server = DeathTestWorld::new().await;
    let world = server.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut fixture = TestBedrockPlayer::new(&world).await;
    fixture.player.permission_lvl.store(PermissionLvl::Four);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    fixture.player.inventory().set_selected_slot(3);
    let held = ItemStack::new(64, &Item::STONE);
    fixture.player.inventory().set_held_item(held.clone());
    let mut observer = observer(&world, &fixture).await;
    let position = BlockPos::new(10, 63, 8);
    let before = stock(&fixture.player, &[]);
    let mut data = use_item(&held.copy_with_count(63), 0, position);
    if let TransactionData::UseItem(data) = &mut data {
        data.hot_bar_slot = VarInt(3);
    }
    send_use(&mut fixture, vec![], data);
    assert_eq!(world.get_block(&position.up()), &Block::STONE);
    assert_eq!(fixture.player.inventory().held_item().item_count, 63);
    assert_conserved(before, &fixture.player, 1);
    assert_single_slot(&mut fixture, 0, 3, &Item::STONE, 63);
    assert_no_equipment(&mut observer);
    fixture.player.tick(&server.server);
    assert_equipment(&mut observer, 0, &Item::STONE, 63);
    fixture.player.tick(&server.server);
    assert_no_equipment(&mut observer);
    observer.close().await;
    fixture.close().await;
    server.server.shutdown().await;
}

fn assert_attack_damage_update(fixture: &mut TestBedrockPlayer, base_damage: f64) {
    let packets = fixture.take_packets();
    assert!(
        packets
            .iter()
            .all(|p| packet_payload(p, 49).is_none() && packet_payload(p, 50).is_none())
    );
    let updates: Vec<_> = packets
        .iter()
        .filter_map(|p| packet_payload(p, 29))
        .collect();
    assert_eq!(
        updates.len(),
        1,
        "changed attributes still need publication"
    );
    let mut bytes = updates[0];
    assert_eq!(
        VarULong::read(&mut bytes).unwrap().0,
        fixture.player.entity_id() as u64
    );
    assert_eq!(VarUInt::read(&mut bytes).unwrap().0, 1);
    f32::read(&mut bytes).unwrap();
    f32::read(&mut bytes).unwrap();
    assert_eq!(f32::read(&mut bytes).unwrap(), base_damage as f32);
    f32::read(&mut bytes).unwrap();
    f32::read(&mut bytes).unwrap();
    f32::read(&mut bytes).unwrap();
    assert_eq!(String::read(&mut bytes).unwrap(), "minecraft:attack_damage");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn noop_after_clear_keeps_attribute_refresh_without_inventory_packets() {
    use pumpkin_data::attributes::Attributes;
    let server = DeathTestWorld::new().await;
    let world = server.world();
    let mut fixture = TestBedrockPlayer::new(&world).await;
    let mut observer = observer(&world, &fixture).await;
    let player = fixture.player.clone();
    player.permission_lvl.store(PermissionLvl::Four);
    let base_damage = player
        .living_entity
        .get_attribute_value(&Attributes::ATTACK_DAMAGE);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player.inventory().set_held_item(sword.clone());
    player.send_menu_equipment(&[(EquipmentSlot::MAIN_HAND, sword)]);
    assert_eq!(
        player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE),
        base_damage
    );
    player.tick(&server.server);
    assert!(
        player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE)
            > base_damage
    );
    fixture.take_packets();
    assert_equipment(&mut observer, 0, &Item::DIAMOND_SWORD, 1);
    let source = player.get_command_source(&server.server);
    assert_eq!(
        server
            .server
            .command_dispatcher
            .load()
            .execute_input("clear @s", &source)
            .unwrap(),
        1
    );
    assert!(player.inventory().held_item().is_empty());
    fixture.take_packets();
    observer.take_packets();
    send_use(
        &mut fixture,
        vec![],
        TransactionData::Normal(NormalTransactionData),
    );
    assert!(
        player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE)
            > base_damage,
        "the transaction must leave modifier refresh to the tick"
    );
    assert!(fixture.take_packets().iter().all(|p| {
        packet_payload(p, 49).is_none()
            && packet_payload(p, 50).is_none()
            && packet_payload(p, 29).is_none()
    }));
    assert_no_equipment(&mut observer);
    player.tick(&server.server);
    assert_eq!(
        player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE),
        base_damage
    );
    assert_attack_damage_update(&mut fixture, base_damage);
    // /clear queued the removal; the unchanged Normal transaction queues neither hand.
    assert_equipment(&mut observer, 0, &Item::AIR, 0);
    send_use(
        &mut fixture,
        vec![],
        TransactionData::Normal(NormalTransactionData),
    );
    player.tick(&server.server);
    assert!(fixture.take_packets().iter().all(|p| {
        packet_payload(p, 49).is_none()
            && packet_payload(p, 50).is_none()
            && packet_payload(p, 29).is_none()
    }));
    assert_no_equipment(&mut observer);
    observer.close().await;
    fixture.close().await;
    server.server.shutdown().await;
}
