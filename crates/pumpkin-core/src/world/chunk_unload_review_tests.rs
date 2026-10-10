//! Deterministic production unload/storage interleavings; disk reads precede shutdown draining.
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::{level::SyncChunk, world::WorldPortalExt};

use pumpkin_world::chunk::io::Dirtiable;
use pumpkin_world::chunk::io::file_manager::PublicationBarrier;

use super::{
    WorldPortal,
    spawn_test_support::{Fixture, proto, publish},
};
use crate::entity::EntityBase;

const POS: Vector2<i32> = Vector2::new(0, 0);

async fn fixture() -> (Fixture, SyncChunk) {
    let fixture = Fixture::new();
    let portal: Arc<dyn WorldPortalExt> = Arc::new(WorldPortal(fixture.world.clone()));
    fixture
        .world
        .level
        .world_portal
        .store(Arc::new(Some(portal)));
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    let chunk = publish(&fixture.world, terrain);
    chunk.mark_dirty(true);
    let entity_chunk = fixture.world.level.get_entity_chunk(POS).await.unwrap();
    fixture.world.make_chunk_entities_live(&entity_chunk, None);
    (fixture, chunk)
}

fn pig(fixture: &Fixture, x: f64) -> Arc<dyn EntityBase> {
    crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(x, 64.0, 1.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    )
}

fn generation(fixture: &Fixture, pos: Vector2<i32>) -> u64 {
    fixture
        .world
        .level
        .chunk_lifecycles
        .at(pos)
        .lock()
        .unwrap()
        .generation
}

async fn wait_publication(pause: &mut PublicationBarrier) {
    tokio::time::timeout(Duration::from_secs(15), pause.wait())
        .await
        .unwrap()
        .unwrap();
}

async fn unload(fixture: &Fixture, chunk: &SyncChunk) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !fixture.world.level.poll_chunk_unload(chunk) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn disk_entities(fixture: &Fixture, pos: Vector2<i32>) -> Vec<NbtCompound> {
    let disk = fixture.reopen_storage();
    let chunk = disk.get_entity_chunk(pos).await.unwrap();
    let records = chunk.data.lock().unwrap().clone();
    disk.shutdown().await.unwrap();
    records
}

