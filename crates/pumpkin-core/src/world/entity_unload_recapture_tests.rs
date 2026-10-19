//! Reopened terrain must receive another final image without an explicit save-all.
use super::*;
use crate::{
    entity::r#type::from_type,
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_data::{Block, biome::Biome, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{
    chunk::{
        ChunkData,
        format::anvil::AnvilChunkFile,
        io::{FileIO, LoadedData, file_manager::ChunkFileManager},
    },
    world::BlockFlags,
};
use std::time::Duration;

const CHEST: BlockPos = BlockPos::new(8, 64, 8);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entity_unload_recaptures_reopened_chest_before_terrain_save_without_save_all() {
    let fixture = Fixture::new();
    let world = &fixture.world;
    let terrain = publish(world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = CHEST.chunk_position();
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    assert!(world.make_chunk_entities_live(&storage, None));
    world.level.request_entity_unloads([pos]);
    world.process_entity_chunk_lifecycle();
    assert!(!storage.live.load(Ordering::Acquire));
    assert!(world.level.get_entity_chunk_sync(&pos).is_none());
    assert!(world.level.is_chunk_loaded(&pos));

    world.set_block_state(
        &CHEST,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let inventory = world
        .get_block_entity(&CHEST)
        .expect("retired loaded terrain refused chest placement")
        .get_inventory()
        .unwrap();
    // No update_block_entity or save-all masks the required second final capture.
    inventory.set_stack(0, ItemStack::new(7, &Item::EMERALD));
    let cow = from_type(
        &EntityType::COW,
        Vector3::new(9.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    assert!(world.spawn_entity(cow.clone()));
    world.process_entity_chunk_lifecycle();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if world
                .level
                .pending_entity_unloads()
                .iter()
                .any(|request| request.position == pos && request.storage.is_some())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reopened terrain never reloaded entity storage before its next capture");
    world.process_entity_chunk_lifecycle();
    assert!(!world.block_entities.contains_key(&pos));
    assert!(cow.get_entity().is_removed());
    assert!(world.level.pending_entity_unloads().is_empty());

    world.level.write_chunks(vec![(pos, terrain)]).await;
    world
        .level
        .chunk_saver
        .block_and_await_ongoing_tasks()
        .await
        .unwrap();
    world.level.loaded_chunks.remove(&pos);
    reload_terrain(world, pos).await;
    let saved = world
        .get_block_entity(&CHEST)
        .expect("reopened chest disappeared on terrain reload")
        .get_inventory()
        .unwrap();
    assert_eq!(saved.get_stack(0).item, &Item::EMERALD);
    assert_eq!(saved.get_stack(0).item_count, 7);
    fixture.finish().await;
}

async fn reload_terrain(world: &World, pos: Vector2<i32>) {
    let disk = ChunkFileManager::<AnvilChunkFile<ChunkData>>::in_dimension(
        AnvilChunkConfig::default(),
        world.dimension.clone(),
    );
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    disk.fetch_chunks(&world.level.level_folder, &[pos], send)
        .await;
    let chunk = match receive.recv().await {
        Some(LoadedData::Loaded(chunk)) => chunk,
        Some(LoadedData::Error((_, error))) => {
            panic!("reopened terrain failed disk reload: {error}")
        }
        Some(LoadedData::Missing(_)) | None => panic!("reopened terrain missing on disk reload"),
    };
    world.level.loaded_chunks.insert(pos, chunk);
}
