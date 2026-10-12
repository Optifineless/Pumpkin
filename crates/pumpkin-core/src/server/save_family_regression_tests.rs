use super::world_info_save_tests::with_worlds;
use super::*;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_data::{Block, item::Item, item_stack::ItemStack};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{
    chunk::{
        ChunkData,
        format::anvil::AnvilChunkFile,
        io::{FileIO, LoadedData, file_manager::ChunkFileManager},
    },
    data::player_data::PlayerDataStorage,
    world::BlockFlags,
};

fn initialize_full_lighting(chunk: &ChunkData) {
    use pumpkin_world::chunk::format::LightContainer;
    let mut light = chunk.light_engine.lock().unwrap();
    light.sky_light = (0..chunk.section.count)
        .map(|_| LightContainer::Full(vec![0xff; LightContainer::ARRAY_SIZE].into_boxed_slice()))
        .collect();
    light.block_light = (0..chunk.section.count)
        .map(|_| LightContainer::Full(vec![0; LightContainer::ARRAY_SIZE].into_boxed_slice()))
        .collect();
}

fn retain_full_ticket(world: &World, pos: Vector2<i32>) {
    use pumpkin_world::chunk_system::ChunkLoading;
    let mut loading = world.level.chunk_loading.lock().unwrap();
    loading.add_ticket(pos, ChunkLoading::FULL_CHUNK_LEVEL);
    loading.send_change();
}

// Seed disk once, then load through the real scheduler so save-all owns a full holder.
async fn prepare_persisted_terrain(world: &Arc<World>) {
    use pumpkin_world::chunk::io::Dirtiable;
    let pos = Vector2::new(0, 0);
    let initial = Arc::new(ChunkData::empty(pos.x, pos.y));
    assert_eq!(initial.section.min_y, world.dimension.min_y);
    assert_eq!(
        initial.section.count,
        usize::try_from(world.dimension.height / 16).unwrap()
    );
    // A full passable fixture has complete uniform lighting; do not generate relight neighbors.
    initial.light_populated.store(true, Ordering::Relaxed);
    initial.lighting_invalid.store(false, Ordering::Relaxed);
    initialize_full_lighting(&initial);
    initial.mark_dirty(true);
    world.level.write_chunks(vec![(pos, initial.clone())]).await;
    world
        .level
        .chunk_saver
        .block_and_await_ongoing_tasks()
        .await
        .unwrap();
    // Cheap actual Anvil round trip precedes the scheduler: assert its admission properties.
    let decoded = disk_chunk(world).await.unwrap();
    assert!(!Arc::ptr_eq(&initial, &decoded));
    assert_eq!(
        decoded.status,
        pumpkin_data::chunk::ChunkStatus::Full,
        "Fixture saved status was not Full"
    );
    assert!(
        decoded.light_populated.load(Ordering::Relaxed),
        "Fixture saved light flag was false"
    );
    assert!(
        !decoded.lighting_invalid.load(Ordering::Relaxed),
        "Fixture saved lighting was invalid"
    );
    let (sky_light_sections, block_light_sections) = {
        let lights = decoded.light_engine.lock().unwrap();
        (lights.sky_light.len(), lights.block_light.len())
    };
    assert_eq!(sky_light_sections, decoded.section.count);
    assert_eq!(block_light_sections, decoded.section.count);
    retain_full_ticket(world, pos);
    tokio::time::timeout(
        Duration::from_secs(30),
        world.level.get_or_fetch_chunk(pos, |_| ()),
    )
    .await
    .expect("Validated Full fixture failed scheduler admission")
    .unwrap();
}

