use super::*;
use crate::{
    block::entities::{BlockEntity, decorated_pot::DecoratedPotBlockEntity},
    net::java::chunk_data::CChunkData,
};
use pumpkin_data::data_component::DataComponent;
use pumpkin_inventory::{
    crafting::crafting_screen_handler::CraftingTableScreenHandler, screen_handler::ScreenHandler,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{ClientPacket, java::server::play::SlotActionType, ser::NetworkReadExt};
use pumpkin_util::version::JavaMinecraftVersion;
use pumpkin_world::chunk::{
    ChunkData,
    io::{FileIO, LoadedData},
};
use std::time::Duration;

const POSITION: BlockPos = BlockPos::new(8, 65, 8);
const FACE_NAMES: [&str; 4] = ["back", "left", "right", "front"];
const SHERDS: [&Item; 4] = [
    &Item::ANGLER_POTTERY_SHERD,
    &Item::ARCHER_POTTERY_SHERD,
    &Item::ARMS_UP_POTTERY_SHERD,
    &Item::BLADE_POTTERY_SHERD,
];

fn craft_mixed_pot(fixture: &Fixture) -> ItemStack {
    let mut menu = CraftingTableScreenHandler::new(1, fixture.player.player.inventory(), None);
    for (index, slot) in [1, 3, 5, 7].into_iter().enumerate() {
        let mut ingredient = ItemStack::new(2, SHERDS[index]);
        ingredient.set_custom_name(FACE_NAMES[index].to_owned());
        ingredient.remove_data_component(DataComponent::Enchantments);
        menu.get_behaviour().slots[slot + 1].set_stack(ingredient);
    }
    menu.send_content_updates();
    menu.on_slot_click(0, 0, SlotActionType::Pickup, fixture.player.player.as_ref());
    for (index, slot) in [2, 4, 6, 8].into_iter().enumerate() {
        let remaining = menu.get_behaviour().slots[slot].get_stack();
        assert_eq!(
            (remaining.item.id, remaining.item_count),
            (SHERDS[index].id, 1)
        );
    }
    let mut cursor = menu.get_behaviour().cursor_stack.lock().unwrap();
    let crafted = std::mem::replace(&mut *cursor, ItemStack::EMPTY.clone());
    assert_eq!(
        (crafted.item.id, crafted.item_count),
        (Item::DECORATED_POT.id, 1)
    );
    crafted
}

fn literal_faces(counts: [i32; 4]) -> NbtCompound {
    // Vanilla 26.3 PotDecorations.CODEC and ItemStackTemplate.CODEC literal fixture.
    let registry_names = [
        "minecraft:angler_pottery_sherd",
        "minecraft:archer_pottery_sherd",
        "minecraft:arms_up_pottery_sherd",
        "minecraft:blade_pottery_sherd",
    ];
    let mut faces = NbtCompound::new();
    for index in 0..4 {
        let mut template = NbtCompound::new();
        template.put_string("id", registry_names[index].to_owned());
        if counts[index] != 1 {
            template.put_int("count", counts[index]);
        }
        let mut components = NbtCompound::new();
        components.put_string("minecraft:custom_name", FACE_NAMES[index].to_owned());
        components.put_compound("!minecraft:enchantments", NbtCompound::new());
        template.put_compound("components", components);
        faces.put_compound(FACE_NAMES[index], template);
    }
    faces
}

fn load_pot(fixture: &Fixture, mut nbt: NbtCompound) -> Arc<dyn BlockEntity> {
    fixture.set_block(POSITION.down(), &Block::STONE);
    fixture.world.remove_block_entity(&POSITION);
    fixture
        .world
        .level
        .read_chunk_sync(&POSITION.chunk_position(), |chunk| {
            chunk.set_block_absolute_y(8, POSITION.0.y, 8, Block::DECORATED_POT.default_state.id);
        })
        .unwrap();
    nbt.put_string("id", DecoratedPotBlockEntity::ID.to_owned());
    nbt.put_int("x", POSITION.0.x);
    nbt.put_int("y", POSITION.0.y);
    nbt.put_int("z", POSITION.0.z);
    fixture.world.add_block_entity_nbt(POSITION, &nbt);
    fixture.world.get_block_entity(&POSITION).unwrap()
}

fn chunk(fixture: &Fixture) -> Arc<ChunkData> {
    fixture
        .world
        .level
        .loaded_chunks
        .get(&POSITION.chunk_position())
        .unwrap()
        .clone()
}

fn chunk_packet_tag(chunk: &ChunkData) -> NbtCompound {
    let version = JavaMinecraftVersion::V_26_3;
    let mut bytes = Vec::new();
    CChunkData(chunk)
        .write_packet_data(&mut bytes, &version)
        .unwrap();
    let mut reader = bytes.as_slice();
    reader.get_i32_be().unwrap();
    reader.get_i32_be().unwrap();
    for _ in 0..reader.get_var_int().unwrap().0 {
        reader.get_var_int().unwrap();
        for _ in 0..reader.get_var_int().unwrap().0 {
            reader.get_i64_be().unwrap();
        }
    }
    let section_bytes = reader.get_var_int().unwrap().0 as usize;
    reader = &reader[section_bytes..];
    assert_eq!(reader.get_var_int().unwrap().0, 1);
    assert_eq!(reader.get_u8().unwrap(), 0x88);
    assert_eq!(reader.get_i16_be().unwrap(), POSITION.0.y as i16);
    reader.get_var_int().unwrap();
    reader
        .get_compound_nbt_with_version(&version)
        .unwrap()
        .unwrap()
}

fn block_update_tags(fixture: &mut Fixture) -> Vec<NbtCompound> {
    let version = JavaMinecraftVersion::V_26_3;
    fixture
        .player
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            if reader.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::BLOCK_ENTITY_DATA.to_id(version)
                || reader.get_block_pos(&version).unwrap() != POSITION
            {
                return None;
            }
            reader.get_var_int().unwrap();
            reader.get_compound_nbt_with_version(&version).unwrap()
        })
        .collect()
}

