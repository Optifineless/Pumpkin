#![expect(
    clippy::unwrap_used,
    reason = "Packet fixtures must construct successfully"
)]

use super::super::inventory_exploit_tests::{assert_conserved, packet_payload, stock, transaction};
use super::*;
use crate::{
    entity::{Entity, EntityBase, death_test_world::DeathTestWorld, passive::sheep::SheepEntity},
    net::bedrock::combat_test_support::TestBedrockPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType, item::Item};
use pumpkin_protocol::{
    bedrock::server::inventory_transaction::{
        InventoryAction, UseItemOnEntityTransactionData, UseItemTransactionData,
    },
    codec::var_ulong::VarULong,
    serial::PacketRead,
};
use pumpkin_util::{math::vector3::Vector3, permission::PermissionLvl};

fn prediction(before: &ItemStack, after: &ItemStack) -> InventoryAction {
    InventoryAction {
        source_type: 0,
        window_id: Some(0),
        source_flags: None,
        inventory_slot: 0,
        old_item: NetworkItemDescriptor::from(before),
        new_item: NetworkItemDescriptor::from(after),
    }
}

fn use_item(stack: &ItemStack, action: i32, position: BlockPos) -> TransactionData {
    TransactionData::UseItem(UseItemTransactionData {
        action_type: VarInt(action),
        trigger_type: 0,
        block_position: position,
        block_face: 1,
        hot_bar_slot: VarInt(0),
        hand: HandSlot::Mainhand,
        item_in_hand: NetworkItemDescriptor::from(stack),
        player_position: Vector3::new(8.5, 64.0, 8.5),
        click_position: Vector3::new(0.5, 1.0, 0.5),
        block_runtime_id: VarUInt(0),
        client_prediction: 0,
        client_cooldown_state: 0,
    })
}

fn send_use(fixture: &mut TestBedrockPlayer, actions: Vec<InventoryAction>, data: TransactionData) {
    fixture.take_packets();
    let mut packet = transaction(actions, data);
    packet.legacy_request_id = VarInt(1);
    fixture
        .client()
        .handle_inventory_action(&fixture.player, packet);
}