async fn disk_chunk(world: &World) -> Option<Arc<ChunkData>> {
    // A fresh manager cannot serve this world's retained or pending in-memory chunks.
    let disk = ChunkFileManager::<AnvilChunkFile<ChunkData>>::in_dimension(
        AnvilChunkConfig::default(),
        world.dimension.clone(),
    );
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    disk.fetch_chunks(&world.level.level_folder, &[Vector2::new(0, 0)], send)
        .await;
    match receive.recv().await {
        Some(LoadedData::Loaded(chunk)) => Some(chunk),
        Some(LoadedData::Error((_, error))) => panic!("Failed to reopen saved terrain: {error}"),
        Some(LoadedData::Missing(_)) | None => None,
    }
}

async fn disk_block(world: &World) -> Option<pumpkin_data::BlockStateId> {
    disk_chunk(world)
        .await
        .and_then(|chunk| chunk.section.get_block_absolute_y(1, 64, 1))
}

async fn await_disk_block(
    world: &World,
    expected: pumpkin_data::BlockStateId,
) -> Option<pumpkin_data::BlockStateId> {
    let mut latest = None;
    for _ in 0..50 {
        latest = disk_block(world).await;
        if latest == Some(expected) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    latest
}

#[derive(Clone, Copy)]
enum Family {
    Player,
    Terrain,
}

async fn independent_family_survives_metadata_failure(family: Family) {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        let world = &worlds[0];
        prepare_persisted_terrain(world).await;
        let player = TestPlayer::new(world);
        let uuid = player.player.gameprofile.id;
        let (session, _) = server.player_data_storage.load_data(&uuid).await.unwrap();
        *player.player.storage_session.lock().unwrap() = Some(session);
        player.player.living_entity.set_health(18.0);
        let position = BlockPos::new(1, 64, 1);
        world.set_block_state(
            &position,
            Block::STONE.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        server.save_all().await.unwrap();
        assert_eq!(
            await_disk_block(world, Block::STONE.default_state.id).await,
            Some(Block::STONE.default_state.id),
            "The fixture did not publish its baseline terrain before fault injection"
        );
        let path = server.basic_config.get_world_path();
        let storage = PlayerDataStorage::new(path.join("players"), true);
        assert_eq!(
            storage
                .load_player_data(&uuid)
                .unwrap()
                .1
                .get_float("Health"),
            Some(18.0),
            "The fixture did not publish its baseline player before fault injection"
        );
        player.player.living_entity.set_health(7.0);
        player
            .player
            .inventory
            .set_slot(0, ItemStack::new(17, &Item::DIAMOND));
        world.set_block_state(
            &position,
            Block::DIAMOND_BLOCK.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let destination = path.join("data/minecraft/game_rules.dat");
        let backup = path.join("game-rules-before-family-fault.dat");
        let original = fs::read(&destination).unwrap();
        fs::rename(&destination, &backup).unwrap();
        fs::create_dir(&destination).unwrap();
        let result = server.save_all().await;
        // Capture durable evidence before repairing the fault or fixture shutdown saves.
        let saved_player = storage.load_player_data(&uuid).unwrap().1;
        let saved_block = await_disk_block(world, Block::DIAMOND_BLOCK.default_state.id).await;
        fs::remove_dir(&destination).unwrap();
        fs::rename(&backup, &destination).unwrap();
        assert!(result.is_err(), "Save-all hid the metadata failure");
        assert_eq!(fs::read(&destination).unwrap(), original);
        match family {
            Family::Player => assert_eq!(
                saved_player.get_float("Health"),
                Some(7.0),
                "A metadata failure skipped independently writable player data"
            ),
            Family::Terrain => assert_eq!(
                saved_block,
                Some(Block::DIAMOND_BLOCK.default_state.id),
                "A metadata failure skipped independently writable terrain"
            ),
        }
        assert_eq!(saved_player.get_float("Health"), Some(7.0));
        assert_eq!(saved_block, Some(Block::DIAMOND_BLOCK.default_state.id));
        server.save_all().await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_save_all_metadata_failure_still_saves_real_player_data() {
    independent_family_survives_metadata_failure(Family::Player).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_save_all_metadata_failure_still_saves_real_terrain() {
    independent_family_survives_metadata_failure(Family::Terrain).await;
}