async fn save_unload_reload(fixture: &Fixture) -> Arc<dyn BlockEntity> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let terrain = chunk(fixture);
        fixture
            .world
            .remove_entities_in_chunks([POSITION.chunk_position()])
            .await;
        fixture.world.tick(&fixture.server);
        assert!(
            !fixture
                .world
                .block_entities
                .contains_key(&POSITION.chunk_position())
        );
        fixture
            .world
            .level
            .chunk_saver
            .save_chunks(
                &fixture.world.level.level_folder,
                vec![(POSITION.chunk_position(), terrain)],
            )
            .await
            .unwrap();
        fixture
            .world
            .level
            .loaded_chunks
            .remove(&POSITION.chunk_position());
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        fixture
            .world
            .level
            .chunk_saver
            .fetch_chunks(
                &fixture.world.level.level_folder,
                &[POSITION.chunk_position()],
                send,
            )
            .await;
        let LoadedData::Loaded(reloaded) = receive.recv().await.unwrap() else {
            panic!("decorated pot terrain was not saved")
        };
        fixture
            .world
            .level
            .loaded_chunks
            .insert(POSITION.chunk_position(), reloaded);
        fixture.world.get_block_entity(&POSITION).unwrap()
    })
    .await
    .expect("decorated pot save/unload/reload did not finish")
}

fn faces_on_item(entity: &dyn BlockEntity) -> PotDecorationsImpl {
    let mut item = ItemStack::new(1, &Item::DECORATED_POT);
    entity.collect_item_components(&mut item);
    item.get_data_component::<PotDecorationsImpl>()
        .unwrap()
        .clone()
}

