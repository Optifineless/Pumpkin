use super::*;
use crate::entity::{Entity, EntityBase, living::LivingEntity, r#type::from_type};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::{Arc, atomic::Ordering};

#[tokio::test]
async fn registry_cas_loss_never_marks_or_serializes_members() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = Vector2::new(0, 0);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    let root = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    let rider = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 65.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(root.clone());
    world.add_entity_silent(rider.clone());
    root.get_entity().add_passenger(root.clone(), rider.clone());
    world.level.request_entity_unloads([pos]);
    let request = world
        .level
        .pending_entity_unloads()
        .into_iter()
        .find(|r| r.position == pos)
        .unwrap();
    let current = world.entities.load_full();
    // Real registry admission wins against the captured retirement incarnation.
    let newcomer = from_type(
        &EntityType::COW,
        Vector3::new(40.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(newcomer);
    assert!(
        world
            .commit_entity_retirement(
                &request,
                &current,
                Some(Arc::new(Vec::new())),
                &[root.get_entity(), rider.get_entity()]
            )
            .is_none()
    );
    for member in [&root, &rider] {
        assert!(!member.get_entity().is_removed());
        assert!(member.get_entity().removal_reason.load().is_none());
        assert!(!member.get_entity().removed.load(Ordering::Acquire));
        assert!(
            world
                .get_entity_by_uuid(member.get_entity().entity_uuid)
                .is_some()
        );
    }
    assert!(storage.live.load(Ordering::Acquire));
    assert!(storage.data.lock().unwrap().is_empty());
    assert!(
        world
            .level
            .pending_entity_unloads()
            .iter()
            .any(|r| r.position == pos)
    );
    world.process_entity_chunk_lifecycle();
    assert!(root.get_entity().is_removed());
    assert!(rider.get_entity().is_removed());
    assert_eq!(
        storage.data.lock().unwrap()[0]
            .get_list("Passengers")
            .unwrap()
            .len(),
        1
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_retries_after_entity_chunk_activation_restores_its_records() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = Vector2::new(0, 0);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    let original = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );

    let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
    let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
    let paused = Arc::new(PausedEntitySave {
        inner: original.clone(),
        captured,
        resume: resume_writer,
        pause: std::sync::atomic::AtomicBool::new(true),
    });
    world.add_entity_silent(paused);

    // The wrapper blocks after World::save serializes its old entity registry.
    let save_world = world.clone();
    let save = tokio::spawn(async move { save_world.save().await });
    tokio::task::spawn_blocking(move || {
        wait_for_capture
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("entity save snapshot did not reach the barrier");
    })
    .await
    .unwrap();

    world.remove_entities_in_chunks([pos]).await;
    world.process_entity_chunk_lifecycle();
    assert!(original.get_entity().is_removed());
    assert!(!storage.live.load(Ordering::Acquire));
    assert_eq!(storage.data.lock().unwrap().len(), 1);

    // Reload storage after retirement, as the next watched chunk load does.
    let restored_storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&restored_storage, None);
    let restored = world
        .get_entity_by_uuid(original.get_entity().entity_uuid)
        .expect("entity activation did not restore the saved root");
    assert!(!restored.get_entity().is_removed());

    resume_capture.send(()).unwrap();
    save.await.unwrap();
    assert_eq!(restored_storage.data.lock().unwrap().len(), 1);

    // Preserve the exact World::save image so shutdown cannot repair a stale result.
    world.entities.store(Arc::new(Vec::new()));
    restored_storage.live.store(false, Ordering::Release);
    let fixture = fixture.restart().await;
    let world = &fixture.world;
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    assert!(
        world
            .get_entity_by_uuid(original.get_entity().entity_uuid)
            .is_some(),
        "saved entity activation was lost after disk reload"
    );
    fixture.finish().await;
}

struct PausedEntitySave {
    inner: Arc<dyn EntityBase>,
    captured: crossbeam::channel::Sender<()>,
    resume: crossbeam::channel::Receiver<()>,
    pause: std::sync::atomic::AtomicBool,
}

impl EntityBase for PausedEntitySave {
    fn get_entity(&self) -> &Entity {
        self.inner.get_entity()
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        self.inner.get_living_entity()
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self.inner.cast_any()
    }

    fn write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.inner.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            self.captured.send(()).unwrap();
            self.resume
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
        }
    }
}
