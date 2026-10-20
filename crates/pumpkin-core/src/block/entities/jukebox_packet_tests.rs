use super::{block_entity_from_nbt, jukebox::JukeboxBlockEntity};
use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::spawn_test_support::{proto, publish},
};
use bytes::Bytes;
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::JukeboxLikeProperties,
    data_component::DataComponent,
    data_component_impl::JukeboxPlayableImpl,
    item::Item,
    meta_data_type::MetaDataType,
    packet::{
        CURRENT_MC_VERSION,
        clientbound::play::{CONTAINER_SET_CONTENT, SET_ENTITY_DATA},
    },
    tracked_data,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn, ser::NetworkReadExt};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::{sync::Arc, time::Duration};

const POSITION: BlockPos = BlockPos::new(0, 64, 1);

fn saved_jukebox(registered: bool) -> NbtCompound {
    let component = if registered {
        NbtTag::String("minecraft:cat".into())
    } else {
        let mut legacy = NbtCompound::new();
        legacy.put_string("song", "minecraft:not_a_song".to_owned());
        NbtTag::Compound(legacy)
    };
    let mut components = NbtCompound::new();
    components.put("minecraft:jukebox_playable", component);
    let mut record = NbtCompound::new();
    record.put_string("id", "minecraft:music_disc_13".to_owned());
    record.put_int("count", 1);
    record.put_compound("components", components);
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:jukebox".to_owned());
    saved.put_int("x", POSITION.0.x);
    saved.put_int("y", POSITION.0.y);
    saved.put_int("z", POSITION.0.z);
    saved.put_compound("RecordItem", record);
    saved.put_long("ticks_since_song_started", 20);
    saved
}

// Decode independently of Pumpkin's component codec so a shared mistake cannot pass.
fn read_disc(input: &mut &[u8], registered: bool) -> bool {
    let count = input.get_var_int().unwrap().0;
    if count == 0 {
        return false;
    }
    assert_eq!(count, 1);
    assert_eq!(
        input.get_var_int().unwrap().0,
        i32::from(Item::MUSIC_DISC_13.id)
    );
    assert_eq!(input.get_var_int().unwrap().0, i32::from(registered));
    assert_eq!(input.get_var_int().unwrap().0, 0);
    if registered {
        assert_eq!(
            input.get_var_int().unwrap().0,
            i32::from(DataComponent::JukeboxPlayable.to_id())
        );
        // Vanilla ByteBufCodecs.holder: cat's jukebox-song registry id 5 + 1.
        assert_eq!(input.get_u8().unwrap(), 6);
    }
    true
}

fn assert_metadata(packets: &[Bytes], entity_id: i32, registered: bool) {
    let metadata: Vec<_> = packets
        .iter()
        .filter_map(|packet| {
            let mut input = packet.as_ref();
            if input.get_var_int().unwrap().0 != SET_ENTITY_DATA.0 {
                return None;
            }
            (input.get_var_int().unwrap().0 == entity_id).then_some(input)
        })
        .collect();
    assert!(
        !metadata.is_empty(),
        "ejected item must send its actual metadata packet"
    );
    // Both the spawn writer and tracker pairing publish the retained stack.
    for mut input in metadata {
        assert_eq!(
            input.get_u8().unwrap(),
            tracked_data::item::ITEM.id.get(&CURRENT_MC_VERSION)
        );
        assert_eq!(input.get_var_int().unwrap().0, MetaDataType::ITEM_STACK.id);
        assert!(read_disc(&mut input, registered));
        assert_eq!(
            input.get_u8().unwrap(),
            0xff,
            "metadata terminator must remain intact"
        );
        assert!(input.is_empty());
    }
}

fn assert_inventory(packets: &[Bytes], registered: bool) {
    let contents: Vec<_> = packets
        .iter()
        .filter_map(|packet| {
            let mut input = packet.as_ref();
            (input.get_var_int().unwrap().0 == CONTAINER_SET_CONTENT.0).then_some(input)
        })
        .collect();
    assert_eq!(
        contents.len(),
        1,
        "real full inventory sync must be captured"
    );
    let mut input = contents[0];
    assert_eq!(input.get_container_id(&CURRENT_MC_VERSION).unwrap().0, 0);
    let _revision = input.get_var_int().unwrap();
    let slots = input.get_var_int().unwrap().0;
    assert_eq!(slots, 46);
    let discs = (0..slots)
        .filter(|_| read_disc(&mut input, registered))
        .count();
    assert_eq!(discs, 1);
    assert!(
        !read_disc(&mut input, registered),
        "carried stack stays empty"
    );
    assert!(input.is_empty(), "all content bytes must be consumed");
}

fn right_click(player: &TestPlayer, server: &Arc<Server>) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: POSITION,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            server,
        )
        .unwrap();
}

async fn eject_restored_record(registered: bool) {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut player = TestPlayer::new(&world);
    player
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 0.5));
    player
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(POSITION.chunk_position());
    let mut properties = JukeboxLikeProperties::from_state_id(Block::JUKEBOX.default_state.id);
    properties.has_record = true;
    world.set_block_state(
        &POSITION,
        properties.to_state_id(&Block::JUKEBOX),
        BlockFlags::FORCE_STATE,
    );
    let installed = block_entity_from_nbt(&saved_jukebox(registered)).unwrap();
    let jukebox = installed
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .unwrap();
    world.add_block_entity(installed.clone());
    assert_eq!(jukebox.is_playing(), registered);
    let restored = jukebox.get_record();
    assert_eq!(
        restored
            .get_data_component::<JukeboxPlayableImpl>()
            .unwrap()
            .song,
        if registered { "cat" } else { "" }
    );
    let mut before = NbtCompound::new();
    restored.write_item_stack(&mut before);
    player.take_packets();
    right_click(&player, &server);
    assert!(
        jukebox.get_record().is_empty(),
        "real right-click ejects the record"
    );
    let dropped: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            let item = entity.get_item_entity()?;
            let stack = item.get_item_stack().lock().unwrap().clone();
            Some((item.get_entity().entity_id, stack))
        })
        .collect();
    assert_eq!(dropped.len(), 1);
    let (entity_id, disc) = &dropped[0];
    assert_metadata(&player.take_packets(), *entity_id, registered);
    player.player.inventory().set_stack(0, disc.clone());
    player
        .player
        .screen_handler_sync_handler
        .store_player(player.player.clone());
    player
        .player
        .on_screen_handler_opened(&player.player.player_screen_handler);
    assert_inventory(&player.take_packets(), registered);
    let mut after = NbtCompound::new();
    disc.write_item_stack(&mut after);
    assert_eq!(
        after, before,
        "client projection must retain saved component meaning"
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_restored_canonical_ejection_and_inventory_packets() {
    tokio::time::timeout(Duration::from_secs(30), eject_restored_record(true))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_restored_stopped_legacy_ejection_and_inventory_packets() {
    tokio::time::timeout(Duration::from_secs(30), eject_restored_record(false))
        .await
        .unwrap();
}
