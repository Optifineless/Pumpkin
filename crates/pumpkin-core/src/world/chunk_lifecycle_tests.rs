//! Headless regressions using real block entities, scheduler and region storage.
use std::sync::Arc;

use pumpkin_data::{Block, biome::Biome, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::WorldPortalExt;

use super::{
    WorldPortal,
    spawn_test_support::{Fixture, proto, publish},
};
use crate::block::entities::{BlockEntity, chest::ChestBlockEntity, hopper::HopperBlockEntity};

fn attach_portal(fixture: &Fixture) {
    let portal: Arc<dyn WorldPortalExt> = Arc::new(WorldPortal(fixture.world.clone()));
    fixture
        .world
        .level
        .world_portal
        .store(Arc::new(Some(portal)));
}

async fn seed_storage(fixture: &Fixture, pos: Vector2<i32>, chest_pos: BlockPos) {
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    terrain.set_block_state(1, 64, 1, Block::CHEST.default_state);
    terrain.set_block_state(1, 65, 1, Block::HOPPER.default_state);
    let chunk = publish(&fixture.world, terrain);
    let chest = Arc::new(ChestBlockEntity::from_nbt(&NbtCompound::new(), chest_pos));
    fixture.world.add_block_entity(chest.clone());
    fixture.world.level.write_chunks(vec![(pos, chunk)]).await;
    fixture.world.level.loaded_chunks.remove(&pos);
    fixture.world.block_entities.remove(&pos);
    drop(chest);
}

#[tokio::test]
async fn hopper_chest_survives_delayed_cleanup_and_restart() {
    let fixture = Fixture::new();
    attach_portal(&fixture);
    let pos = Vector2::new(0, 0);
    let chest_pos = BlockPos::new(1, 64, 1);
    seed_storage(&fixture, pos, chest_pos).await;

    // Load through the actual scheduler, then retain a mutation while its unload runs.
    let mutation = fixture.world.level.begin_chunk_mutation(pos);
    let chunk = fixture
        .world
        .level
        .get_or_fetch_chunk(pos, Clone::clone)
        .await
        .unwrap();
    let chest = fixture.world.get_block_entity(&chest_pos).unwrap();
    let inventory = chest.clone().get_inventory().unwrap();
    let hopper_pos = BlockPos::new(1, 65, 1);
    let hopper = Arc::new(HopperBlockEntity::new(
        hopper_pos,
        pumpkin_data::block_properties::FacingHopper::Down,
    ));
    hopper.set_stack(0, ItemStack::new(16, &Item::COBBLESTONE));
    fixture.world.add_block_entity(hopper.clone());
    for _ in 0..200 {
        hopper.tick(&fixture.world);
        if hopper.get_stack(0).is_empty() {
            break;
        }
    }
    assert!(hopper.get_stack(0).is_empty());
    drop(hopper);
    assert_eq!(inventory.get_stack(0).item_count, 16);
    // Old scheduler order serializes the old empty NBT while this cleanup is delayed.
    fixture
        .world
        .level
        .should_unload
        .store(true, std::sync::atomic::Ordering::Release);
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert_eq!(inventory.get_stack(0).item_count, 16);
    drop(inventory);
    drop(chest);
    drop(mutation);
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while fixture.world.level.is_chunk_loaded(&pos) {
            fixture
                .world
                .level
                .should_unload
                .store(true, std::sync::atomic::Ordering::Release);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        chunk.pending_block_entities.lock().unwrap()[&chest_pos]
            .get_list("Items")
            .is_some()
    );
    drop(chunk);
    fixture.world.level.world_portal.store(Arc::new(None));
    let fixture = fixture.restart().await;
    attach_portal(&fixture);
    let mutation = fixture.world.level.begin_chunk_mutation(pos);
    fixture
        .world
        .level
        .get_or_fetch_chunk(pos, |_| ())
        .await
        .unwrap();
    let chest = fixture
        .world
        .get_block_entity(&chest_pos)
        .unwrap()
        .get_inventory()
        .unwrap();
    assert_eq!(chest.get_stack(0).item_count, 16);
    assert_eq!(chest.get_stack(0).item, &Item::COBBLESTONE);
    assert!(
        fixture
            .world
            .get_block_entity(&hopper_pos)
            .unwrap()
            .get_inventory()
            .unwrap()
            .is_empty()
    );
    drop(chest);
    drop(mutation);
    fixture.world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
async fn concurrent_lazy_chest_lookups_return_the_canonical_inventory() {
    let fixture = Fixture::new();
    let pos = BlockPos::new(1, 64, 1);
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.set_block_state(1, 64, 1, Block::CHEST.default_state);
    let chunk = publish(&fixture.world, terrain);
    let chest = ChestBlockEntity::from_nbt(&NbtCompound::new(), pos);
    let mut nbt = NbtCompound::new();
    chest.write_internal(&mut nbt);
    chunk
        .pending_block_entities
        .lock()
        .unwrap()
        .insert(pos, nbt);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let entities = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..8 {
            let world = &fixture.world;
            let barrier = barrier.clone();
            workers.push(scope.spawn(move || {
                barrier.wait();
                world.get_block_entity(&pos).unwrap()
            }));
        }
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        entities
            .iter()
            .all(|entity| Arc::ptr_eq(entity, &entities[0]))
    );
    drop(entities);
    fixture.world.remove_block_entity(&pos);
    fixture.world.set_block_state(
        &pos,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::empty(),
    );
    assert!(fixture.world.get_block_entity(&pos).is_none());
    fixture.finish().await;
}

#[tokio::test]
async fn rewatch_preserves_live_chest_during_an_obsolete_cleanup() {
    let fixture = Fixture::new();
    let pos = Vector2::new(0, 0);
    let block = BlockPos::new(1, 64, 1);
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.set_block_state(1, 64, 1, Block::CHEST.default_state);
    publish(&fixture.world, terrain);
    fixture
        .world
        .add_block_entity(Arc::new(ChestBlockEntity::from_nbt(
            &NbtCompound::new(),
            block,
        )));
    fixture
        .world
        .level
        .mark_chunks_as_newly_watched(&[pos])
        .await;
    let entity_chunk = fixture.world.level.get_entity_chunk(pos).await.unwrap();
    fixture.world.make_chunk_entities_live(&entity_chunk, None);
    let pig = crate::entity::r#type::from_type(
        &pumpkin_data::entity::EntityType::PIG,
        pumpkin_util::math::vector3::Vector3::new(1.5, 64.0, 1.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    assert!(fixture.world.spawn_entity(pig.clone()));
    let stale = fixture.world.level.mark_chunks_as_not_watched([pos]).await;
    fixture
        .world
        .level
        .mark_chunks_as_newly_watched(&[pos])
        .await;
    let chest = fixture.world.get_block_entity(&block).unwrap();
    chest
        .clone()
        .get_inventory()
        .unwrap()
        .set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    fixture.world.remove_entities_in_chunks(&stale).await;
    fixture.world.level.clean_entity_chunks(&stale);
    let mut loaded = fixture.world.level.receive_entity_chunks(vec![pos]);
    let (cached, first_load) = loaded.recv().await.unwrap();
    assert!(!first_load);
    fixture
        .world
        .activate_chunk_entities(&cached.upgrade().unwrap(), None);
    assert!(Arc::ptr_eq(
        &pig,
        &fixture
            .world
            .get_entity_by_uuid(pig.get_entity().entity_uuid)
            .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &chest,
        &fixture.world.get_block_entity(&block).unwrap()
    ));
    assert_eq!(chest.get_inventory().unwrap().get_stack(0).item_count, 7);
    fixture.finish().await;
}
