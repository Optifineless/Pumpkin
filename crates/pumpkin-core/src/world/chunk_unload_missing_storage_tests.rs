//! Missing storage must never turn cancelled live snapshots into dormant entities.
use super::*;

pub(super) fn fixture() -> (Fixture, SyncChunk) {
    let fixture = Fixture::new();
    let portal: Arc<dyn WorldPortalExt> = Arc::new(WorldPortal(fixture.world.clone()));
    fixture
        .world
        .level
        .world_portal
        .store(Arc::new(Some(portal)));
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    fixture.world.level.update_chunk_watchers(&[POS], &[]);
    (fixture, chunk)
}

async fn begin_publication(fixture: &Fixture, chunk: &SyncChunk) -> PublicationBarrier {
    assert!(fixture.world.level.get_entity_chunk_sync(&POS).is_none());
    fixture.world.level.update_chunk_watchers(&[], &[POS]);
    assert!(!fixture.world.level.poll_chunk_unload(chunk));
    // Reading a new region may publish its initial file too; pause only the unload snapshot.
    let storage = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(storage) = fixture.world.level.get_entity_chunk_sync(&POS) {
                break storage;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!storage.live.load(Ordering::Acquire));
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !fixture.world.unloading_entities.contains_key(&POS) {
            assert!(!fixture.world.level.poll_chunk_unload(chunk));
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    wait_publication(&mut pause).await;
    pause
}

#[tokio::test]
async fn followup_review_missing_storage_killed_entity_is_not_resaved() {
    let (fixture, chunk) = fixture();
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let pause = begin_publication(&fixture, &chunk).await;
    let old = generation(&fixture, POS);
    fixture.world.remove_entity(pig.as_ref());
    assert_ne!(generation(&fixture, POS), old);
    pause.resume();
    unload(&fixture, &chunk).await;
    assert!(disk_entities(&fixture, POS).await.is_empty());
    finish(fixture).await;
}

#[tokio::test]
async fn followup_review_missing_storage_moved_entity_is_saved_only_at_destination() {
    let (fixture, source) = fixture();
    let destination_pos = Vector2::new(1, 0);
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.x = destination_pos.x;
    let destination = publish(&fixture.world, terrain);
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let pause = begin_publication(&fixture, &source).await;
    let old = generation(&fixture, POS);
    pig.get_entity().set_pos(Vector3::new(17.5, 64.0, 1.5));
    assert_ne!(generation(&fixture, POS), old);
    pause.resume();
    unload(&fixture, &source).await;
    assert!(!pig.get_entity().is_removed());
    unload(&fixture, &destination).await;
    assert!(disk_entities(&fixture, POS).await.is_empty());
    let records = disk_entities(&fixture, destination_pos).await;
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].get_uuid("UUID"),
        Some(pig.get_entity().entity_uuid)
    );
    finish(fixture).await;
}

#[tokio::test]
async fn followup_review_dormant_records_survive_cancelled_live_snapshot() {
    let (fixture, chunk) = fixture();
    let dormant = pig(&fixture, 2.5);
    let uuid = dormant.get_entity().entity_uuid;
    let record = super::super::entity_persistence::save_riding_tree(&dormant);
    drop(dormant);
    // Write storage through a separate Level so the live world still has no entity storage.
    let disk = fixture.reopen_storage();
    let entities = disk.get_entity_chunk(POS).await.unwrap();
    entities.data.lock().unwrap().push(record);
    entities.mark_dirty(true);
    disk.save_retained_entity_chunk(entities).await.unwrap();
    disk.shutdown().await.unwrap();
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    let pause = begin_publication(&fixture, &chunk).await;
    fixture.world.remove_entity(pig.as_ref());
    pause.resume();
    unload(&fixture, &chunk).await;
    let records = disk_entities(&fixture, POS).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].get_uuid("UUID"), Some(uuid));
    finish(fixture).await;
}

#[tokio::test]
async fn followup_review_save_all_clears_removed_live_root_in_dormant_storage() {
    let (fixture, _) = fixture();
    let pig = pig(&fixture, 1.5);
    assert!(fixture.world.spawn_entity(pig.clone()));
    assert!(fixture.world.level.get_entity_chunk_sync(&POS).is_none());
    fixture.world.save().await;
    fixture.world.remove_entity(pig.as_ref());
    fixture.world.save().await;
    let entities = fixture.world.level.get_entity_chunk_sync(&POS).unwrap();
    fixture
        .world
        .level
        .save_retained_entity_chunk(entities)
        .await
        .unwrap();
    assert!(disk_entities(&fixture, POS).await.is_empty());
    finish(fixture).await;
}