async fn finish(fixture: Fixture) {
    fixture.world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
async fn spawn_during_publication_retries_and_persists_pig() {
    let (fixture, chunk) = fixture().await;
    let pig = pig(&fixture, 1.5);
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    wait_publication(&mut pause).await;
    let old = generation(&fixture, POS);
    let uuid = pig.get_entity().entity_uuid;
    assert!(fixture.world.spawn_entity(pig.clone()));
    assert_ne!(generation(&fixture, POS), old);
    pause.resume();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    assert!(Arc::ptr_eq(
        &pig,
        &fixture.world.get_entity_by_uuid(uuid).unwrap()
    ));
    unload(&fixture, &chunk).await;
    assert!(pig.get_entity().is_removed());
    assert_eq!(
        disk_entities(&fixture, POS).await[0].get_uuid("UUID"),
        Some(uuid)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn cross_chunk_move_invalidates_source_and_destination_snapshots() {
    let (fixture, source) = fixture().await;
    let destination_pos = Vector2::new(1, 0);
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    terrain.x = 1;
    let destination = publish(&fixture.world, terrain);
    let entities = fixture
        .world
        .level
        .get_entity_chunk(destination_pos)
        .await
        .unwrap();
    fixture.world.make_chunk_entities_live(&entities, None);
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let mut pause = fixture.world.level.chunk_saver.pause_next_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&source));
    wait_publication(&mut pause).await;
    assert!(!fixture.world.level.poll_chunk_unload(&destination));
    let old_source = generation(&fixture, POS);
    let old_destination = generation(&fixture, destination_pos);
    pig.get_entity().set_pos(Vector3::new(17.5, 64.0, 1.5));
    assert_ne!(generation(&fixture, POS), old_source);
    assert_ne!(generation(&fixture, destination_pos), old_destination);
    pause.resume();
    unload(&fixture, &source).await;
    assert!(!pig.get_entity().is_removed());
    unload(&fixture, &destination).await;
    assert!(disk_entities(&fixture, POS).await.is_empty());
    assert_eq!(
        disk_entities(&fixture, destination_pos).await[0].get_uuid("UUID"),
        Some(pig.get_entity().entity_uuid)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn dismount_invalidates_old_root_and_new_root_chunks() {
    let (fixture, source) = fixture().await;
    let destination_pos = Vector2::new(1, 0);
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    terrain.x = 1;
    let destination = publish(&fixture.world, terrain);
    let entities = fixture
        .world
        .level
        .get_entity_chunk(destination_pos)
        .await
        .unwrap();
    fixture.world.make_chunk_entities_live(&entities, None);
    let vehicle = pig(&fixture, 1.5);
    let passenger = pig(&fixture, 17.5);
    assert!(fixture.world.spawn_entity(vehicle.clone()));
    assert!(fixture.world.spawn_entity(passenger.clone()));
    vehicle
        .get_entity()
        .add_passenger(vehicle.clone(), passenger.clone());
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&source));
    wait_publication(&mut pause).await;
    assert!(!fixture.world.level.poll_chunk_unload(&destination));
    let old_source = generation(&fixture, POS);
    let old_destination = generation(&fixture, destination_pos);
    vehicle
        .get_entity()
        .remove_passenger_before_teleport(passenger.get_entity().entity_id);
    assert_ne!(generation(&fixture, POS), old_source);
    assert_ne!(generation(&fixture, destination_pos), old_destination);
    pause.resume();
    unload(&fixture, &source).await;
    assert!(!passenger.get_entity().is_removed());
    unload(&fixture, &destination).await;
    let source_records = disk_entities(&fixture, POS).await;
    assert_eq!(source_records.len(), 1);
    assert!(source_records[0].get_list("Passengers").is_none());
    assert_eq!(
        disk_entities(&fixture, destination_pos).await[0].get_uuid("UUID"),
        Some(passenger.get_entity().entity_uuid)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn held_entity_writer_follows_nbt_chunk_move() {
    let (fixture, source) = fixture().await;
    let destination_pos = Vector2::new(1, 0);
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    terrain.x = 1;
    let destination = publish(&fixture.world, terrain);
    let entities = fixture
        .world
        .level
        .get_entity_chunk(destination_pos)
        .await
        .unwrap();
    fixture.world.make_chunk_entities_live(&entities, None);
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let mut nbt = NbtCompound::new();
    pig.write_nbt(&mut nbt);
    nbt.put(
        "Pos",
        NbtTag::List(vec![
            NbtTag::Double(17.5),
            NbtTag::Double(64.0),
            NbtTag::Double(1.5),
        ]),
    );
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&source));
    wait_publication(&mut pause).await;
    let writer = pig.get_entity().try_begin_mutation().unwrap();
    pig.read_nbt_non_mut(&nbt);
    assert!(!fixture.world.level.poll_chunk_unload(&destination));
    assert!(
        !fixture
            .world
            .unloading_entities
            .contains_key(&destination_pos)
    );
    // A native async writer can finish raw state updates after its nested move permit drops.
    let mut values = NbtCompound::new();
    values.put_int("value", 31);
    pig.get_entity()
        .custom_data
        .lock()
        .unwrap()
        .put_compound("test", values);
    pause.resume();
    assert!(!fixture.world.level.poll_chunk_unload(&source));
    assert!(Arc::ptr_eq(&pig, &fixture.world.entities.load()[0]));
    drop(writer);
    unload(&fixture, &source).await;
    assert!(!pig.get_entity().is_removed());
    unload(&fixture, &destination).await;
    assert!(disk_entities(&fixture, POS).await.is_empty());
    let records = disk_entities(&fixture, destination_pos).await;
    assert_eq!(
        records[0].get_uuid("UUID"),
        Some(pig.get_entity().entity_uuid)
    );
    assert_eq!(
        records[0]
            .get_compound("PumpkinCustomData")
            .unwrap()
            .get_compound("test")
            .unwrap()
            .get_int("value"),
        Some(31)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn entity_state_write_during_publication_is_resaved() {
    let (fixture, chunk) = fixture().await;
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    wait_publication(&mut pause).await;
    let old = generation(&fixture, POS);
    pig.get_entity()
        .set_custom_data("test", "value", NbtTag::Int(17));
    assert_ne!(generation(&fixture, POS), old);
    pause.resume();
    unload(&fixture, &chunk).await;
    let records = disk_entities(&fixture, POS).await;
    assert!(pig.get_entity().try_begin_mutation().is_none());
    assert_eq!(
        records[0]
            .get_compound("PumpkinCustomData")
            .unwrap()
            .get_compound("test")
            .unwrap()
            .get_int("value"),
        Some(17)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn entity_removal_during_publication_is_resaved() {
    let (fixture, chunk) = fixture().await;
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    wait_publication(&mut pause).await;
    let old = generation(&fixture, POS);
    fixture.world.remove_entity(pig.as_ref());
    assert_ne!(generation(&fixture, POS), old);
    pause.resume();
    unload(&fixture, &chunk).await;
    assert!(disk_entities(&fixture, POS).await.is_empty());
    finish(fixture).await;
}

#[tokio::test]
async fn captured_entity_identity_is_validated_before_detachment() {
    let (fixture, chunk) = fixture().await;
    let original = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(original.clone()));
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    wait_publication(&mut pause).await;
    // Exercise the final identity fence independently of generation invalidation: an unadmitted
    // replacement with identical saved bytes must still never be detached as the captured Arc.
    let replacement = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(1.5, 64.0, 1.5),
        &fixture.world,
        original.get_entity().entity_uuid,
    );
    let mut nbt = NbtCompound::new();
    original.write_nbt(&mut nbt);
    replacement.read_nbt_non_mut(&nbt);
    // Construction above admits mutations; capture a fresh snapshot of the original first.
    pause.resume();
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !fixture
            .world
            .level
            .chunk_lifecycles
            .at(POS)
            .lock()
            .unwrap()
            .quiescing
        {
            assert!(!fixture.world.level.poll_chunk_unload(&chunk));
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    wait_publication(&mut pause).await;
    fixture.world.entities.rcu(|_| vec![replacement.clone()]);
    pause.resume();
    // Hold rewatch admission after validation has rejected the old identity so it cannot retry.
    tokio::time::timeout(Duration::from_secs(15), async {
        while fixture
            .world
            .level
            .chunk_lifecycles
            .at(POS)
            .lock()
            .unwrap()
            .quiescing
        {
            assert!(!fixture.world.level.poll_chunk_unload(&chunk));
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!replacement.get_entity().is_removed());
    assert!(Arc::ptr_eq(&replacement, &fixture.world.entities.load()[0]));
    unload(&fixture, &chunk).await;
    assert!(fixture.world.entities.load().is_empty());
    assert_eq!(
        disk_entities(&fixture, POS).await[0].get_uuid("UUID"),
        Some(original.get_entity().entity_uuid)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn retained_native_writer_cancels_quiescence_and_rejects_detached_handles() {
    let (fixture, chunk) = fixture().await;
    fixture
        .world
        .level
        .set_retained_chunk_custom_data(&chunk, "test", "value", NbtTag::Int(7))
        .unwrap();
    let mut pause = fixture.world.level.chunk_saver.pause_next_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    wait_publication(&mut pause).await;
    let old_generation = generation(&fixture, POS);
    fixture
        .world
        .level
        .set_retained_chunk_custom_data(&chunk, "test", "value", NbtTag::Int(8))
        .unwrap();
    assert_ne!(generation(&fixture, POS), old_generation);
    assert!(
        !fixture
            .world
            .level
            .chunk_lifecycles
            .at(POS)
            .lock()
            .unwrap()
            .quiescing
    );
    pause.resume();
    unload(&fixture, &chunk).await;
    assert!(
        fixture
            .world
            .level
            .set_retained_chunk_custom_data(&chunk, "test", "value", NbtTag::Int(9))
            .is_err()
    );
    assert!(
        fixture
            .world
            .level
            .remove_retained_chunk_custom_data(&chunk, "test", "value")
            .is_err()
    );
    assert_eq!(chunk.get_custom_data("test", "value"), Some(NbtTag::Int(8)));
    let disk = fixture.reopen_storage();
    let permit = disk.begin_chunk_mutation(POS);
    let value = tokio::time::timeout(
        Duration::from_secs(15),
        disk.get_or_fetch_chunk(POS, |chunk| chunk.get_custom_data("test", "value")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(value, Some(NbtTag::Int(8)));
    drop(permit);
    disk.shutdown().await.unwrap();
    finish(fixture).await;
}

#[tokio::test]
async fn cancelled_entity_only_cleanup_reopens_admission_and_retries() {
    let (fixture, terrain) = fixture().await;
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    fixture.world.level.loaded_chunks.remove(&POS);
    drop(terrain);
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    let mut cleanup = Box::pin(fixture.world.remove_entities_in_chunks([POS]));
    tokio::select! {
        () = &mut cleanup => panic!("cleanup completed before publication"),
        () = wait_publication(&mut pause) => {},
    }
    assert!(
        fixture
            .world
            .level
            .chunk_lifecycles
            .at(POS)
            .lock()
            .unwrap()
            .quiescing
    );
    drop(cleanup);
    assert!(
        !fixture
            .world
            .level
            .chunk_lifecycles
            .at(POS)
            .lock()
            .unwrap()
            .quiescing
    );
    assert!(fixture.world.level.clean_memory().contains(&POS));
    assert!(Arc::ptr_eq(&pig, &fixture.world.entities.load()[0]));
    pause.resume();
    fixture.world.remove_entities_in_chunks([POS]).await;
    assert!(pig.get_entity().is_removed());
    assert_eq!(
        disk_entities(&fixture, POS).await[0].get_uuid("UUID"),
        Some(pig.get_entity().entity_uuid)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn rewatch_during_publication_keeps_live_members_and_resaves_state() {
    let (fixture, chunk) = fixture().await;
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    wait_publication(&mut pause).await;
    let old = generation(&fixture, POS);
    fixture.world.level.update_chunk_watchers(&[POS], &[]);
    assert_ne!(generation(&fixture, POS), old);
    assert!(!fixture.world.unloading_entities.contains_key(&POS));
    assert!(fixture.world.level.try_chunk_mutation(POS).is_some());
    pause.resume();
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    assert!(Arc::ptr_eq(&pig, &fixture.world.entities.load()[0]));
    assert!(fixture.world.level.is_chunk_loaded(&POS));
    pig.get_entity()
        .set_custom_data("test", "value", NbtTag::Int(23));
    fixture.world.level.update_chunk_watchers(&[], &[POS]);
    unload(&fixture, &chunk).await;
    let records = disk_entities(&fixture, POS).await;
    assert_eq!(
        records[0]
            .get_compound("PumpkinCustomData")
            .unwrap()
            .get_compound("test")
            .unwrap()
            .get_int("value"),
        Some(23)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn stub_piston_lease_prevents_snapshot_admission() {
    let (fixture, chunk) = fixture().await;
    let leased = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let check = leased.clone();
    fixture
        .world
        .level
        .chunk_lifecycles
        .set_unload_gate(Arc::new(move |_| !check.load(Ordering::Acquire)));
    assert!(!fixture.world.level.poll_chunk_unload(&chunk));
    assert!(!fixture.world.unloading_entities.contains_key(&POS));
    assert!(
        !fixture
            .world
            .level
            .chunk_lifecycles
            .at(POS)
            .lock()
            .unwrap()
            .quiescing
    );
    leased.store(false, Ordering::Release);
    unload(&fixture, &chunk).await;
    assert!(disk_entities(&fixture, POS).await.is_empty());
    finish(fixture).await;
}