fn assert_private_absent(nbt: &NbtCompound) {
    for field in [
        "PotDecorations",
        "PotDecorationsRetained",
        "PotDecorationsEmpty",
    ] {
        assert!(
            nbt.get(field).is_none(),
            "client received private field {field}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_placed_faces_are_canonical_on_wire_and_disk() {
    let mut fixture = Fixture::new();
    let crafted = craft_mixed_pot(&fixture);
    fixture.set_block(POSITION.down(), &Block::STONE);
    fixture.give(crafted);
    fixture.player.take_packets();
    fixture.use_on(POSITION.down());
    assert!(fixture.player.player.inventory().held_item().is_empty());
    fixture.insert(POSITION, &Item::DIAMOND);
    fixture.world.tick(&fixture.server);

    let expected = literal_faces([1; 4]);
    let entity = fixture.world.get_block_entity(&POSITION).unwrap();
    let update = entity.chunk_data_nbt().unwrap();
    assert_eq!(
        update.get_compound("sherds"),
        Some(&expected),
        "placed faces require canonical named sherds"
    );
    assert_private_absent(&update);
    assert_eq!(
        update.get_compound("item").unwrap().get_string("id"),
        Some("minecraft:diamond")
    );
    let tags = block_update_tags(&mut fixture);
    assert!(
        !tags.is_empty(),
        "placement did not publish a block-entity update"
    );
    assert!(
        tags.iter()
            .any(|tag| tag.get_compound("sherds") == Some(&expected))
    );
    for tag in &tags {
        assert_private_absent(tag);
    }

    let wire_chunk = chunk_packet_tag(&chunk(&fixture));
    assert_eq!(wire_chunk.get_compound("sherds"), Some(&expected));
    assert_private_absent(&wire_chunk);
    let reloaded = save_unload_reload(&fixture).await;
    let mut saved = NbtCompound::new();
    reloaded.write_nbt(&mut saved);
    assert_eq!(saved.get_compound("sherds"), Some(&expected));
    assert_eq!(
        chunk_packet_tag(&chunk(&fixture)).get_compound("sherds"),
        Some(&expected)
    );
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(POSITION);
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 1);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    for sherd in SHERDS {
        assert_eq!(item_count(&fixture.world, sherd), 1);
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_legacy_face_stock_and_private_empty_keep_their_meaning() {
    let list = [
        "minecraft:brick",
        "minecraft:angler_pottery_sherd",
        "minecraft:archer_pottery_sherd",
        "minecraft:brick",
    ]
    .map(|name| NbtTag::String(name.into()))
    .to_vec();
    for kind in 0..3 {
        let fixture = Fixture::new();
        let mut old = NbtCompound::new();
        if kind != 0 {
            old.put_list("sherds", list.clone());
        }
        if kind == 2 {
            old.put_compound("PotDecorations", NbtCompound::new());
        }
        let loaded = load_pot(&fixture, old);
        let before = faces_on_item(loaded.as_ref());
        let reloaded = save_unload_reload(&fixture).await;
        assert_eq!(faces_on_item(reloaded.as_ref()), before);
        fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
        fixture.dig(POSITION);
        assert_eq!(item_count(&fixture.world, &Item::BRICK), [4, 2, 0][kind]);
        assert_eq!(
            item_count(&fixture.world, &Item::DECORATED_POT),
            u32::from(kind == 2)
        );
        for sherd in [&Item::ANGLER_POTTERY_SHERD, &Item::ARCHER_POTTERY_SHERD] {
            assert_eq!(item_count(&fixture.world, sherd), u32::from(kind == 1));
        }
        fixture.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_imported_templates_keep_counts_patches_and_unknown_sources() {
    let fixture = Fixture::new();
    let expected = literal_faces([2, 3, 4, 5]);
    let mut imported = expected.clone();
    imported.put_long("future_face_data", 42);
    let mut old = NbtCompound::new();
    old.put_compound("sherds", imported.clone());
    let _ = load_pot(&fixture, old);
    let reloaded = save_unload_reload(&fixture).await;
    let mut saved = NbtCompound::new();
    reloaded.write_nbt(&mut saved);
    assert_eq!(saved.get_compound("sherds"), Some(&expected));
    assert_eq!(
        saved
            .get_compound("PotDecorationsRetained")
            .unwrap()
            .get_compound("sherds"),
        Some(&imported)
    );
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(POSITION);
    for (index, sherd) in SHERDS.into_iter().enumerate() {
        assert_eq!(item_count(&fixture.world, sherd), index as u32 + 2);
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_empty_reuse_and_unsupported_sources_survive_reload() {
    let fixture = Fixture::new();
    let mut unknown = NbtCompound::new();
    unknown.put_list("future", vec![NbtTag::Long(17)]);
    let mut face = NbtCompound::new();
    face.put_string("id", "minecraft:angler_pottery_sherd".to_owned());
    face.put_compound("components", unknown);
    let mut private = NbtCompound::new();
    private.put_compound("back", face);
    let mut input = NbtCompound::new();
    input.put_compound("PotDecorations", private.clone());
    let loaded = load_pot(&fixture, input);
    assert_eq!(faces_on_item(loaded.as_ref()), PotDecorationsImpl::EMPTY);
    let reloaded = save_unload_reload(&fixture).await;
    assert_eq!(faces_on_item(reloaded.as_ref()), PotDecorationsImpl::EMPTY);
    let mut saved = NbtCompound::new();
    reloaded.write_nbt(&mut saved);
    assert_eq!(
        saved
            .get_compound("PotDecorationsRetained")
            .unwrap()
            .get_compound("PotDecorations"),
        Some(&private)
    );
    assert!(saved.get("sherds").is_none());
    assert_eq!(saved.get_bool("PotDecorationsEmpty"), Some(true));

    reloaded.apply_item_components(&craft_mixed_pot(&fixture));
    reloaded.apply_item_components(&ItemStack::new(1, &Item::DECORATED_POT));
    let reloaded = save_unload_reload(&fixture).await;
    assert_eq!(faces_on_item(reloaded.as_ref()), PotDecorationsImpl::EMPTY);
    let mut edited = NbtCompound::new();
    reloaded.write_nbt(&mut edited);
    edited.put_compound("sherds", literal_faces([1; 4]));
    // BlockDataAccessor.setData reconstructs the entity from the edited durable tag.
    let edited = load_pot(&fixture, edited);
    assert_ne!(faces_on_item(edited.as_ref()), PotDecorationsImpl::EMPTY);
    let packet = edited.chunk_data_nbt().unwrap();
    assert_eq!(packet.get_compound("sherds"), Some(&literal_faces([1; 4])));
    assert_private_absent(&packet);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_unreadable_canonical_faces_are_retained_without_legacy_bricks() {
    let fixture = Fixture::new();
    let mut future = NbtCompound::new();
    let mut face = NbtCompound::new();
    face.put_string("id", "test:unknown_future_material".to_owned());
    face.put_int("count", 7);
    future.put_compound("back", face);
    future.put_list(
        "future_faces",
        vec![NbtTag::ByteArray(vec![1, 4, 9].into())],
    );
    let mut input = NbtCompound::new();
    input.put_compound("sherds", future.clone());
    let loaded = load_pot(&fixture, input);
    assert_eq!(faces_on_item(loaded.as_ref()), PotDecorationsImpl::EMPTY);
    for _ in 0..2 {
        let reloaded = save_unload_reload(&fixture).await;
        assert_eq!(faces_on_item(reloaded.as_ref()), PotDecorationsImpl::EMPTY);
        let mut saved = NbtCompound::new();
        reloaded.write_nbt(&mut saved);
        assert_eq!(
            saved
                .get_compound("PotDecorationsRetained")
                .unwrap()
                .get_compound("sherds"),
            Some(&future)
        );
        assert!(saved.get("sherds").is_none());
        assert_private_absent(&reloaded.chunk_data_nbt().unwrap());
    }
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(POSITION);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 1);
    fixture.shutdown().await;
}

#[test]
fn b1020b_g09_canonical_edits_keep_unsupported_data_beside_older_raw_sources() {
    let mut original = NbtCompound::new();
    let list = ["minecraft:brick"; 4]
        .map(|name| NbtTag::String(name.into()))
        .to_vec();
    original.put_list("sherds", list.clone());
    let loaded = DecoratedPotBlockEntity::from_nbt(&original, POSITION);
    let mut edited = NbtCompound::new();
    loaded.write_nbt(&mut edited);
    let mut current = literal_faces([1; 4]);
    current.put_list("future_data", vec![NbtTag::Long(99)]);
    edited.put_compound("sherds", current.clone());
    let loaded = DecoratedPotBlockEntity::from_nbt(&edited, POSITION);
    let mut saved = NbtCompound::new();
    loaded.write_nbt(&mut saved);
    let backup = saved.get_compound("PotDecorationsRetained").unwrap();
    assert_eq!(backup.get_list("sherds"), Some(list.as_slice()));
    assert_eq!(backup.get_compound("CurrentSherds"), Some(&current));
    let reloaded = DecoratedPotBlockEntity::from_nbt(&saved, POSITION);
    let mut saved_again = NbtCompound::new();
    reloaded.write_nbt(&mut saved_again);
    assert_eq!(
        saved_again.get_compound("PotDecorationsRetained"),
        Some(backup)
    );
}

struct GuardCheckedComponent(Arc<std::sync::atomic::AtomicUsize>);

impl DataComponentImpl for GuardCheckedComponent {
    fn write_data(&self) -> NbtTag {
        pumpkin_util::sync::assert_no_guards_held("pot face component write");
        self.0.fetch_add(1, Relaxed);
        NbtTag::String("guard checked".into())
    }

    fn clone_dyn(&self) -> Box<dyn DataComponentImpl> {
        pumpkin_util::sync::assert_no_guards_held("pot face component clone");
        self.0.fetch_add(1, Relaxed);
        Box::new(Self(self.0.clone()))
    }

    fn equal(&self, other: &dyn DataComponentImpl) -> bool {
        other.as_any().is::<Self>()
    }

    fn get_enum() -> DataComponent {
        DataComponent::CustomName
    }
    fn get_self_enum(&self) -> DataComponent {
        Self::get_enum()
    }
    fn to_dyn(self) -> Box<dyn DataComponentImpl> {
        Box::new(self)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn b1020b_g09_face_component_callbacks_run_after_leaf_release() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut face = decoration(&Item::ANGLER_POTTERY_SHERD, 1);
    face.patch.push((
        DataComponent::CustomName,
        Some(Box::new(GuardCheckedComponent(calls.clone()))),
    ));
    let mut item = ItemStack::new(1, &Item::DECORATED_POT);
    item.set_data_component(PotDecorationsImpl {
        decorations: [Some(face), None, None, None],
    });
    let pot = DecoratedPotBlockEntity::new(POSITION);
    pot.apply_item_components(&item);
    let _ = faces_on_item(&pot);
    pot.write_nbt(&mut NbtCompound::new());
    let _ = pot.chunk_data_nbt();
    pot.apply_item_components(&ItemStack::new(1, &Item::DECORATED_POT));
    assert!(
        calls.load(Relaxed) >= 4,
        "component callbacks were not exercised"
    );
}

#[test]
fn b1020b_g09_bedrock_face_projection_uses_supported_items_and_pattern_removals() {
    let mut removed = decoration(&Item::ANGLER_POTTERY_SHERD, 1);
    removed
        .patch
        .push((DataComponent::ProvidesPotteryPattern, None));
    let mut item = ItemStack::new(1, &Item::DECORATED_POT);
    item.set_data_component(PotDecorationsImpl {
        decorations: [
            Some(decoration(&Item::DIAMOND, 1)),
            Some(removed),
            None,
            Some(decoration(&Item::HEART_POTTERY_SHERD, 1)),
        ],
    });
    let pot = DecoratedPotBlockEntity::new(POSITION);
    pot.apply_item_components(&item);
    let data = pot
        .bedrock_block_actor_data(Block::DECORATED_POT.default_state.id)
        .unwrap();
    let expected = [
        "minecraft:brick",
        "minecraft:brick",
        "minecraft:brick",
        "minecraft:heart_pottery_sherd",
    ]
    .map(|name| NbtTag::String(name.into()));
    assert_eq!(data.get_list("sherds"), Some(expected.as_slice()));
    assert!(
        pot.bedrock_block_actor_data(Block::STONE.default_state.id)
            .is_none()
    );
}