fn assert_hand_correction(
    fixture: &mut TestBedrockPlayer,
    item: &'static Item,
    amount: u8,
    result: u8,
) {
    let packets = fixture.take_packets();
    // CInventoryContent must describe the hand AFTER the authoritative use.
    let corrections: Vec<_> = packets
        .iter()
        .filter_map(|packet| {
            let mut bytes = packet_payload(packet, 49)?;
            (VarUInt::read(&mut bytes).unwrap().0 == 0).then_some(bytes)
        })
        .collect();
    assert_eq!(
        corrections.len(),
        1,
        "expected one final main-inventory correction"
    );
    let mut bytes = corrections[0];
    assert!(VarUInt::read(&mut bytes).unwrap().0 > 0);
    let corrected = NetworkItemStackDescriptor::read(&mut bytes).unwrap();
    let expected = NetworkItemStackDescriptor::from(&ItemStack::new(amount, item));
    assert_eq!(corrected.id, expected.id);
    assert_eq!(corrected.stack_size, u16::from(amount));
    let mut response = packets
        .iter()
        .find_map(|packet| packet_payload(packet, 148))
        .unwrap();
    assert_eq!(VarUInt::read(&mut response).unwrap().0, 1);
    assert_eq!(u8::read(&mut response).unwrap(), result);
    assert_eq!(VarInt::read(&mut response).unwrap(), VarInt(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_decrement_still_places_one_block() {
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
    let held = ItemStack::new(64, &Item::STONE);
    fixture.player.inventory().set_held_item(held.clone());
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![prediction(&held, &held.copy_with_count(63))],
        use_item(&held, 0, position),
    );
    assert_eq!(world.get_block(&position.up()), &Block::STONE);
    assert_eq!(fixture.player.inventory().held_item().item_count, 63);
    assert_conserved(before, &fixture.player, 1);
    assert_hand_correction(&mut fixture, &Item::STONE, 63, 0);
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_decrement_still_throws_one_egg() {
    let server = DeathTestWorld::new().await;
    let world = server.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut fixture = TestBedrockPlayer::new(&world).await;
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let held = ItemStack::new(2, &Item::EGG);
    fixture.player.inventory().set_held_item(held.clone());
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![prediction(&held, &held.copy_with_count(1))],
        use_item(&held, 1, BlockPos::new(8, 64, 8)),
    );
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_type == &EntityType::EGG)
            .count(),
        1
    );
    assert_eq!(fixture.player.inventory().held_item().item_count, 1);
    assert_conserved(before, &fixture.player, 1);
    assert_hand_correction(&mut fixture, &Item::EGG, 1, 0);
    fixture
        .player
        .inventory()
        .set_held_item(ItemStack::EMPTY.clone());
    let before_forgery = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![prediction(
            ItemStack::EMPTY,
            &ItemStack::new(64, &Item::DIAMOND),
        )],
        use_item(ItemStack::EMPTY, 1, BlockPos::new(8, 64, 8)),
    );
    assert_conserved(before_forgery, &fixture.player, 0);
    assert!(fixture.player.inventory().held_item().is_empty());
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_type == &EntityType::EGG)
            .count(),
        1
    );
    assert_hand_correction(&mut fixture, &Item::AIR, 0, 0);
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prediction_still_allows_authoritative_shearing() {
    let server = DeathTestWorld::new().await;
    let world = server.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut fixture = TestBedrockPlayer::new(&world).await;
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let sheep = SheepEntity::new(Entity::new(
        world.clone(),
        Vector3::new(9.5, 64.0, 8.5),
        &EntityType::SHEEP,
    ));
    world.add_entity_silent(sheep.clone());
    let held = ItemStack::new(1, &Item::SHEARS);
    fixture.player.inventory().set_held_item(held.clone());
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![prediction(&held, &held)],
        TransactionData::UseItemOnEntity(UseItemOnEntityTransactionData {
            target_entity_runtime_id: VarULong(sheep.mob_entity.living_entity.entity_id() as u64),
            action_type: VarInt(0),
            hot_bar_slot: VarInt(0),
            item_in_hand: NetworkItemDescriptor::from(&held),
            player_position: Vector3::new(8.5, 64.0, 8.5),
            click_position: Vector3::new(0.0, 0.0, 0.0),
        }),
    );
    assert!(sheep.is_sheared());
    let after = fixture.player.inventory().held_item();
    assert_eq!(after.item, &Item::SHEARS);
    assert_eq!(after.item_count, 1);
    assert_eq!(after.get_damage(), held.get_damage() + 1);
    // Generated wool is mob loot; no player-owned stock is transferred or consumed.
    assert_conserved(before, &fixture.player, 0);
    assert_hand_correction(&mut fixture, &Item::SHEARS, 1, 0);
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_decrement_still_charges_anchor_once() {
    use pumpkin_data::block_properties::RespawnAnchorLikeProperties;
    use pumpkin_world::world::BlockFlags;
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
    let held = ItemStack::new(2, &Item::GLOWSTONE);
    fixture.player.inventory().set_held_item(held.clone());
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![prediction(&held, &held.copy_with_count(1))],
        use_item(&held, 0, position),
    );
    assert_eq!(
        RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&position)).charges,
        1
    );
    assert_eq!(fixture.player.inventory().held_item().item_count, 1);
    assert_conserved(before, &fixture.player, 1);
    assert_hand_correction(&mut fixture, &Item::GLOWSTONE, 1, 0);
    fixture.close().await;
    server.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prediction_still_allows_release_item() {
    use pumpkin_protocol::bedrock::server::inventory_transaction::ReleaseItemTransactionData;
    let server = DeathTestWorld::new().await;
    let mut fixture = TestBedrockPlayer::new(&server.world()).await;
    let held = ItemStack::new(1, &Item::BOW);
    fixture.player.inventory().set_held_item(held.clone());
    fixture.player.living_entity.set_active_hand(
        Hand::Right,
        held.clone(),
        held.get_max_use_time(),
    );
    let before = stock(&fixture.player, &[]);
    send_use(
        &mut fixture,
        vec![prediction(&held, &held)],
        TransactionData::ReleaseItem(ReleaseItemTransactionData {
            action_type: VarInt(0),
            hot_bar_slot: VarInt(0),
            item_in_hand: NetworkItemDescriptor::from(&held),
            head_position: Vector3::new(8.5, 65.0, 8.5),
        }),
    );
    assert!(
        fixture
            .player
            .living_entity
            .item_in_use
            .lock()
            .unwrap()
            .is_none()
    );
    assert_conserved(before, &fixture.player, 0);
    assert_hand_correction(&mut fixture, &Item::BOW, 1, 0);
    fixture.close().await;
    server.server.shutdown().await;
}

#[path = "inventory_followup_tests.rs"]
mod followup;
