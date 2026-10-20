#![expect(clippy::unwrap_used, reason = "Packet regression fixtures must decode")]

use std::sync::Arc;

use pumpkin_data::{Block, item_stack::ItemStack, packet::CURRENT_MC_VERSION};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    ServerPacket, VarInt,
    java::{
        client::play::CBlockEntityData,
        server::play::{SBlockEntityTagQuery, SEntityTagQuery},
    },
    ser::NetworkReadExt,
};
use pumpkin_util::{
    PermissionLvl,
    math::{position::BlockPos, vector2::Vector2},
};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::entities::{BlockEntity, barrel::BarrelBlockEntity},
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};

pub const POSITION: BlockPos = BlockPos::new(8, 64, 8);

fn application_data() -> NbtCompound {
    let mut components = NbtCompound::new();
    components.put_int("test:application_blob", 17);
    let mut data = NbtCompound::new();
    data.put_string("id", "minecraft:stone".into());
    data.put_compound("components", components);
    data
}

pub fn saved_barrel() -> NbtCompound {
    let mut future = NbtCompound::new();
    future.put_int("a", 1);
    let mut components = NbtCompound::new();
    components.put_compound("minecraft:future_thing", future);
    components.put_string("minecraft:food", "undecodable".into());
    components.put_int("minecraft:damage", 12);
    components.put_int("!minecraft:repair_cost", 9);
    components.put_byte("!test:future", 1);
    components.put_compound("minecraft:custom_data", application_data());
    let mut item = NbtCompound::new();
    item.put_string("id", "minecraft:stone".into());
    item.put_int("count", 7);
    item.put_compound("components", components);
    let mut entry = NbtCompound::new();
    entry.put_int("slot", 0);
    entry.put_compound("item", item);
    let mut components = NbtCompound::new();
    components.put_list("minecraft:container", vec![NbtTag::Compound(entry)]);
    components.put_int("minecraft:future_container", 23);
    let mut outer = NbtCompound::new();
    outer.put_string("id", "minecraft:shulker_box".into());
    outer.put_int("count", 1);
    outer.put_compound("components", components);
    outer.put_byte("Slot", 0);
    let mut barrel = NbtCompound::new();
    barrel.put_string("id", "minecraft:barrel".into());
    barrel.put_int("x", 8);
    barrel.put_int("y", 64);
    barrel.put_int("z", 8);
    // Item-shaped roots must still traverse their block-entity inventory fields.
    barrel.put_compound("components", NbtCompound::new());
    barrel.put_list("Items", vec![NbtTag::Compound(outer)]);
    barrel
}

pub fn assert_client_items(nbt: &NbtCompound) {
    let items = nbt.get_list("Items").unwrap();
    assert_eq!(items.len(), 1);
    let outer = items[0].extract_compound().unwrap();
    assert_eq!(outer.get_int("count"), Some(1));
    let components = outer.get_compound("components").unwrap();
    assert!(!components.has("minecraft:future_container"));
    let contents = components.get_list("minecraft:container").unwrap();
    let inner = contents[0]
        .extract_compound()
        .unwrap()
        .get_compound("item")
        .unwrap();
    assert_eq!(inner.get_int("count"), Some(7));
    let components = inner.get_compound("components").unwrap();
    assert!(!components.has("minecraft:future_thing"));
    assert!(!components.has("minecraft:food"));
    assert!(!components.has("!test:future"));
    assert_eq!(components.get_int("minecraft:damage"), Some(12));
    assert!(
        components
            .get_compound("!minecraft:repair_cost")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        components.get_compound("minecraft:custom_data"),
        Some(&application_data())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_components_world_notifications_and_operator_queries_omit_raw() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let mut player = TestPlayer::new(&world);
    player.client().set_player(player.player.clone());
    world.set_block_state(
        &POSITION,
        Block::BARREL.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    player.take_packets();
    let barrel: Arc<dyn BlockEntity> =
        Arc::new(BarrelBlockEntity::from_nbt(&saved_barrel(), POSITION));
    let mut before = NbtCompound::new();
    barrel.write_nbt(&mut before);
    world.add_block_entity(barrel.clone());
    world.update_block_entity(&barrel);
    let mut updates = 0;
    for packet in player.take_packets() {
        let mut input = packet.as_ref();
        if input.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::BLOCK_ENTITY_DATA.to_id(CURRENT_MC_VERSION)
        {
            let packet = CBlockEntityData::read(&mut input, &CURRENT_MC_VERSION).unwrap();
            let mut input = packet.nbt_data.as_ref();
            assert_client_items(
                &input
                    .get_compound_nbt_with_version(&CURRENT_MC_VERSION)
                    .unwrap()
                    .unwrap(),
            );
            updates += 1;
        }
    }
    assert_eq!(updates, 2);
    player.player.permission_lvl.store(PermissionLvl::Two);
    player.client().handle_block_entity_tag_query(
        &player.player,
        &SBlockEntityTagQuery {
            transaction_id: VarInt(399),
            location: POSITION,
        },
    );
    let packets = player.take_packets();
    assert_eq!(packets.len(), 1);
    let mut input = packets[0].as_ref();
    assert_eq!(
        input.get_var_int().unwrap().0,
        pumpkin_data::packet::clientbound::play::TAG_QUERY.to_id(CURRENT_MC_VERSION)
    );
    assert_eq!(input.get_var_int().unwrap().0, 399);
    assert_client_items(
        &input
            .get_compound_nbt_with_version(&CURRENT_MC_VERSION)
            .unwrap()
            .unwrap(),
    );
    let mut after = NbtCompound::new();
    barrel.write_nbt(&mut after);
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_components_operator_entity_query_omits_raw_without_mutating_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut player = TestPlayer::new(&world);
    player.client().set_player(player.player.clone());
    player.player.permission_lvl.store(PermissionLvl::Two);
    let saved = saved_barrel();
    let saved = saved.get_list("Items").unwrap()[0]
        .extract_compound()
        .unwrap();
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::read_item_stack(saved).unwrap());
    let mut before = NbtCompound::new();
    player
        .player
        .inventory()
        .get_stack(0)
        .write_item_stack(&mut before);
    player.take_packets();
    player.client().handle_entity_tag_query(
        &player.player,
        &SEntityTagQuery {
            transaction_id: VarInt(399),
            entity_id: VarInt(player.player.get_entity().entity_id),
        },
    );
    let packets = player.take_packets();
    assert_eq!(packets.len(), 1);
    let mut input = packets[0].as_ref();
    assert_eq!(
        input.get_var_int().unwrap().0,
        pumpkin_data::packet::clientbound::play::TAG_QUERY.to_id(CURRENT_MC_VERSION)
    );
    assert_eq!(input.get_var_int().unwrap().0, 399);
    let nbt = input
        .get_compound_nbt_with_version(&CURRENT_MC_VERSION)
        .unwrap()
        .unwrap();
    let mut inventory = NbtCompound::new();
    inventory.put_list("Items", nbt.get_list("Inventory").unwrap().to_vec());
    assert_client_items(&inventory);
    let mut after = NbtCompound::new();
    player
        .player
        .inventory()
        .get_stack(0)
        .write_item_stack(&mut after);
    assert_eq!(after, before);
}
