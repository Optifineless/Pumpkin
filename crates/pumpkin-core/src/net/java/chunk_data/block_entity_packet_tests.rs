use super::*;
use crate::block::entities::{
    BlockEntity, capture::capture_legacy, chest::ChestBlockEntity,
    chiseled_bookshelf::ChiseledBookshelfBlockEntity, vault::VaultBlockEntity,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::position::BlockPos;

struct PacketBlockEntity {
    packed_xz: u8,
    y: i16,
    type_id: i32,
    tag: Option<NbtCompound>,
}

fn encoded_block_entities(chunk: &ChunkData) -> Vec<PacketBlockEntity> {
    let version = JavaMinecraftVersion::V_26_3;
    let mut bytes = Vec::new();
    CChunkData(chunk)
        .write_packet_data(&mut bytes, &version)
        .unwrap();
    let mut input = bytes.as_slice();
    assert_eq!(input.get_i32_be().unwrap(), chunk.x);
    assert_eq!(input.get_i32_be().unwrap(), chunk.z);
    let heightmaps = input.get_var_int().unwrap().0;
    for _ in 0..heightmaps {
        input.get_var_int().unwrap();
        let entries = input.get_var_int().unwrap().0;
        for _ in 0..entries {
            input.get_i64_be().unwrap();
        }
    }
    let section_bytes = usize::try_from(input.get_var_int().unwrap().0).unwrap();
    input = &input[section_bytes..];
    let count = input.get_var_int().unwrap().0;
    (0..count)
        .map(|_| PacketBlockEntity {
            packed_xz: input.get_u8().unwrap(),
            y: input.get_i16_be().unwrap(),
            type_id: input.get_var_int().unwrap().0,
            tag: input.get_compound_nbt_with_version(&version).unwrap(),
        })
        .collect()
}

fn marker(value: &str) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("marker", value.to_owned());
    nbt
}

#[test]
fn block_entity_chunk_packet_projects_full_persistent_images_without_changing_them() {
    let chunk = ChunkData::empty(0, 0);
    let chest_pos = BlockPos::new(1, 64, 2);
    let mut chest_input = NbtCompound::new();
    chest_input.put_string("LootTable", "minecraft:chests/simple_dungeon".into());
    chest_input.put_long("LootTableSeed", 172);
    chest_input.put_compound("lock", marker("private lock"));
    let chest = ChestBlockEntity::from_nbt(&chest_input, chest_pos);
    let mut chest_saved = capture_legacy(&chest).into_nbt();
    assert_eq!(
        chest_saved.get_compound("lock"),
        chest_input.get_compound("lock")
    );
    chest_saved.put_compound("PumpkinCustomData", marker("private plugin data"));

    let vault_pos = BlockPos::new(3, 65, 4);
    let vault = VaultBlockEntity::new(vault_pos);
    *vault.config.lock().unwrap() = Some(marker("private vault config"));
    *vault.server_data.lock().unwrap() = Some(marker("private vault state"));
    let shared = marker("visible vault state");
    *vault.shared_data.lock().unwrap() = Some(shared.clone());
    let vault_saved = capture_legacy(&vault).into_nbt();
    let expected = [(chest_pos, chest_saved), (vault_pos, vault_saved)]
        .into_iter()
        .collect();
    *chunk.pending_block_entities.lock().unwrap() = expected;
    let before = chunk.pending_block_entities.lock().unwrap().clone();

    let encoded = encoded_block_entities(&chunk);
    assert_eq!(encoded.len(), 2);
    let chest_packet = encoded
        .iter()
        .find(|entry| entry.packed_xz == 0x12)
        .unwrap();
    assert_eq!(chest_packet.y, 64);
    assert_eq!(chest_packet.type_id, chest.get_id() as i32);
    assert!(
        chest_packet.tag.is_none(),
        "chunk packet exposed deferred loot, the lock or plugin data"
    );
    let vault_packet = encoded
        .iter()
        .find(|entry| entry.packed_xz == 0x34)
        .unwrap();
    assert_eq!(vault_packet.y, 65);
    assert_eq!(vault_packet.type_id, vault.get_id() as i32);
    let mut expected_shared = NbtCompound::new();
    expected_shared.put_compound("shared_data", shared);
    assert_eq!(
        vault_packet.tag,
        Some(expected_shared),
        "chunk packet exposed vault config or server data"
    );
    assert_eq!(*chunk.pending_block_entities.lock().unwrap(), before);
}

#[test]
fn block_entity_chunk_packet_keeps_positions_without_private_nbt_for_missing_projections() {
    let chunk = ChunkData::empty(0, 0);
    let bookshelf_pos = BlockPos::new(1, 64, 2);
    let bookshelf = ChiseledBookshelfBlockEntity::new(bookshelf_pos);
    bookshelf.set_book(0, ItemStack::new(1, &Item::BOOK));
    let bookshelf_saved = capture_legacy(&bookshelf).into_nbt();
    let mut unknown = NbtCompound::new();
    unknown.put_string("id", "test:unknown_native_entity".into());
    unknown.put_compound("private_data", marker("private native state"));
    let mut pending = chunk.pending_block_entities.lock().unwrap();
    pending.insert(bookshelf_pos, bookshelf_saved);
    pending.insert(BlockPos::new(3, 65, 4), unknown);
    drop(pending);
    let encoded = encoded_block_entities(&chunk);
    assert_eq!(encoded.len(), 2);
    assert!(encoded.iter().any(|entry| entry.packed_xz == 0x12));
    assert!(encoded.iter().any(|entry| entry.packed_xz == 0x34));
    assert!(encoded.iter().all(|entry| entry.tag.is_none()));
}
