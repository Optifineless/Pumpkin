use super::{BlockEntity, jukebox::JukeboxBlockEntity};
use crate::{
    block::{
        BlockBehaviour, BlockHitResult, UseWithItemArgs, blocks::jukebox::JukeboxBlock,
        registry::BlockActionResult,
    },
    entity::death_test_world::DeathTestWorld,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, data_component_impl::EquipmentSlot,
    data_component_impl::JukeboxPlayableImpl, item::Item, item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{
    chunk::{ChunkData, io::FileIO, io::LoadedData},
    world::BlockFlags,
};
use std::{sync::Arc, time::Duration};

fn literal_disc(component: NbtTag) -> ItemStack {
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", "minecraft:music_disc_13".to_owned());
    nbt.put_int("count", 2);
    let mut components = NbtCompound::new();
    components.put("minecraft:jukebox_playable", component);
    nbt.put_compound("components", components);
    ItemStack::read_item_stack(&nbt).expect("literal song fixture must load through ItemStack")
}

fn save_entity(entity: &dyn BlockEntity) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    entity.write_internal(&mut nbt);
    nbt
}

fn assert_saved_song(saved: &NbtCompound, song: &str) {
    assert_eq!(
        saved
            .get_compound("RecordItem")
            .and_then(|record| record.get_compound("components"))
            .and_then(|components| components.get_string("minecraft:jukebox_playable")),
        Some(format!("minecraft:{song}").as_str()),
        "an explicit playable patch must save its canonical registered song"
    );
}

async fn disk_reload(fixture: &DeathTestWorld, chunk: Arc<ChunkData>) {
    let world = fixture.world();
    let position = Vector2::new(0, 0);
    // The ordinary entity phase captures and retires live block entities before chunk saving.
    world.remove_entities_in_chunks([position]).await;
    world.tick(&fixture.server);
    assert!(world.block_entities.get(&position).is_none());
    world
        .level
        .chunk_saver
        .save_chunks(&world.level.level_folder, vec![(position, chunk)])
        .await
        .expect("ordinary chunk save must finish");
    world.level.loaded_chunks.remove(&position);
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    world
        .level
        .chunk_saver
        .fetch_chunks(&world.level.level_folder, &[position], send)
        .await;
    let LoadedData::Loaded(reloaded) = receive.recv().await.expect("saved chunk must be returned")
    else {
        panic!("saved jukebox chunk must exist on disk");
    };
    world.level.loaded_chunks.insert(position, reloaded);
}

fn assert_song(record: &ItemStack, song: &str, patched: bool) {
    assert_eq!(record.item, &Item::MUSIC_DISC_13);
    assert_eq!(record.item_count, 1);
    assert_eq!(!record.patch.is_empty(), patched);
    assert_eq!(
        record
            .get_data_component::<JukeboxPlayableImpl>()
            .map(|component| component.song.rsplit(':').next().expect("song key")),
        Some(song),
        "the disc must retain its chosen registered song instead of its default"
    );
}

fn insert_record(
    fixture: &DeathTestWorld,
    world: &Arc<World>,
    position: BlockPos,
    mut disc: ItemStack,
) -> Arc<JukeboxBlockEntity> {
    world.set_block_state(
        &position,
        Block::JUKEBOX.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let jukebox = Arc::new(JukeboxBlockEntity::new(position));
    world.add_block_entity(jukebox.clone());
    let player = fixture.player("PatchedRecord");
    player.inventory().set_held_item(disc.clone());
    let cursor = Vector3::new(0.5, 1.0, 0.5);
    let hit = BlockHitResult {
        face: &BlockDirection::Up,
        cursor_pos: &cursor,
    };
    assert!(matches!(
        JukeboxBlock.use_with_item(UseWithItemArgs {
            server: &fixture.server,
            world,
            block: &Block::JUKEBOX,
            position: &position,
            player: &player,
            hit: &hit,
            item_stack: &mut disc,
            equipment_slot: &EquipmentSlot::MAIN_HAND,
        }),
        BlockActionResult::Success
    ));
    assert_eq!(
        disc.item_count, 1,
        "Survival insertion pays exactly one disc"
    );
    assert_eq!(player.inventory().held_item().item_count, 1);
    for _ in 0..20 {
        jukebox.tick(world);
    }
    assert!(jukebox.is_playing());
    assert_eq!(
        save_entity(jukebox.as_ref()).get_long("ticks_since_song_started"),
        Some(20)
    );
    jukebox
}

async fn exercise_reload(records: Vec<(ItemStack, &str)>) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let chunk = publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut inserted = Vec::new();
    for (index, (disc, song)) in records.into_iter().enumerate() {
        let patched = !disc.patch.is_empty();
        let position = BlockPos::new(4 + i32::try_from(index).expect("small fixture") * 2, 64, 8);
        let jukebox = insert_record(&fixture, &world, position, disc);
        assert_song(&jukebox.get_record(), song, patched);
        if patched {
            assert_saved_song(&save_entity(jukebox.as_ref()), song);
        }
        inserted.push((position, jukebox, song, patched));
    }
    // Player ticking is unrelated to chunk retirement and this fixture has no watched chunks.
    world.players.store(Arc::new(Vec::new()));
    disk_reload(&fixture, chunk).await;
    for (position, previous, song, patched) in inserted {
        let installed = world
            .get_block_entity(&position)
            .expect("disk-loaded pending jukebox must use the real block entity loader");
        let restored = installed
            .as_any()
            .downcast_ref::<JukeboxBlockEntity>()
            .expect("loaded block entity must be a jukebox");
        assert!(!std::ptr::eq(previous.as_ref(), restored));
        assert_song(&restored.get_record(), song, patched);
        assert!(
            restored.is_playing(),
            "active patched song must remain active"
        );
        let saved = save_entity(restored);
        assert_eq!(
            saved.get_long("ticks_since_song_started"),
            Some(20),
            "disk reload must retain elapsed ticks without restarting the song"
        );
        if patched {
            assert_saved_song(&saved, song);
        }
        installed.tick(&world);
        assert_eq!(
            save_entity(restored).get_long("ticks_since_song_started"),
            Some(21),
            "restored active state must advance from the retained elapsed time"
        );
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_explicit_jukebox_component_survives_reload() {
    let mut disc = ItemStack::new(2, &Item::MUSIC_DISC_13);
    disc.set_data_component(JukeboxPlayableImpl {
        song: "minecraft:cat",
    });
    tokio::time::timeout(
        Duration::from_secs(120),
        exercise_reload(vec![(disc, "cat")]),
    )
    .await
    .expect("explicit component save/unload/reload must finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_literal_canonical_and_legacy_jukebox_components_survive_reload() {
    let canonical = literal_disc(NbtTag::String("minecraft:cat".into()));
    let mut legacy = NbtCompound::new();
    legacy.put_string("song", "minecraft:cat".to_owned());
    let legacy = literal_disc(NbtTag::Compound(legacy));
    tokio::time::timeout(
        Duration::from_secs(120),
        exercise_reload(vec![(canonical, "cat"), (legacy, "cat")]),
    )
    .await
    .expect("literal canonical and old fork components must survive disk reload");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_default_jukebox_disc_survives_reload() {
    tokio::time::timeout(
        Duration::from_secs(120),
        exercise_reload(vec![(ItemStack::new(2, &Item::MUSIC_DISC_13), "13")]),
    )
    .await
    .expect("ordinary generated disc must survive disk reload");
}
